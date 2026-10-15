//! Command-level error that is converted to a Nushell `LabeledError` at the `run` boundary.

use crate::git::GixError;

use nu_protocol::{LabeledError, Span};

use std::error::Error as _;

/// Errors a command can raise: invalid arguments or a failure in the git layer.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    /// The git layer failed.
    #[error(transparent)]
    Git(#[from] GixError),

    /// A negative value was given for the commit limit.
    #[error("limit must not be negative, got {0}")]
    NegativeLimit(i64),
}

impl CommandError {
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
            Self::NegativeLimit(_) => Some("expected a non-negative integer"),
            Self::Git(GixError::Open {
                path: _path,
                source: _source,
            }) => Some("run this from inside a git repository"),
            Self::Git(GixError::Head(_)) => Some("the repository may have no commits yet"),
            Self::Git(_) => None,
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
            let labeled = CommandError::NegativeLimit(-3).into_labeled(Span::test_data());

            assert_eq!(labeled.msg, "limit must not be negative, got -3");
            assert_eq!(labeled.labels.len(), 1);
        }

        #[rstest]
        fn has_no_label_for_walk_errors() {
            let err = CommandError::Git(GixError::Timestamp {
                id: "abc".to_owned(),
            });

            let labeled = err.into_labeled(Span::test_data());

            assert!(labeled.labels.is_empty());
        }

        #[rstest]
        fn forwards_git_message() {
            let err = CommandError::from(GixError::Timestamp {
                id: "abc".to_owned(),
            });

            let labeled = err.into_labeled(Span::test_data());

            assert_eq!(labeled.msg, "commit abc has an out-of-range timestamp");
        }
    }
}
