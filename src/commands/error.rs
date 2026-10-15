//! Command-level error that is converted to a Nushell `LabeledError` at the `run` boundary.

use crate::git::GixError;

use nu_protocol::{LabeledError, Span};

use std::{error::Error as _, path::PathBuf};

/// What a command needed and what its pipeline input actually was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrongInput {
    /// Nushell type name the command expects.
    pub expected: &'static str,
    /// How to obtain a value of that type.
    pub hint: &'static str,
    /// Nushell type name of what was received.
    pub got: String,
    /// Span of the offending input.
    pub span: Span,
}

/// Errors a command can raise: invalid arguments or a failure in the git layer.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    /// The git layer failed.
    #[error(transparent)]
    Git(#[from] GixError),

    /// The path given to `--repo` is not inside a git repository.
    #[error("'{}' is not inside a git repository", path.display())]
    NotARepository {
        /// The path as the caller wrote it.
        path: PathBuf,
        /// Span of the `--repo` value.
        span: Span,
        /// The underlying git-layer failure.
        #[source]
        source: GixError,
    },

    /// The pipeline input is not the handle type the command works on.
    #[error("expected a `{}` value", .0.expected)]
    WrongInput(WrongInput),

    /// A negative value was given for the commit limit.
    #[error("limit must not be negative, got {0}")]
    NegativeLimit(i64),
}

impl CommandError {
    /// Converts into a `LabeledError` pointing at `span`, appending the source chain as help text.
    #[must_use = "returns the converted error; the original is consumed"]
    pub fn into_labeled(self, span: Span) -> LabeledError {
        let mut chain = Vec::new();
        let mut source = match &self {
            Self::NotARepository {
                path: _path,
                span: _span,
                source: found,
            } => found.source(),
            Self::Git(_) | Self::NegativeLimit(_) | Self::WrongInput(_) => self.source(),
        };

        while let Some(err) = source {
            chain.push(err.to_string());
            source = err.source();
        }

        let span = self.own_span().unwrap_or(span);

        let labeled = match &self {
            Self::WrongInput(input) => LabeledError::new(self.to_string())
                .with_label(format!("got {}", input.got), span)
                .with_help(input.hint),
            _ => Self::label(&self).map_or_else(
                || LabeledError::new(self.to_string()),
                |label| LabeledError::new(self.to_string()).with_label(label, span),
            ),
        };

        if chain.is_empty() {
            labeled
        } else {
            labeled.with_help(chain.join(": "))
        }
    }

    const fn own_span(&self) -> Option<Span> {
        match self {
            Self::NotARepository {
                span: flag_span,
                path: _path,
                source: _source,
            } => Some(*flag_span),
            Self::WrongInput(input) => Some(input.span),
            Self::Git(_) | Self::NegativeLimit(_) => None,
        }
    }

    const fn label(&self) -> Option<&'static str> {
        match self {
            Self::NotARepository {
                path: _path,
                span: _span,
                source: _source,
            } => Some("no git repository found at this path"),
            Self::NegativeLimit(_) => Some("expected a non-negative integer"),
            Self::Git(GixError::NotFound {
                path: _path,
                source: _source,
            }) => Some("run this from inside a git repository"),
            Self::Git(GixError::Open {
                path: _path,
                source: _source,
            }) => Some("the repository was found but could not be opened; it may be corrupt"),
            Self::Git(GixError::RepositoryGone { git_dir: _git_dir }) => Some(
                "the repository is no longer at this location; run `gix open` again if it moved",
            ),
            Self::Git(GixError::Head(_)) => Some("the repository may have no commits yet"),
            Self::WrongInput(_) | Self::Git(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::git::discover;

    use rstest::rstest;

    use std::{io, path::Path};

    const MISSING_PATH: &str = "/nonexistent/gix-test";

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error("Test failure: {0}")]
        Failure(String),
    }

    /// The error `git::discover` gives for a path outside any repository.
    fn missing_repository() -> Result<GixError, TestError> {
        discover(Path::new(MISSING_PATH))
            .err()
            .ok_or_else(|| TestError::Failure("expected an error".to_owned()))
    }

    mod into_labeled {
        use super::*;

        #[rstest]
        fn points_wrong_input_at_the_input() {
            let input_span = Span::new(3, 9);
            let err = CommandError::WrongInput(WrongInput {
                expected: "git repository",
                hint: "pipe the output of `gix open` into this command",
                got: "int".to_owned(),
                span: input_span,
            });

            let labeled = err.into_labeled(Span::test_data());

            assert_eq!(labeled.msg, "expected a `git repository` value");
            assert_eq!(labeled.labels.first().map(|l| l.span), Some(input_span));
            assert_eq!(
                labeled.help.as_deref(),
                Some("pipe the output of `gix open` into this command")
            );
        }

        #[rstest]
        fn keeps_message_and_label() {
            let labeled = CommandError::NegativeLimit(-3).into_labeled(Span::test_data());

            assert_eq!(labeled.msg, "limit must not be negative, got -3");
            assert_eq!(labeled.labels.len(), 1);
        }

        #[rstest]
        fn points_at_the_flag_span_for_missing_repository() {
            let flag_span = Span::new(10, 20);
            let err = CommandError::NotARepository {
                path: PathBuf::from("/nope"),
                span: flag_span,
                source: GixError::Spawn(io::Error::other("x")),
            };

            let labeled = err.into_labeled(Span::test_data());

            assert_eq!(labeled.msg, "'/nope' is not inside a git repository");
            assert_eq!(labeled.labels.first().map(|l| l.span), Some(flag_span));
        }

        #[rstest]
        fn suggests_reopening_a_deleted_repository() {
            let err = CommandError::from(GixError::RepositoryGone {
                git_dir: PathBuf::from("/gone/.git"),
            });

            let labeled = err.into_labeled(Span::test_data());

            assert_eq!(
                labeled.msg,
                "git repository at '/gone/.git' was moved or deleted"
            );
            assert_eq!(labeled.labels.len(), 1);
        }

        #[rstest]
        fn labels_missing_repository_as_a_location_problem() -> Result<(), TestError> {
            let err = CommandError::from(missing_repository()?);

            let labeled = err.into_labeled(Span::test_data());

            assert_eq!(
                labeled.labels.first().map(|l| l.text.as_str()),
                Some("run this from inside a git repository")
            );

            Ok(())
        }

        #[rstest]
        fn labels_unopenable_repository_as_possibly_corrupt() -> Result<(), TestError> {
            let source = gix::open(MISSING_PATH)
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            let err = CommandError::from(GixError::Open {
                path: PathBuf::from(MISSING_PATH),
                source,
            });

            let labeled = err.into_labeled(Span::test_data());

            assert_eq!(
                labeled.labels.first().map(|l| l.text.as_str()),
                Some("the repository was found but could not be opened; it may be corrupt")
            );

            Ok(())
        }

        #[rstest]
        fn does_not_repeat_the_headline_in_help() -> Result<(), TestError> {
            let err = CommandError::NotARepository {
                path: PathBuf::from(MISSING_PATH),
                span: Span::test_data(),
                source: missing_repository()?,
            };

            let labeled = err.into_labeled(Span::test_data());

            assert!(
                labeled
                    .help
                    .as_deref()
                    .is_none_or(|help| !help.contains("no git repository found")),
                "got {:?}",
                labeled.help
            );

            Ok(())
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
