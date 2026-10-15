//! Error type for the git layer; knows nothing about Nushell.

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

    /// The git directory of an opened repository no longer exists.
    #[error("git repository at '{}' was moved or deleted", git_dir.display())]
    RepositoryGone {
        /// The git directory that disappeared.
        git_dir: PathBuf,
    },

    /// HEAD does not point at a commit (e.g. an empty repository).
    #[error("could not resolve HEAD")]
    Head(#[source] gix::Error),

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
}
