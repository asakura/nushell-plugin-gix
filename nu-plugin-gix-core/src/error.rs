//! Error type for the git layer; knows nothing about Nushell.

use crate::refs::{RefExpect, RefTarget};

use std::{io, path::PathBuf};

/// Errors produced while opening a repository or reading its history.
#[derive(Debug, thiserror::Error)]
pub enum GixError {
    /// No repository contains `path`, neither the directory itself nor any of its parents.
    #[error("no git repository found in '{}' or any parent directory", path.display())]
    NotFound {
        /// The directory the search started from.
        path: PathBuf,
        /// The underlying `gix` error.
        #[source]
        source: gix::Error,
    },

    /// `gix` failed to open a repository that was found.
    #[error("could not open git repository at '{}'", path.display())]
    Open {
        /// The directory that was opened.
        path: PathBuf,
        /// The underlying `gix` error.
        #[source]
        source: gix::Error,
    },

    /// The repository is no longer at its location: its git directory or worktree is gone, or the
    /// path now holds a different repository.
    #[error("git repository at '{}' was moved or deleted", git_dir.display())]
    RepositoryGone {
        /// The git directory that disappeared.
        git_dir: PathBuf,
    },

    /// HEAD could not be read or peeled to an object id. An unborn branch is reported as
    /// [`GixError::UnbornHead`] instead.
    #[error("could not resolve HEAD")]
    Head(#[source] gix::Error),

    /// A revision expression could not be resolved in the repository.
    #[error("could not resolve revision '{expression}'")]
    Revision {
        /// The revision expression as written.
        expression: String,
        /// The underlying `gix` error.
        #[source]
        source: gix::Error,
    },

    /// A resolved object id could not be shortened because its object could not be read.
    #[error("could not shorten the object id of revision '{expression}'")]
    Shorten {
        /// The revision expression as written.
        expression: String,
        /// The underlying `gix` error.
        #[source]
        source: gix::Error,
    },

    /// Setting up or running the revision walk failed.
    #[error("revision walk failed")]
    Walk(#[source] gix::Error),

    /// Reading or decoding a commit failed.
    #[error("could not read commit {id}")]
    Commit {
        /// Hex id of the commit.
        id: String,
        /// The underlying `gix` error.
        #[source]
        source: gix::Error,
    },

    /// A commit timestamp could not be represented as a date.
    #[error("commit {id} has an out-of-range timestamp")]
    Timestamp {
        /// Hex id of the commit.
        id: String,
    },

    /// The background thread that walks history could not be started.
    #[error("could not start the history walker")]
    Spawn(#[source] io::Error),

    /// HEAD points at a branch that has no commits yet (e.g. a fresh repository).
    #[error("HEAD points at branch '{name}', which has no commits yet")]
    UnbornHead {
        /// Full name of the unborn branch.
        name: String,
    },

    /// Reading a reference or the reference list failed.
    #[error("could not read references")]
    RefRead(#[source] gix::Error),

    /// A symbolic reference chain is broken: it never reaches an object id.
    #[error(
        "reference '{name}' never resolves to an object; its symbolic chain is broken or too deep"
    )]
    DanglingRef {
        /// Full name of the reference that could not be resolved.
        name: String,
    },

    /// A compare-and-swap reference update failed because the reference moved concurrently.
    #[error(
        "reference '{name}' moved concurrently: expected {expected}, but found {}",
        .actual.as_ref().map_or(RefExpect::MISSING, RefTarget::spelled)
    )]
    RefMoved {
        /// Full name of the reference.
        name: String,
        /// The value the update required the reference to hold.
        expected: RefExpect,
        /// What the reference actually holds; `None` when it does not exist.
        actual: Option<RefTarget>,
    },

    /// Writing a reference failed after its value agreed with the expectation.
    #[error("could not update reference '{name}'")]
    RefUpdate {
        /// Full name of the reference.
        name: String,
        /// The underlying `gix` error.
        #[source]
        source: gix::Error,
    },

    /// A value that should be an object id is not one.
    #[error("'{value}' is not a valid object id")]
    InvalidOid {
        /// The value as written.
        value: String,
    },

    /// A reference name is not a valid full git reference name.
    #[error("'{name}' is not a valid reference name: {detail}")]
    InvalidRefName {
        /// The name as written.
        name: String,
        /// Why the name was rejected.
        detail: String,
    },

    /// A requested short object id length is out of range.
    #[error("invalid short length {len}: {detail}")]
    ShortLen {
        /// The requested length in hexadecimal characters.
        len: usize,
        /// Why the length was rejected.
        detail: String,
    },
}
