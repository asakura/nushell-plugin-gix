//! The path filter that decides which commits a walk lists and which parents it follows.
//!
//! How [`PathFilter::simplify`] treats a commit, as `git log -- <paths>` does:
//!
//! | Commit | Listed | Followed |
//! |---|---|---|
//! | empty filter | yes | every parent |
//! | root commit that contains a filtered path | yes | none |
//! | root commit that contains none | no | none |
//! | equals some parent at the paths | no | the first such parent only |
//! | differs from every parent at the paths | yes | every parent |

use crate::GixError;
use crate::log::changes::{FileChange, changed_files_between, for_each_change};

use gix::{
    bstr::{BStr, ByteSlice as _},
    pathspec::{Pattern, Search},
};

use std::{ops::ControlFlow, path::Path};

/// Repository-relative paths that decide which commits are listed and which parents a walk follows.
#[derive(Clone, Debug, Default)]
pub(in crate::log) struct PathFilter {
    /// The pathspecs, matched against the repository-relative path of each changed entry; `None` names the
    /// whole tree.
    search: Option<Search>,
}

/// How a commit is treated under history simplification at the filtered paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::log) struct Simplified {
    /// Whether the commit is listed.
    pub listed: bool,
    /// The first parent whose entries at the paths equal the commit's, when there is one; the walk then
    /// follows only that parent. `None` means the walk follows every parent.
    pub follow: Option<gix::ObjectId>,
}

impl PathFilter {
    /// Builds the filter from `paths`, patterns made by [`pathspec`](super::pathspec) so that globs and magic such as
    /// `:(exclude)`, `:(icase)` and a trailing `/` behave as in `git log -- <paths>`. An empty list names the
    /// whole tree.
    ///
    /// # Errors
    ///
    /// Returns [`GixError::Pathspec`] when gix cannot build the search from the pathspecs, naming the first
    /// pattern that fails on its own, or all of them when each builds alone.
    #[must_use = "the filter is the only result of building it from the paths"]
    pub(in crate::log) fn new(paths: &[Pattern]) -> Result<Self, GixError> {
        if paths.is_empty() {
            return Ok(Self::default());
        }

        let patterns = paths.iter().cloned();
        let search = Search::from_specs(patterns, None, Path::new(""))
            .map_err(|source| invalid_spec(paths, source))?;

        Ok(Self {
            search: Some(search),
        })
    }

    /// Whether the filter names no paths, so that every commit is listed.
    #[must_use = "the result decides whether history is simplified"]
    pub(in crate::log) const fn is_empty(&self) -> bool {
        self.search.is_none()
    }

    /// Decides how `commit` is treated, as `git log -- <paths>` does. A commit is listed when it differs
    /// from every parent at some path, and a root commit when it contains one of the paths. A merge that
    /// is identical to a parent at the paths is not listed, and only the first such parent is followed.
    /// `parents` are the parents compared: the walk passes only the first parent under `--first-parent`, as
    /// `git log --first-parent -- <paths>` compares only the first parent. An empty filter lists every
    /// commit and follows every parent.
    ///
    /// # Errors
    ///
    /// Returns the `gix` error when a tree or a parent commit cannot be read.
    #[must_use = "the result decides whether the commit is listed and which parent is followed"]
    pub(in crate::log) fn simplify(
        &mut self,
        repo: &gix::Repository,
        commit: &gix::Commit<'_>,
        parents: &[gix::ObjectId],
    ) -> Result<Simplified, gix::Error> {
        if self.is_empty() {
            return Ok(Simplified {
                listed: true,
                follow: None,
            });
        }

        let ours = commit.tree()?;

        if parents.is_empty() {
            return Ok(Simplified {
                listed: self.differs(&repo.empty_tree(), &ours)?,
                follow: None,
            });
        }

        for parent in parents {
            let theirs = repo.find_commit(*parent)?.tree()?;

            if !self.differs(&theirs, &ours)? {
                return Ok(Simplified {
                    listed: false,
                    follow: Some(*parent),
                });
            }
        }

        Ok(Simplified {
            listed: true,
            follow: None,
        })
    }

