//! Resolves a pathspec written relative to a directory into one relative to the repository root.
//!
//! In order, [`pathspec`]:
//!
//! 1. parses the spec, keeping its magic (`:(exclude)`, globs) as written, and rejects attribute magic;
//! 2. picks the prefix to put in front of it: none for `:/` (top) magic or an absolute spec, otherwise
//!    the directory of `cwd` inside the worktree (none in a bare repository);
//! 3. normalises the pattern against the worktree root, lexically, and for an absolute spec retries
//!    against the worktree's real path if that fails;
//! 4. rejects a spec that leaves the worktree, and in a bare repository any absolute spec;
//! 5. spells a spec that names the root itself as the empty pattern, which matches every path.

use crate::GixError;

use gix::{
    path::{from_bstr, normalize, realpath},
    pathspec::{Defaults, MagicSignature, Pattern, parse},
};

use std::{borrow::Cow, path::Path};

/// Separates the components of a repository-relative path.
const PATH_SEPARATOR: &str = "/";

/// Resolves `spec`, written relative to `cwd` or absolute, into a pathspec relative to the root of
/// `worktree`; a bare repository (`worktree` is `None`) has no current directory, so `spec` is taken
/// from its root.
///
/// The magic and the pattern are kept as written, so globs and `:(exclude)` keep their meaning. A
/// pathspec that names the root itself matches every path. The directory prefix is normalised
/// lexically, without touching the filesystem. An absolute spec is matched against `worktree` as
/// given and, if that fails, against the resolved (real) path of `worktree`, so a worktree reached
/// through a symbolic link is tried in resolved form. The spec itself is never resolved through
/// symbolic links: a spec that goes through a link is rejected when `worktree` is given by its real
/// path.
///
/// # Errors
///
/// Returns [`GixError::Pathspec`] when `spec` is not a valid pathspec,
/// [`GixError::PathspecAttributes`] when it uses attribute magic, and
/// [`GixError::PathOutsideRepository`] when it does not lie inside the worktree; for a bare
/// repository, when it is absolute or climbs above the root.
pub fn pathspec(worktree: Option<&Path>, cwd: &Path, spec: &str) -> Result<Pattern, GixError> {
    let parsed =
        parse(spec.as_bytes(), Defaults::default()).map_err(|source| GixError::Pathspec {
            spec: spec.to_owned(),
            source,
        })?;

    if !parsed.attributes.is_empty() {
        return Err(GixError::PathspecAttributes {
            spec: spec.to_owned(),
        });
    }

    let outside = || GixError::PathOutsideRepository {
        path: spec.to_owned(),
    };

    let absolute = from_bstr(parsed.path())
        .map_err(|_error| outside())?
        .is_absolute();

    let prefix = if parsed.signature.contains(MagicSignature::TOP) {
        String::new()
    } else if absolute {
        if worktree.is_none() {
            return Err(outside());
        }

        String::new()
    } else {
        match worktree {
            Some(root) => cwd_prefix(root, cwd).ok_or_else(outside)?,
            None => String::new(),
        }
    };

    let root = worktree.unwrap_or_else(|| Path::new(""));

    let resolved = normalize_pattern(&parsed, &prefix, root).or_else(|| {
        let real = realpath(worktree.filter(|_| absolute)?).ok()?;

        normalize_pattern(&parsed, &prefix, &real)
    });

    let mut pattern = resolved.ok_or_else(outside)?;

    if pattern.is_nil() {
        // `.` names the root, which every path lies under; gix keeps it as a nil pattern whose literal `.` would
        // be taken as a prefix that no path has, so the root is spelled as the empty pattern that matches all.
        pattern = Pattern::default();
    }

    Ok(pattern)
}

/// The repository path of `cwd` to prefix a relative pathspec with, joined with `/`; empty for the
/// root, `None` when `cwd` is outside `root`.
///
/// `cwd` is normalised lexically, so deleted paths and symbolic links inside the worktree are taken as
/// written. When that fails, both `root` and `cwd` are resolved through symbolic links and compared again,
/// so a current directory reached through a link still matches.
fn cwd_prefix(root: &Path, cwd: &Path) -> Option<String> {
    let below = |base: &Path, path: &Path| {
        let target = normalize(Cow::Borrowed(path), path)?;
        let base = normalize(Cow::Borrowed(base), base)?;
        let relative = target.strip_prefix(&base).ok()?;

        Some(
            relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join(PATH_SEPARATOR),
        )
    };

    below(root, cwd).or_else(|| below(&realpath(root).ok()?, &realpath(cwd).ok()?))
}

