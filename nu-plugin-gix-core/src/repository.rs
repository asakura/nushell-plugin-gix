//! Opening and discovering repositories.

use super::GixError;

use gix::error::Class;

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
    /// The location of an already opened repository.
    #[must_use = "returns the location; the repository is unchanged"]
    pub(crate) fn from_repository(repo: &gix::Repository) -> Self {
        Self {
            git_dir: repo.git_dir().to_owned(),
            worktree: repo.workdir().map(Path::to_owned),
        }
    }

    /// The directory that opens this repository: the worktree, or the git directory when bare.
    #[must_use = "returns the directory that opens the repository; ignoring it does nothing"]
    pub fn root(&self) -> &Path {
        self.worktree.as_deref().unwrap_or(&self.git_dir)
    }

    /// Whether the repository is still on disk: its git directory exists and, when it has a
    /// worktree, so does the worktree.
    #[must_use = "returns whether the repository still exists; the check has no other effect"]
    pub fn exists(&self) -> bool {
        self.git_dir.is_dir() && self.worktree.as_deref().is_none_or(Path::is_dir)
    }

    /// Finds the repository containing `path`, searching `path` and then its parents like git does.
    ///
    /// # Errors
    ///
    /// Returns [`GixError::NotFound`] when neither `path` nor any parent is a git repository, and
    /// [`GixError::Open`] when a repository is found but cannot be opened, for example because its
    /// configuration is corrupt or its owner is not trusted.
    pub fn discover(path: &Path) -> Result<Self, GixError> {
        OpenRepository::discover(path).map(|(_, location)| location)
    }
}

/// An opened repository shared between commands; `gix` types never leave the git side.
#[derive(Debug)]
pub struct OpenRepository {
    git_dir: PathBuf,
    repo: gix::ThreadSafeRepository,
}

impl OpenRepository {
    /// Opens the repository at exactly the root of `location`, without parent-directory discovery.
    ///
    /// # Errors
    ///
    /// Returns [`GixError::RepositoryGone`] when the repository found at the root is not the one
    /// `location` names, for example because the worktree was deleted and another repository now
    /// sits at its path, and [`GixError::Open`] when the repository cannot be opened.
    pub fn open(location: &RepositoryLocation) -> Result<Self, GixError> {
        let root = location.root();

        let repo = gix::open(root).map_err(|source| GixError::Open {
            path: root.to_owned(),
            source,
        })?;

        if !same_directory(repo.git_dir(), &location.git_dir) {
            return Err(GixError::RepositoryGone {
                git_dir: location.git_dir.clone(),
            });
        }

        // Callers key the registry by the location's git directory, so it is kept as given
        // rather than re-derived from the opened repository.
        Ok(Self {
            git_dir: location.git_dir.clone(),
            repo: repo.into_sync(),
        })
    }

    /// Wraps an already opened repository, taking its git directory from the repository itself.
    #[must_use = "the handle is the only way to read the repository"]
    pub(crate) fn from_repository(repo: gix::Repository) -> Self {
        Self {
            git_dir: repo.git_dir().to_owned(),
            repo: repo.into_sync(),
        }
    }

    /// Finds the repository containing `path`, searching `path` and then its parents like git does,
    /// and returns the opened repository together with its location.
    ///
    /// The repository is opened once, so the returned handle is the one discovery produced.
    ///
    /// # Errors
    ///
    /// Returns [`GixError::NotFound`] when neither `path` nor any parent is a git repository, and
    /// [`GixError::Open`] when a repository is found but cannot be opened, for example because its
    /// configuration is corrupt or its owner is not trusted.
    pub(crate) fn discover(path: &Path) -> Result<(Self, RepositoryLocation), GixError> {
        let repo = gix::discover(path).map_err(|source| {
            if is_not_found(path, &source) {
                GixError::NotFound {
                    path: path.to_owned(),
                    source,
                }
            } else {
                GixError::Open {
                    path: path.to_owned(),
                    source,
                }
            }
        })?;

        let location = RepositoryLocation::from_repository(&repo);

        Ok((Self::from_repository(repo), location))
    }