    /// Like [`simplify`](Self::simplify), and also returns the files the commit changes against its only parent,
    /// as [`changed_files`](crate::log::changes::changed_files) would, from the same tree diff. The files are
    /// `None` when the filter is empty or the commit has several parents; the caller then lists them itself.
    /// With `stat` each file carries its line counts.
    ///
    /// # Errors
    ///
    /// Returns the `gix` error when a tree, a parent commit or a blob cannot be read.
    #[must_use = "the result decides whether the commit is listed and which files it changes"]
    pub(in crate::log) fn simplify_with_files(
        &mut self,
        repo: &gix::Repository,
        commit: &gix::Commit<'_>,
        parents: &[gix::ObjectId],
        stat: bool,
    ) -> Result<(Simplified, Option<Vec<FileChange>>), gix::Error> {
        if self.is_empty() || parents.len() > 1 {
            return Ok((self.simplify(repo, commit, parents)?, None));
        }

        let ours = commit.tree()?;
        let parent = parents.first().copied();

        let theirs = match parent {
            Some(id) => repo.find_commit(id)?.tree()?,
            None => repo.empty_tree(),
        };

        let files = changed_files_between(repo, &theirs, &ours, self, stat)?;
        let unchanged = files.is_empty();

        Ok((
            Simplified {
                listed: !unchanged,
                follow: parent.filter(|_| unchanged),
            },
            Some(files),
        ))
    }

