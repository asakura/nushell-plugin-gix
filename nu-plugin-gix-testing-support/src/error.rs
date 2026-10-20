//! Errors that shared fixture setup can raise.

use gix::refs::name::Error as RefsNameError;

use std::{io, path::PathBuf};

/// Errors that shared fixture setup can raise.
#[derive(Debug, thiserror::Error)]
pub enum FixtureError {
    /// A git operation in the fixture repository failed.
    #[error(transparent)]
    Gix(#[from] gix::Error),

    /// A reference name in the fixture was invalid (fixture bug, not test subject).
    #[error(transparent)]
    Name(#[from] RefsNameError),

    /// A temporary directory could not be created.
    #[error(transparent)]
    Io(#[from] io::Error),

    /// The temporary directory lies inside an existing git repository, so it is not a "plain" directory.
    #[error(
        "temporary directory `{}` lies inside the git repository `{}`; set TMPDIR to a directory outside any checkout",
        .dir.display(),
        .git_dir.display()
    )]
    EnclosedInRepository {
        /// The directory that was checked.
        dir: PathBuf,
        /// The git directory of the repository that encloses `dir`.
        git_dir: PathBuf,
    },

    /// An assertion about fixture setup failed.
    #[error("Fixture failure: {0}")]
    Failure(String),
}
