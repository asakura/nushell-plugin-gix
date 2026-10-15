//! Opening and validating repositories.

use super::GixError;

use std::path::Path;

/// Opens the repository at exactly `path` (no parent-directory discovery).
pub(super) fn open(path: &Path) -> Result<gix::Repository, GixError> {
    gix::open(path).map_err(|source| GixError::Open {
        path: path.to_owned(),
        source,
    })
}

/// Checks that `path` is a git repository.
///
/// # Errors
///
/// Returns [`GixError::Open`] when `path` is not a git repository.
pub fn verify(path: &Path) -> Result<(), GixError> {
    open(path).map(drop)
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

    mod verify {
        use super::*;

        #[rstest]
        fn accepts_existing_repository(dir: Result<TempDir, TestError>) -> Result<(), TestError> {
            let dir = dir?;

            gix::init(dir.path())?;
            verify(dir.path())?;

            Ok(())
        }

        #[rstest]
        fn rejects_plain_directory(dir: Result<TempDir, TestError>) -> Result<(), TestError> {
            let dir = dir?;

            let err = verify(dir.path())
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert!(matches!(err, GixError::Open { .. }), "got {err:?}");

            Ok(())
        }

        #[rstest]
        fn does_not_discover_parent_repository(
            dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            let dir = dir?;
            gix::init(dir.path())?;

            let child = dir.path().join("sub");
            fs::create_dir_all(&child)?;

            assert!(matches!(verify(&child), Err(GixError::Open { .. })));

            Ok(())
        }
    }
}