    /// Whether an entry named by the filter differs between `old` and `new`; stops at the first one.
    fn differs(&mut self, old: &gix::Tree<'_>, new: &gix::Tree<'_>) -> Result<bool, gix::Error> {
        let mut found = false;

        for_each_change(old, new, |path, _kind| {
            found = self.matches(path);
            if found {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })?;

        Ok(found)
    }

    /// Whether a positive pattern names `path` and no exclude pattern does. Every path matches an empty filter.
    #[must_use = "the result decides whether the change is listed"]
    pub(in crate::log) fn matches(&mut self, path: &BStr) -> bool {
        self.search.as_mut().is_none_or(|search| {
            search
                .pattern_matching_relative_path(path, Some(false), &mut |_, _, _, _| false)
                .is_some_and(|found| !found.is_excluded())
        })
    }
}

/// Reports the pattern that gix rejects on its own, or every pattern joined when each builds alone, so the
/// error names the spec the user wrote.
fn invalid_spec(paths: &[Pattern], source: gix::Error) -> GixError {
    let offending = paths.iter().find_map(|pattern| {
        Search::from_specs([pattern.clone()], None, Path::new(""))
            .err()
            .map(|error| GixError::Pathspec {
                spec: pattern.path().to_str_lossy().into_owned(),
                source: error,
            })
    });

    offending.unwrap_or_else(|| GixError::Pathspec {
        spec: paths
            .iter()
            .map(|pattern| pattern.path().to_str_lossy())
            .collect::<Vec<_>>()
            .join(" "),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fixtures::open_path;
    use crate::log::paths::pathspec;

    use gix::pathspec::{Defaults, parse};
    use nu_plugin_gix_testing_support::{
        COMMITTER_TIME, FixtureError, FixtureRepo as _, PATHS_DIR, PATHS_FILE, PATHS_NESTED_FILE,
        PathsTestRepo, committer, paths_repo,
    };
    use rstest::rstest;

    use std::assert_matches;

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Gix(#[from] gix::Error),

        #[error(transparent)]
        Git(#[from] GixError),

        #[error(transparent)]
        Fixture(#[from] FixtureError),
    }

    mod simplify {
        use super::*;

        const MISSING_PATH: &str = "missing";
        const WHOLE_TREE: &str = ".";
        const MERGE_MESSAGE: &str = "merge";

        #[derive(Clone, Copy, Debug)]
        enum Step {
            AddFile,
            AddDir,
            EditFile,
            EditNested,
            Unchanged,
        }

        impl Step {
            fn id(self, fixture: &PathsTestRepo) -> gix::ObjectId {
                match self {
                    Self::AddFile => fixture.add_file,
                    Self::AddDir => fixture.add_dir,
                    Self::EditFile => fixture.edit_file,
                    Self::EditNested => fixture.edit_nested,
                    Self::Unchanged => fixture.unchanged,
                }
            }
        }

        fn simplified(
            fixture: &PathsTestRepo,
            step: Step,
            paths: &[&str],
        ) -> Result<Simplified, TestError> {
            let repo = open_path(fixture.path())?.repo();
            let commit = repo.find_commit(step.id(fixture))?;
            let parents: Vec<gix::ObjectId> = commit.parent_ids().map(gix::Id::detach).collect();

            let specs = paths
                .iter()
                .map(|spec| pathspec(Some(fixture.path()), fixture.path(), spec))
                .collect::<Result<Vec<Pattern>, _>>()?;

            Ok(PathFilter::new(&specs)?.simplify(&repo, &commit, &parents)?)
        }

        #[rstest]
        #[case::file_changed(Step::EditFile, &[PATHS_FILE], true)]
        #[case::file_left_alone(Step::EditNested, &[PATHS_FILE], false)]
        #[case::change_below_a_directory(Step::EditNested, &[PATHS_DIR], true)]
        #[case::nested_file_added(Step::AddDir, &[PATHS_NESTED_FILE], true)]
        #[case::root_adds_the_path(Step::AddFile, &[PATHS_FILE], true)]
        #[case::root_lacks_the_path(Step::AddFile, &[PATHS_DIR], false)]
        #[case::any_of_several_paths(Step::EditNested, &[PATHS_FILE, PATHS_DIR], true)]
        #[case::missing_path(Step::EditFile, &[MISSING_PATH], false)]
        #[case::whole_tree_changed(Step::EditFile, &[WHOLE_TREE], true)]
        #[case::whole_tree_unchanged(Step::Unchanged, &[WHOLE_TREE], false)]
        #[case::no_paths(Step::Unchanged, &[], true)]
        #[case::glob(Step::EditFile, &["*.txt"], true)]
        #[case::glob_misses(Step::EditFile, &["*.rs"], false)]
        #[case::exclude_drops_the_only_change(Step::EditFile, &[":(exclude)a.txt"], false)]
        #[case::exclude_keeps_other_changes(Step::EditNested, &[":!a.txt"], true)]
        #[case::icase(Step::EditFile, &[":(icase)A.TXT"], true)]
        #[case::current_dir_prefix(Step::EditNested, &["./dir"], true)]
        #[case::trailing_slash(Step::EditNested, &["dir/"], true)]
        fn keeps_a_commit_only_when_it_changes_a_listed_path(
            paths_repo: Result<PathsTestRepo, FixtureError>,
            #[case] step: Step,
            #[case] paths: &[&str],
            #[case] expected: bool,
        ) -> Result<(), TestError> {
            let fixture = paths_repo?;

            assert_eq!(simplified(&fixture, step, paths)?.listed, expected);

            Ok(())
        }

        #[rstest]
        #[case::first_identical_parent_is_followed(Step::EditFile, Step::Unchanged, Step::EditFile)]
        #[case::order_of_the_parents_decides(Step::Unchanged, Step::EditFile, Step::Unchanged)]
        fn follows_only_the_first_parent_identical_at_the_paths(
            paths_repo: Result<PathsTestRepo, FixtureError>,
            #[case] first: Step,
            #[case] second: Step,
            #[case] expected: Step,
        ) -> Result<(), TestError> {
            let fixture = paths_repo?;
            let repo = open_path(fixture.path())?.repo();
            let parents = vec![first.id(&fixture), second.id(&fixture)];
            let tree = repo.find_commit(fixture.unchanged)?.tree_id()?.detach();

            let merge = repo
                .new_commit_as(
                    committer(COMMITTER_TIME),
                    committer(COMMITTER_TIME),
                    MERGE_MESSAGE,
                    tree,
                    parents.clone(),
                )?
                .id()
                .detach();

            let commit = repo.find_commit(merge)?;
            let specs = vec![pathspec(Some(fixture.path()), fixture.path(), PATHS_FILE)?];

            assert_eq!(
                PathFilter::new(&specs)?.simplify(&repo, &commit, &parents)?,
                Simplified {
                    listed: false,
                    follow: Some(expected.id(&fixture)),
                }
            );

            Ok(())
        }
    }
    mod new {
        use super::*;

        /// Parses, but leaves the repository once normalized.
        const ESCAPING_PATH: &str = "../outside";

        #[rstest]
        fn reports_a_path_that_leaves_the_repository_as_an_invalid_pathspec()
        -> Result<(), TestError> {
            let escaping = parse(ESCAPING_PATH.as_bytes(), Defaults::default())?;

            assert_matches!(PathFilter::new(&[escaping]), Err(GixError::Pathspec { .. }));

            Ok(())
        }
    }
}