/// A copy of `parsed` normalised against `prefix` and `root`; `None` when it leaves `root`.
fn normalize_pattern(parsed: &Pattern, prefix: &str, root: &Path) -> Option<Pattern> {
    let mut pattern = parsed.clone();

    pattern.normalize(Path::new(prefix), root).ok()?;

    Some(pattern)
}

#[cfg(test)]
mod tests {
    use super::*;

    use gix::bstr::ByteSlice as _;
    use nu_plugin_gix_testing_support::PATHS_DIR;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::{assert_matches, fs, io};

    const WORKTREE: &str = "/work/repo";
    const SUBDIR: &str = "/work/repo/src";
    const ELSEWHERE: &str = "/elsewhere";
    const REAL_DIR: &str = "real";
    const LINK_DIR: &str = "link";

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Git(#[from] GixError),

        #[error(transparent)]
        Io(#[from] io::Error),
    }

    mod pathspec {
        use super::*;

        #[rstest]
        #[case::relative_in_subdir(SUBDIR, "main.rs", "src/main.rs")]
        #[case::glob_in_subdir(SUBDIR, "*.rs", "src/*.rs")]
        #[case::top_magic(SUBDIR, ":(top)README.md", "README.md")]
        #[case::short_top(SUBDIR, ":/README.md", "README.md")]
        #[case::parent_dir(SUBDIR, "../README.md", "README.md")]
        #[case::parent_is_root(SUBDIR, "..", "")]
        #[case::current_dir(SUBDIR, ".", "src")]
        #[case::dot_components(WORKTREE, "./dir/./b.txt", "dir/b.txt")]
        #[case::trailing_slash(WORKTREE, "dir/", "dir")]
        #[case::absolute_from_outside(ELSEWHERE, "/work/repo/a.txt", "a.txt")]
        fn resolves_to_the_repository_path(
            #[case] cwd: &str,
            #[case] spec: &str,
            #[case] expected: &str,
        ) -> Result<(), TestError> {
            let resolved = pathspec(Some(Path::new(WORKTREE)), Path::new(cwd), spec)?;

            assert_eq!(resolved.path().to_str_lossy(), expected);

            Ok(())
        }

        #[rstest]
        fn exclude_keeps_its_magic_and_resolves_the_path() -> Result<(), TestError> {
            let resolved = pathspec(
                Some(Path::new(WORKTREE)),
                Path::new(SUBDIR),
                ":(exclude)*.md",
            )?;

            assert_eq!(resolved.path().to_str_lossy(), "src/*.md");
            assert!(resolved.is_excluded());

            Ok(())
        }

        #[rstest]
        #[case::climbs_out(WORKTREE, "../other")]
        #[case::cwd_outside(ELSEWHERE, "a.txt")]
        #[case::absolute_outside(WORKTREE, "/etc/passwd")]
        fn rejects_a_path_outside_the_worktree(#[case] cwd: &str, #[case] spec: &str) {
            let result = pathspec(Some(Path::new(WORKTREE)), Path::new(cwd), spec);

            assert_matches!(
                &result,
                Err(GixError::PathOutsideRepository { path: written }) if written == spec,
                "got {result:?}"
            );
        }

        #[rstest]
        fn rejects_attribute_magic() {
            let spec = ":(attr:binary)x";
            let result = pathspec(Some(Path::new(WORKTREE)), Path::new(WORKTREE), spec);

            assert_matches!(
                &result,
                Err(GixError::PathspecAttributes { spec: written }) if written == spec,
                "got {result:?}"
            );
        }

        #[rstest]
        fn rejects_an_unparsable_pathspec() {
            let spec = ":(nonsense)x";
            let result = pathspec(Some(Path::new(WORKTREE)), Path::new(WORKTREE), spec);

            assert_matches!(
                &result,
                Err(GixError::Pathspec { spec: written, .. }) if written == spec,
                "got {result:?}"
            );
        }

        #[rstest]
        fn resolves_a_bare_repository_path_from_the_root() -> Result<(), TestError> {
            let resolved = pathspec(None, Path::new(ELSEWHERE), "dir/b.txt")?;

            assert_eq!(resolved.path().to_str_lossy(), "dir/b.txt");

            Ok(())
        }

        #[rstest]
        #[case::absolute("/abs")]
        #[case::climbs_out("../x")]
        fn rejects_a_bare_repository_path_outside_the_root(#[case] spec: &str) {
            let result = pathspec(None, Path::new(ELSEWHERE), spec);

            assert_matches!(
                &result,
                Err(GixError::PathOutsideRepository { path: written }) if written == spec,
                "got {result:?}"
            );
        }

        #[cfg(unix)]
        mod symlinks {
            use super::*;

            #[fixture]
            fn symlinked_tree() -> Result<TempDir, TestError> {
                let tmp = TempDir::new()?;

                fs::create_dir_all(tmp.path().join(REAL_DIR))?;
                symlink(tmp.path().join(REAL_DIR), tmp.path().join(LINK_DIR))?;

                Ok(tmp)
            }

            #[rstest]
            fn absolute_spec_in_the_real_path_of_a_symlinked_worktree(
                symlinked_tree: Result<TempDir, TestError>,
            ) -> Result<(), TestError> {
                let tmp = symlinked_tree?;
                let root = fs::canonicalize(tmp.path())?;
                let real = root.join(REAL_DIR);
                let link = root.join(LINK_DIR);
                let spec = format!("{}/a.txt", real.display());

                let resolved = pathspec(Some(link.as_path()), link.as_path(), &spec)?;

                assert_eq!(resolved.path().to_str_lossy(), "a.txt");

                Ok(())
            }

            #[rstest]
            fn absolute_spec_through_the_symlink_of_a_real_worktree(
                symlinked_tree: Result<TempDir, TestError>,
            ) -> Result<(), TestError> {
                let tmp = symlinked_tree?;
                let root = fs::canonicalize(tmp.path())?;
                let real = root.join(REAL_DIR);
                let link = root.join(LINK_DIR);
                let spec = format!("{}/a.txt", link.display());

                let result = pathspec(Some(real.as_path()), real.as_path(), &spec);

                assert_matches!(
                    &result,
                    Err(GixError::PathOutsideRepository { path: written }) if written == &spec,
                    "got {result:?}"
                );

                Ok(())
            }
        }
    }

    mod cwd_prefix {
        use super::*;

        #[rstest]
        #[case::worktree_root(WORKTREE, "")]
        #[case::subdir(SUBDIR, "src")]
        #[case::parent_of_subdir_is_root("/work/repo/src/..", "")]
        #[case::dot_components("/work/repo/./src/.", "src")]
        #[case::parent_components("/work/repo/src/../src/lib", "src/lib")]
        fn resolves_to_the_repository_path(#[case] cwd: &str, #[case] expected: &str) {
            assert_eq!(
                cwd_prefix(Path::new(WORKTREE), Path::new(cwd)).as_deref(),
                Some(expected)
            );
        }

        #[rstest]
        #[case::elsewhere(ELSEWHERE)]
        #[case::sibling_with_shared_prefix("/work/repo2")]
        fn rejects_a_cwd_outside_the_worktree(#[case] cwd: &str) {
            assert_eq!(cwd_prefix(Path::new(WORKTREE), Path::new(cwd)), None);
        }

        #[cfg(unix)]
        #[rstest]
        fn matches_a_current_directory_reached_through_a_symlink() -> Result<(), TestError> {
            let tmp = TempDir::new()?;
            let real = tmp.path().join(REAL_DIR);

            fs::create_dir_all(real.join(PATHS_DIR))?;

            let link = tmp.path().join(LINK_DIR);

            symlink(&real, &link)?;

            assert_eq!(
                cwd_prefix(&real, &link.join(PATHS_DIR)).as_deref(),
                Some(PATHS_DIR)
            );

            Ok(())
        }
    }
}