    /// The git directory this handle was opened from.
    #[must_use = "returns the git directory; ignoring it does nothing"]
    pub(crate) fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    /// A repository handle for the calling thread.
    #[must_use = "the handle is the only way to read the repository"]
    pub(crate) fn repo(&self) -> gix::Repository {
        self.repo.to_thread_local()
    }
}

/// Whether a failed discovery found no repository at all, rather than one that could not be opened.
///
/// gix reports a start path that is not a directory as a validation error, but a corrupt
/// configuration inside a real directory is also a validation error; only the former counts as not found.
fn is_not_found(path: &Path, source: &gix::Error) -> bool {
    match source.dominant_class() {
        Some(Class::NotFound) => true,
        Some(Class::Validation) => !path.is_dir(),
        _ => false,
    }
}

/// Whether `opened` and `expected` name the same directory; a path that cannot be canonicalized
/// (for example because it was deleted) names none.
fn same_directory(opened: &Path, expected: &Path) -> bool {
    match (opened.canonicalize(), expected.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use nu_plugin_gix_testing_support::{
        CorruptTestRepo, FixtureError, GIT_DIR, UnbornTestRepo, bare_repo, corrupt_repo, plain_dir,
        unborn_repo,
    };
    use rstest::rstest;
    use tempfile::TempDir;

    use std::{assert_matches, fs, io};

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Io(#[from] io::Error),

        #[error(transparent)]
        Gix(#[from] gix::Error),

        #[error(transparent)]
        Git(#[from] GixError),

        #[error(transparent)]
        Fixture(#[from] FixtureError),

        #[error("Test failure: {0}")]
        Failure(String),
    }

    mod repository_location {
        use super::*;

        mod discover {
            use super::*;

            #[rstest]
            fn finds_repository_at_path(
                unborn_repo: Result<UnbornTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let dir = unborn_repo?.dir;
                let location = RepositoryLocation::discover(dir.path())?;

                assert_eq!(location.root().canonicalize()?, dir.path().canonicalize()?);

                assert_eq!(
                    location.git_dir.canonicalize()?,
                    dir.path().join(GIT_DIR).canonicalize()?
                );

                Ok(())
            }

            #[rstest]
            fn finds_parent_repository_from_subdirectory(
                unborn_repo: Result<UnbornTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let dir = unborn_repo?.dir;
                let child = dir.path().join("sub").join("deeper");

                fs::create_dir_all(&child)?;

                let location = RepositoryLocation::discover(&child)?;

                assert_eq!(location.root().canonicalize()?, dir.path().canonicalize()?);

                Ok(())
            }

            #[rstest]
            fn normalises_parent_components(
                unborn_repo: Result<UnbornTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let dir = unborn_repo?.dir;

                fs::create_dir_all(dir.path().join("other"))?;

                let indirect = dir.path().join("other").join("..");

                assert_eq!(
                    RepositoryLocation::discover(&indirect)?,
                    RepositoryLocation::discover(dir.path())?
                );

                Ok(())
            }

            #[rstest]
            fn has_no_worktree_for_bare_repository(
                bare_repo: Result<UnbornTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let dir = bare_repo?.dir;
                let location = RepositoryLocation::discover(dir.path())?;

                assert_eq!(location.worktree, None);
                assert_eq!(location.root(), location.git_dir);

                Ok(())
            }

            // Existing plain directories are not used here: discovery also searches their parents,
            // so the outcome would depend on whether the temporary directory sits inside a repository.
            #[rstest]
            fn rejects_missing_path(
                plain_dir: Result<TempDir, FixtureError>,
            ) -> Result<(), TestError> {
                let dir = plain_dir?;

                let err = RepositoryLocation::discover(&dir.path().join("missing"))
                    .err()
                    .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

                assert_matches!(err, GixError::NotFound { .. }, "got {err:?}");

                Ok(())
            }

            #[rstest]
            fn rejects_file_path(
                plain_dir: Result<TempDir, FixtureError>,
            ) -> Result<(), TestError> {
                let dir = plain_dir?;
                let file = dir.path().join("file.txt");

                fs::write(&file, "")?;

                let err = RepositoryLocation::discover(&file)
                    .err()
                    .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

                assert_matches!(err, GixError::NotFound { .. }, "got {err:?}");

                Ok(())
            }

            #[rstest]
            fn reports_corrupt_repository_as_open_failure(
                corrupt_repo: Result<CorruptTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let dir = corrupt_repo?.dir;

                let err = RepositoryLocation::discover(dir.path())
                    .err()
                    .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

                assert_matches!(err, GixError::Open { .. }, "got {err:?}");

                Ok(())
            }
        }

        mod exists {
            use super::*;

            #[rstest]
            fn is_true_for_an_existing_repository(
                unborn_repo: Result<UnbornTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let dir = unborn_repo?.dir;

                assert!(RepositoryLocation::discover(dir.path())?.exists());

                Ok(())
            }

            #[rstest]
            fn is_false_when_the_worktree_is_gone(
                unborn_repo: Result<UnbornTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let dir = unborn_repo?.dir;

                let location = RepositoryLocation {
                    git_dir: dir.path().join(GIT_DIR),
                    worktree: Some(dir.path().join("gone")),
                };

                assert!(!location.exists());

                Ok(())
            }
        }
    }

    mod open_repository {
        use super::*;

        mod open {
            use super::*;

            #[rstest]
            fn opens_the_repository_at_the_location(
                unborn_repo: Result<UnbornTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let dir = unborn_repo?.dir;
                let location = RepositoryLocation::discover(dir.path())?;
                let opened = OpenRepository::open(&location)?;

                assert_eq!(opened.git_dir(), location.git_dir);

                Ok(())
            }

            #[rstest]
            fn reports_repository_gone_when_the_worktree_holds_another_repository(
                unborn_repo: Result<UnbornTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let dir = unborn_repo?.dir;
                let other = TempDir::new()?;

                gix::init(other.path())?;

                let location = RepositoryLocation {
                    git_dir: dir.path().join(GIT_DIR),
                    worktree: Some(other.path().to_owned()),
                };

                let err = OpenRepository::open(&location)
                    .err()
                    .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

                assert_matches!(err, GixError::RepositoryGone { .. }, "got {err:?}");

                Ok(())
            }

            #[rstest]
            fn opens_bare_repository(
                bare_repo: Result<UnbornTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let dir = bare_repo?.dir;
                let location = RepositoryLocation::discover(dir.path())?;

                assert_eq!(location.worktree, None);
                assert_eq!(location.root(), location.git_dir);

                let opened = OpenRepository::open(&location)?;

                assert_eq!(opened.git_dir(), location.git_dir);

                Ok(())
            }

            #[rstest]
            fn errors_with_open_for_a_missing_path(
                plain_dir: Result<TempDir, FixtureError>,
            ) -> Result<(), TestError> {
                let dir = plain_dir?;
                let missing = dir.path().join("missing");

                let location = RepositoryLocation {
                    git_dir: missing.join(GIT_DIR),
                    worktree: Some(missing),
                };

                let err = OpenRepository::open(&location)
                    .err()
                    .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

                assert_matches!(err, GixError::Open { .. }, "got {err:?}");

                Ok(())
            }

            #[rstest]
            fn errors_with_open_for_a_corrupt_config(
                corrupt_repo: Result<CorruptTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let dir = corrupt_repo?.dir;

                let location = RepositoryLocation {
                    git_dir: dir.path().join(GIT_DIR),
                    worktree: Some(dir.path().to_owned()),
                };

                let err = OpenRepository::open(&location)
                    .err()
                    .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

                assert_matches!(err, GixError::Open { .. }, "got {err:?}");

                Ok(())
            }

            // Opening is exact: the plain subdirectory is inside the unborn repository's worktree,
            // so discovery would find that repository, but `open` must not look past the given root.
            #[rstest]
            fn does_not_discover_parent_repository(
                unborn_repo: Result<UnbornTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let dir = unborn_repo?.dir;
                let child = dir.path().join("sub");

                fs::create_dir_all(&child)?;

                let location = RepositoryLocation {
                    git_dir: child.join(GIT_DIR),
                    worktree: Some(child),
                };

                let err = OpenRepository::open(&location)
                    .err()
                    .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

                assert_matches!(err, GixError::Open { .. }, "got {err:?}");

                Ok(())
            }
        }
    }
}
