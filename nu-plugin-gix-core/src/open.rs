//! Opening and discovering repositories.

use super::GixError;

use std::path::{Path, PathBuf};

/// Where an opened repository lives on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryLocation {
    /// The repository's git directory (for a linked worktree, its private `worktrees/<name>` directory).
    pub git_dir: PathBuf,
    /// The checked-out worktree, or `None` for a bare repository.
    pub worktree: Option<PathBuf>,
}

impl RepositoryLocation {
    /// The directory that opens this repository: the worktree, or the git directory when bare.
    #[must_use = "returns the path; it is not used anywhere else"]
    pub fn root(&self) -> &Path {
        self.worktree.as_deref().unwrap_or(&self.git_dir)
    }
}

/// Opens the repository at exactly `path` (no parent-directory discovery).
pub fn open(path: &Path) -> Result<gix::Repository, GixError> {
    gix::open(path).map_err(|source| GixError::Open {
        path: path.to_owned(),
        source,
    })
}

/// Finds the repository containing `path`, searching `path` and then its parents like git does.
///
/// # Errors
///
/// Returns [`GixError::NotFound`] when neither `path` nor any parent is a git repository.
pub fn discover(path: &Path) -> Result<RepositoryLocation, GixError> {
    let repo = gix::discover(path).map_err(|source| GixError::NotFound {
        path: path.to_owned(),
        source,
    })?;

    Ok(RepositoryLocation {
        git_dir: repo.git_dir().to_owned(),
        worktree: repo.workdir().map(Path::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use std::{fs, io};

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Io(#[from] io::Error),

        #[error(transparent)]
        Gix(#[from] gix::Error),

        #[error(transparent)]
        Git(#[from] GixError),

        #[error("Test failure: {0}")]
        Failure(String),
    }

    #[fixture]
    fn dir() -> Result<TempDir, TestError> {
        Ok(TempDir::new()?)
    }

    /// A temporary directory holding a freshly initialised git repository.
    #[fixture]
    fn repo_dir(dir: Result<TempDir, TestError>) -> Result<TempDir, TestError> {
        let dir = dir?;

        gix::init(dir.path())?;

        Ok(dir)
    }

    /// A temporary directory holding a freshly initialised bare repository.
    #[fixture]
    fn bare_repo_dir(dir: Result<TempDir, TestError>) -> Result<TempDir, TestError> {
        let dir = dir?;

        gix::init_bare(dir.path())?;

        Ok(dir)
    }

    mod discover {
        use super::*;

        #[rstest]
        fn finds_repository_at_path(repo_dir: Result<TempDir, TestError>) -> Result<(), TestError> {
            let dir = repo_dir?;
            let location = discover(dir.path())?;

            assert_eq!(location.root().canonicalize()?, dir.path().canonicalize()?);
            assert_eq!(
                location.git_dir.canonicalize()?,
                dir.path().join(".git").canonicalize()?
            );

            Ok(())
        }

        #[rstest]
        fn finds_parent_repository_from_subdirectory(
            repo_dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            let dir = repo_dir?;
            let child = dir.path().join("sub").join("deeper");

            fs::create_dir_all(&child)?;

            let location = discover(&child)?;

            assert_eq!(location.root().canonicalize()?, dir.path().canonicalize()?);

            Ok(())
        }

        #[rstest]
        fn normalises_parent_components(
            repo_dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            let dir = repo_dir?;

            fs::create_dir_all(dir.path().join("other"))?;

            let indirect = dir.path().join("other").join("..");

            assert_eq!(discover(&indirect)?, discover(dir.path())?);

            Ok(())
        }

        #[rstest]
        fn has_no_worktree_for_bare_repository(
            bare_repo_dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            let dir = bare_repo_dir?;
            let location = discover(dir.path())?;

            assert_eq!(location.worktree, None);
            assert_eq!(location.root(), location.git_dir);

            Ok(())
        }

        #[rstest]
        fn rejects_plain_directory(dir: Result<TempDir, TestError>) -> Result<(), TestError> {
            let dir = dir?;

            let err = discover(dir.path())
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert!(matches!(err, GixError::NotFound { .. }), "got {err:?}");

            Ok(())
        }
    }
}
