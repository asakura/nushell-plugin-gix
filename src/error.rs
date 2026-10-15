//! Error type for the git layer, converted to a Nushell `LabeledError` only at the command boundary.

use nu_protocol::{LabeledError, Span};

use std::{error::Error as _, io, path::PathBuf};

/// Errors produced while opening a repository or reading its history.
#[derive(Debug, thiserror::Error)]
pub enum GixError {
    /// `gix` failed to open the repository.
    #[error("could not open git repository at '{}'", path.display())]
    Open {
        /// The directory that was opened.
        path: PathBuf,
        /// The underlying `gix` error.
        #[source]
        source: gix::Error,
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

    /// A negative value was given for the commit limit.
    #[error("limit must not be negative, got {0}")]
    NegativeLimit(i64),
}

impl GixError {
    /// Converts into a `LabeledError` pointing at `span`, appending the source chain as help text.
    #[must_use = "returns the converted error; the original is consumed"]
    pub fn into_labeled(self, span: Span) -> LabeledError {
        let mut chain = Vec::new();
        let mut source = self.source();

        while let Some(err) = source {
            chain.push(err.to_string());
            source = err.source();
        }

        let labeled = Self::label(&self).map_or_else(
            || LabeledError::new(self.to_string()),
            |label| LabeledError::new(self.to_string()).with_label(label, span),
        );

        if chain.is_empty() {
            labeled
        } else {
            labeled.with_help(chain.join(": "))
        }
    }

    const fn label(&self) -> Option<&'static str> {
        match self {
            Self::Open {
                path: _path,
                source: _source,
            } => Some("run this from inside a git repository"),
            Self::Head(_) => Some("the repository may have no commits yet"),
            Self::NegativeLimit(_) => Some("expected a non-negative integer"),
            Self::Walk(_) | Self::Spawn(_) => None,
            Self::Commit {
                id: _id,
                source: _source,
            } => None,
            Self::Timestamp { id: _id } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use rstest::rstest;

    mod into_labeled {
        use super::*;

        #[rstest]
        fn keeps_message_and_label() {
            let labeled = GixError::NegativeLimit(-3).into_labeled(Span::test_data());

            assert_eq!(labeled.msg, "limit must not be negative, got -3");
            assert_eq!(labeled.labels.len(), 1);
        }

        #[rstest]
        fn has_no_label_for_walk_errors() {
            let err = GixError::Timestamp {
                id: "abc".to_owned(),
            };

            let labeled = err.into_labeled(Span::test_data());

            assert!(labeled.labels.is_empty());
        }
    }
}
