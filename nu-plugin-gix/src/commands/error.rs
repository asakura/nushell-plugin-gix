//! Command-level error that is converted to a Nushell `LabeledError` at the `run` boundary.
//!
//! Error model: every surfaced error carries a stable code (`CommandError::code`), listed per
//! command in its `extra_description`. Commands that ask a question (`gix ref list`) return an empty
//! table on "nothing found", never an error.
//!
//! Failures reported by the engine are coded too: `gix::current-dir-unavailable` when the working
//! directory cannot be read, `gix::help-unavailable` when the help text cannot be read,
//! `gix::invalid-argument` when an argument cannot be read, and `gix::invalid-input` when the
//! pipeline input cannot be read. The first two point at the call; the third points at the flag or
//! argument it was reading, or at the call when none is known; the fourth points at the call that
//! received the input.
//!
//! A git failure to resolve a revision points at the revision argument (`gix::bad-revision`), and
//! an out-of-range `--short-len` at its value (`gix::invalid-short-len`). A revision that names a
//! tree or blob is `gix::not-a-commit`. A reference update
//! rejected for its name points at the name (`gix::invalid-ref-name`), and one whose `--expect`
//! value no longer holds points at that value (`gix::ref-moved`). Such a failure is a
//! `CommandError::Located`, which pairs the git error with the span it is labelled at. A `--paths`
//! entry outside the repository points at that entry (`gix::path-outside-repository`), and an
//! invalid or unsupported `--paths` pathspec points at that entry (`gix::invalid-pathspec`).
//!
//! A handle whose repository is gone (`gix::repository-gone`) or can no longer be opened
//! (`gix::open-failed`) points at the value that carried the handle.
//!
//! A negative `--limit` or `--skip` points at its value.

use super::oid::{SHA1_HEX_LEN, SHA256_HEX_LEN};

use nu_plugin_gix_core::GixError;

use nu_protocol::{LabeledError, ShellError, Span};

use std::{error::Error as _, path::PathBuf};

const NOT_A_REPOSITORY_CODE: &str = "gix::not-a-repository";
const OPEN_FAILED_CODE: &str = "gix::open-failed";
const REPOSITORY_GONE_CODE: &str = "gix::repository-gone";
const CURRENT_DIR_CODE: &str = "gix::current-dir-unavailable";
const HELP_UNAVAILABLE_CODE: &str = "gix::help-unavailable";
const INVALID_ARGUMENT_CODE: &str = "gix::invalid-argument";
const INVALID_INPUT_CODE: &str = "gix::invalid-input";
const BAD_REVISION_CODE: &str = "gix::bad-revision";
const NOT_A_COMMIT_CODE: &str = "gix::not-a-commit";
const INVALID_REVISION_CODE: &str = "gix::invalid-revision";
const PATH_OUTSIDE_REPOSITORY_CODE: &str = "gix::path-outside-repository";
const INVALID_PATHSPEC_CODE: &str = "gix::invalid-pathspec";
const NOT_A_FULL_OBJECT_ID_CODE: &str = "gix::not-a-full-object-id";
const WALK_FAILED_CODE: &str = "gix::walk-failed";
const COMMIT_UNREADABLE_CODE: &str = "gix::commit-unreadable";
const BAD_TIMESTAMP_CODE: &str = "gix::bad-timestamp";
const IO_CODE: &str = "gix::io";
const WALKER_PANICKED_CODE: &str = "gix::walker-panicked";
const WRONG_INPUT_CODE: &str = "gix::wrong-input";
const NEGATIVE_LIMIT_CODE: &str = "gix::negative-limit";
const NEGATIVE_SKIP_CODE: &str = "gix::negative-skip";
const INVALID_SIDE_CODE: &str = "gix::invalid-side";
const REF_READ_FAILED_CODE: &str = "gix::ref-read-failed";
const REF_MOVED_CODE: &str = "gix::ref-moved";
const REF_UPDATE_FAILED_CODE: &str = "gix::ref-update-failed";
const INVALID_REF_NAME_CODE: &str = "gix::invalid-ref-name";
const INVALID_SHORT_LEN_CODE: &str = "gix::invalid-short-len";
const UNBORN_HEAD_CODE: &str = "gix::unborn-head";
const MISSING_EXPECT_CODE: &str = "gix::missing-expect";

/// Every stable code an error of this plugin can carry.
#[cfg(test)]
const ALL_CODES: &[&str] = &[
    NOT_A_REPOSITORY_CODE,
    OPEN_FAILED_CODE,
    REPOSITORY_GONE_CODE,
    CURRENT_DIR_CODE,
    HELP_UNAVAILABLE_CODE,
    INVALID_ARGUMENT_CODE,
    INVALID_INPUT_CODE,
    BAD_REVISION_CODE,
    NOT_A_COMMIT_CODE,
    INVALID_REVISION_CODE,
    PATH_OUTSIDE_REPOSITORY_CODE,
    INVALID_PATHSPEC_CODE,
    NOT_A_FULL_OBJECT_ID_CODE,
    WALK_FAILED_CODE,
    COMMIT_UNREADABLE_CODE,
    BAD_TIMESTAMP_CODE,
    IO_CODE,
    WALKER_PANICKED_CODE,
    WRONG_INPUT_CODE,
    NEGATIVE_LIMIT_CODE,
    NEGATIVE_SKIP_CODE,
    INVALID_SIDE_CODE,
    REF_READ_FAILED_CODE,
    REF_MOVED_CODE,
    REF_UPDATE_FAILED_CODE,
    INVALID_REF_NAME_CODE,
    INVALID_SHORT_LEN_CODE,
    UNBORN_HEAD_CODE,
    MISSING_EXPECT_CODE,
];

const INVALID_SIDE_LABEL: &str = "invalid side";
const UNRESOLVED_REVISION_LABEL: &str = "this revision does not resolve in the repository";
const NOT_A_COMMIT_LABEL: &str = "this revision names a tree or blob, not a commit";
const UNSHORTENABLE_REVISION_LABEL: &str = "the object this revision names could not be read";
const SHORT_LEN_OUT_OF_RANGE_LABEL: &str = "this short length is out of range";
const INVALID_REF_NAME_LABEL: &str = "this is not a valid full reference name";
const REF_MOVED_LABEL: &str = "the reference does not hold the value expected here";
const PATH_OUTSIDE_REPOSITORY_LABEL: &str =
    "this path is outside the repository; paths are relative to the current directory";
const PATHSPEC_ATTRIBUTES_LABEL: &str = "attribute pathspecs such as :(attr:...) are not supported";

/// Label for a repository that was found but cannot be opened.
const UNOPENABLE_LABEL: &str =
    "the repository was found but could not be opened; it may be corrupt";

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

    /// The git layer rejected an argument: a revision it could not resolve, shorten or that names
    /// no commit, a short length out of range, a reference name that is not valid, a reference
    /// that moved, a `--paths` entry outside the repository, or a `--paths` entry that is not a
    /// supported pathspec.
    #[error("{source}")]
    Located {
        /// Span of the argument the failure is attributed to, chosen by the command that raised it.
        span: Span,
        /// The underlying git-layer failure.
        #[source]
        source: GixError,
    },

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

    /// The repository named by `--repo` was found but cannot be opened.
    #[error("could not open git repository at '{}'", path.display())]
    OpenFailed {
        /// The path as the caller wrote it.
        path: PathBuf,
        /// Span of the `--repo` value.
        span: Span,
        /// The underlying git-layer failure.
        #[source]
        source: GixError,
    },

    /// The repository behind a pipeline handle was moved or deleted after the handle was made.
    #[error("the git repository at '{}' behind this handle was moved or deleted", git_dir.display())]
    HandleRepositoryGone {
        /// The git directory that is no longer there.
        git_dir: PathBuf,
        /// Span of the handle value that was piped in.
        span: Span,
        /// The underlying git-layer failure.
        #[source]
        source: GixError,
    },

    /// The repository behind a pipeline handle was found again but cannot be reopened.
    #[error("could not reopen the git repository at '{}' behind this handle", path.display())]
    HandleOpenFailed {
        /// The directory that could not be reopened.
        path: PathBuf,
        /// Span of the handle value that was piped in.
        span: Span,
        /// The underlying git-layer failure.
        #[source]
        source: GixError,
    },

    /// The engine could not report the current directory.
    #[error("could not read the current directory")]
    CurrentDir {
        /// The engine's failure.
        #[source]
        source: Box<ShellError>,
        /// Span of the command call.
        span: Span,
    },

    /// The engine could not report the help text of a command.
    #[error("could not read the command help")]
    HelpUnavailable {
        /// The engine's failure.
        #[source]
        source: Box<ShellError>,
        /// Span of the command call.
        span: Span,
    },

    /// An argument of the call could not be read.
    #[error("could not read the command arguments")]
    InvalidArgument {
        /// The failure reported while reading the argument.
        #[source]
        source: Box<ShellError>,
        /// Span of the argument, or of the call when it names no argument.
        span: Span,
    },

    /// The pipeline input could not be read.
    #[error("could not read the pipeline input")]
    InvalidInput {
        /// The failure reported while reading the input.
        #[source]
        source: Box<ShellError>,
        /// Span of the command call that received the input.
        span: Span,
    },

    /// The pipeline input is not the handle type the command works on.
    #[error("expected a `{}` value", .0.expected)]
    WrongInput(WrongInput),

    /// A negative value was given for the commit limit.
    #[error("limit must not be negative, got {limit}")]
    NegativeLimit {
        /// The negative limit as written.
        limit: i64,
        /// Span of the limit value.
        span: Span,
    },

    /// A negative value was given for the number of commits to skip.
    #[error("skip must not be negative, got {skip}")]
    NegativeSkip {
        /// The negative skip as written.
        skip: i64,
        /// Span of the skip value.
        span: Span,
    },

    /// A negative value was given for the short object id length.
    #[error("short length must not be negative, got {len}")]
    NegativeShortLen {
        /// The negative length as written.
        len: i64,
        /// Span of the short length value.
        span: Span,
    },

    /// A side must be a string or a record.
    #[error("a side must be a string or a record, got {got}")]
    InvalidSideType {
        /// Nushell type name that was received.
        got: String,
        /// Span of the invalid side.
        span: Span,
    },

    /// A revision expression cannot be blank.
    #[error("revision must not be empty")]
    EmptyRevision {
        /// Span of the blank revision.
        span: Span,
    },

    /// A value has the length of a SHA-1 or SHA-256 digest but is not hexadecimal.
    #[error("'{expression}' has the length of an object id but is not a valid one")]
    InvalidObjectId {
        /// The value as written.
        expression: String,
        /// Span of the invalid value.
        span: Span,
    },

    /// A value is not the full length of a SHA-1 or SHA-256 digest.
    #[error(
        "'{value}' is not a full object id (expected {} or {} hexadecimal characters)",
        SHA1_HEX_LEN,
        SHA256_HEX_LEN
    )]
    NotAFullObjectId {
        /// The value as written.
        value: String,
        /// Span of the invalid value.
        span: Span,
    },

    /// A mandatory expectation was not provided where one is required.
    #[error("gix ref set requires --expect")]
    MissingExpect {
        /// Span the error points at: where `--expect` was expected.
        span: Span,
    },

    /// A side record must contain only a `rev` field.
    #[error("a side record must contain only a `rev` field")]
    InvalidSideRecord {
        /// Span of the invalid record.
        span: Span,
    },

    /// A side record field had an invalid value.
    #[error("side field `{field}` must be a string, got {got}")]
    InvalidSideField {
        /// Field whose value was invalid.
        field: &'static str,
        /// Nushell type name that was received.
        got: String,
        /// Span of the invalid field value.
        span: Span,
    },
}

impl CommandError {
    /// Converts into a `LabeledError` pointing at `span`, appending the source chain as help text.
    #[must_use = "returns the converted error; the original is consumed"]
    #[expect(
        clippy::too_many_lines,
        reason = "the source table names every variant, so it is inherently wide"
    )]
    pub fn into_labeled(self, span: Span) -> LabeledError {
        let mut chain = Vec::new();

        let mut source = match &self {
            Self::NotARepository {
                path: _path,
                span: _span,
                source: found,
            }
            | Self::OpenFailed {
                path: _path,
                span: _span,
                source: found,
            } => found.source(),
            Self::HandleRepositoryGone {
                git_dir: _git_dir,
                span: _span,
                source: found,
            } => found.source(),
            Self::HandleOpenFailed {
                path: _path,
                span: _span,
                source: found,
            } => found.source(),
            Self::Git(_) | Self::WrongInput(_) => self.source(),
            Self::Located {
                span: _span,
                source: found,
            } => found.source(),
            Self::NegativeLimit {
                limit: _limit,
                span: _span,
            } => self.source(),
            Self::NegativeSkip {
                skip: _skip,
                span: _span,
            } => self.source(),
            Self::NegativeShortLen {
                len: _len,
                span: _span,
            } => self.source(),
            Self::InvalidSideType {
                got: _got,
                span: _span,
            } => self.source(),
            Self::InvalidObjectId {
                expression: _expression,
                span: _span,
            } => self.source(),
            Self::NotAFullObjectId {
                value: _value,
                span: _span,
            } => self.source(),
            Self::InvalidSideField {
                field: _field,
                got: _got,
                span: _span,
            } => self.source(),
            Self::EmptyRevision { span: _span }
            | Self::MissingExpect { span: _span }
            | Self::InvalidSideRecord { span: _span } => self.source(),
            Self::CurrentDir {
                source: _source,
                span: _span,
            }
            | Self::HelpUnavailable {
                source: _source,
                span: _span,
            }
            | Self::InvalidArgument {
                source: _source,
                span: _span,
            }
            | Self::InvalidInput {
                source: _source,
                span: _span,
            } => self.source(),
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

        let labeled = if let Some(help) = Self::argument_help(&self) {
            labeled.with_help(help)
        } else {
            labeled
        };

        let labeled = labeled.with_code(self.code());

        if chain.is_empty() {
            labeled
        } else {
            labeled.with_help(chain.join(": "))
        }
    }

    /// The stable code surfaced on the `LabeledError`.
    ///
    /// The match must cover every variant, so no error reaches the user without a code.
    const fn code(&self) -> &'static str {
        match self {
            Self::Git(git) => git_code(git),
            Self::Located {
                span: _span,
                source,
            } => git_code(source),
            Self::NotARepository {
                path: _path,
                span: _span,
                source: _source,
            } => NOT_A_REPOSITORY_CODE,
            Self::OpenFailed {
                path: _path,
                span: _span,
                source: _source,
            } => OPEN_FAILED_CODE,
            Self::HandleRepositoryGone {
                git_dir: _git_dir,
                span: _span,
                source: _source,
            } => REPOSITORY_GONE_CODE,
            Self::HandleOpenFailed {
                path: _path,
                span: _span,
                source: _source,
            } => OPEN_FAILED_CODE,
            Self::CurrentDir {
                source: _source,
                span: _span,
            } => CURRENT_DIR_CODE,
            Self::HelpUnavailable {
                source: _source,
                span: _span,
            } => HELP_UNAVAILABLE_CODE,
            Self::InvalidArgument {
                source: _source,
                span: _span,
            } => INVALID_ARGUMENT_CODE,
            Self::InvalidInput {
                source: _source,
                span: _span,
            } => INVALID_INPUT_CODE,
            Self::WrongInput(_) => WRONG_INPUT_CODE,
            Self::NegativeLimit {
                limit: _limit,
                span: _span,
            } => NEGATIVE_LIMIT_CODE,
            Self::NegativeSkip {
                skip: _skip,
                span: _span,
            } => NEGATIVE_SKIP_CODE,
            Self::NegativeShortLen {
                len: _len,
                span: _span,
            } => INVALID_SHORT_LEN_CODE,
            Self::MissingExpect { span: _span } => MISSING_EXPECT_CODE,
            Self::InvalidSideType {
                got: _got,
                span: _span,
            } => INVALID_SIDE_CODE,
            Self::InvalidSideRecord { span: _span } => INVALID_SIDE_CODE,
            Self::InvalidSideField {
                field: _field,
                got: _got,
                span: _span,
            } => INVALID_SIDE_CODE,
            Self::EmptyRevision { span: _span } => INVALID_REVISION_CODE,
            Self::InvalidObjectId {
                expression: _expression,
                span: _span,
            } => INVALID_REVISION_CODE,
            Self::NotAFullObjectId {
                value: _value,
                span: _span,
            } => NOT_A_FULL_OBJECT_ID_CODE,
        }
    }

    const fn own_span(&self) -> Option<Span> {
        match self {
            Self::NotARepository {
                span: flag_span,
                path: _path,
                source: _source,
            }
            | Self::OpenFailed {
                span: flag_span,
                path: _path,
                source: _source,
            } => Some(*flag_span),
            Self::HandleRepositoryGone {
                span,
                git_dir: _git_dir,
                source: _source,
            } => Some(*span),
            Self::HandleOpenFailed {
                span,
                path: _path,
                source: _source,
            } => Some(*span),
            Self::WrongInput(input) => Some(input.span),
            Self::EmptyRevision { span }
            | Self::MissingExpect { span }
            | Self::InvalidSideRecord { span } => Some(*span),
            Self::InvalidSideType { span, got: _got } => Some(*span),
            Self::InvalidObjectId {
                span,
                expression: _expression,
            } => Some(*span),
            Self::NotAFullObjectId {
                span,
                value: _value,
            } => Some(*span),
            Self::InvalidSideField {
                span,
                field: _field,
                got: _got,
            } => Some(*span),
            Self::NegativeLimit {
                span,
                limit: _limit,
            } => Some(*span),
            Self::NegativeSkip { span, skip: _skip } => Some(*span),
            Self::NegativeShortLen { span, len: _len } => Some(*span),
            Self::CurrentDir {
                span,
                source: _source,
            }
            | Self::HelpUnavailable {
                span,
                source: _source,
            }
            | Self::InvalidArgument {
                span,
                source: _source,
            }
            | Self::InvalidInput {
                span,
                source: _source,
            } => Some(*span),
            Self::Located {
                span,
                source: _source,
            } => Some(*span),
            Self::Git(_) => None,
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "the label table names every variant, so it is inherently wide"
    )]
    const fn label(&self) -> Option<&'static str> {
        match self {
            Self::NotARepository {
                path: _path,
                span: _span,
                source: _source,
            } => Some("no git repository found at this path"),
            Self::OpenFailed {
                path: _path,
                span: _span,
                source: _source,
            } => Some(UNOPENABLE_LABEL),
            Self::HandleRepositoryGone {
                git_dir: _git_dir,
                span: _span,
                source: _source,
            } => Some("this handle's repository is no longer at its location"),
            Self::HandleOpenFailed {
                path: _path,
                span: _span,
                source: _source,
            } => Some("this handle's repository can no longer be opened; it may be corrupt"),
            Self::CurrentDir {
                source: _source,
                span: _span,
            } => Some("the engine did not report a working directory"),
            Self::HelpUnavailable {
                source: _source,
                span: _span,
            } => Some("the engine did not report the help text"),
            Self::InvalidArgument {
                source: _source,
                span: _span,
            } => Some("this argument could not be read"),
            Self::InvalidInput {
                source: _source,
                span: _span,
            } => Some("this input could not be read"),
            Self::NegativeLimit {
                limit: _limit,
                span: _span,
            } => Some("expected a non-negative integer"),
            Self::NegativeSkip {
                skip: _skip,
                span: _span,
            } => Some("expected a non-negative integer"),
            Self::NegativeShortLen {
                len: _len,
                span: _span,
            } => Some("expected a non-negative integer"),
            Self::InvalidSideType {
                got: _got,
                span: _span,
            } => Some(INVALID_SIDE_LABEL),
            Self::InvalidSideRecord { span: _span } => Some(INVALID_SIDE_LABEL),
            Self::InvalidSideField {
                field: _field,
                got: _got,
                span: _span,
            } => Some(INVALID_SIDE_LABEL),
            Self::EmptyRevision { span: _span } => Some("the revision expression is empty"),
            Self::InvalidObjectId {
                expression: _expression,
                span: _span,
            } => Some("contains characters that are not hexadecimal digits"),
            Self::NotAFullObjectId {
                value: _value,
                span: _span,
            } => Some("a full object id has 40 characters for SHA-1 or 64 for SHA-256"),
            Self::MissingExpect { span: _span } => {
                Some("provide `--expect` with the value the ref must currently hold")
            }
            Self::Git(GixError::NotFound {
                path: _path,
                source: _source,
            }) => Some("run this from inside a git repository"),
            Self::Git(GixError::Open {
                path: _path,
                source: _source,
            }) => Some(UNOPENABLE_LABEL),
            Self::Git(GixError::RepositoryGone { git_dir: _git_dir }) => Some(
                "the repository is no longer at this location; run `gix open` again if it moved",
            ),
            Self::Git(GixError::Head(_)) => Some("HEAD could not be resolved to a commit"),
            Self::Located {
                span: _span,
                source,
            } => match source {
                GixError::Revision {
                    expression: _expression,
                    source: _source,
                } => Some(UNRESOLVED_REVISION_LABEL),
                GixError::NotACommit {
                    expression: _expression,
                    kind: _kind,
                } => Some(NOT_A_COMMIT_LABEL),
                GixError::Shorten {
                    expression: _expression,
                    source: _source,
                } => Some(UNSHORTENABLE_REVISION_LABEL),
                GixError::ShortLen {
                    len: _len,
                    detail: _detail,
                } => Some(SHORT_LEN_OUT_OF_RANGE_LABEL),
                GixError::InvalidRefName {
                    name: _name,
                    detail: _detail,
                } => Some(INVALID_REF_NAME_LABEL),
                GixError::RefMoved {
                    name: _name,
                    expected: _expected,
                    actual: _actual,
                } => Some(REF_MOVED_LABEL),
                GixError::PathOutsideRepository { path: _path } => {
                    Some(PATH_OUTSIDE_REPOSITORY_LABEL)
                }
                GixError::Pathspec {
                    spec: _spec,
                    source: _source,
                } => Some("this pathspec cannot be parsed"),
                GixError::PathspecAttributes { spec: _spec } => Some(PATHSPEC_ATTRIBUTES_LABEL),
                _ => None,
            },
            Self::WrongInput(_) | Self::Git(_) => None,
        }
    }

    /// The `help` text for argument errors (sides, revisions, object ids) and for handles whose
    /// repository is gone; `None` for variants without a hint.
    const fn argument_help(&self) -> Option<&'static str> {
        match self {
            Self::InvalidSideType {
                got: _got,
                span: _span,
            } => Some("use `head`, `index`, `worktree`, or a revision"),
            Self::EmptyRevision { span: _span } => Some("provide a non-empty revision expression"),
            Self::InvalidObjectId {
                expression: _expression,
                span: _span,
            } => Some("use only the digits 0-9 and the letters a-f or A-F"),
            Self::InvalidSideRecord { span: _span } => {
                Some("use a record with exactly one `rev` field")
            }
            Self::InvalidSideField {
                field: _field,
                got: _got,
                span: _span,
            } => Some("the `rev` field must contain a revision string"),
            Self::Git(_) | Self::WrongInput(_) => None,
            Self::Located {
                span: _span,
                source: _source,
            } => None,
            Self::HandleRepositoryGone {
                git_dir: _git_dir,
                span: _span,
                source: _source,
            } => Some("run `gix open` again to get a handle to the repository's current location"),
            Self::NotARepository {
                path: _path,
                span: _span,
                source: _source,
            }
            | Self::OpenFailed {
                path: _path,
                span: _span,
                source: _source,
            }
            | Self::HandleOpenFailed {
                path: _path,
                span: _span,
                source: _source,
            } => None,
            Self::NegativeLimit {
                limit: _limit,
                span: _span,
            } => None,
            Self::NegativeSkip {
                skip: _skip,
                span: _span,
            } => None,
            Self::NegativeShortLen {
                len: _len,
                span: _span,
            } => None,
            Self::CurrentDir {
                source: _source,
                span: _span,
            }
            | Self::HelpUnavailable {
                source: _source,
                span: _span,
            }
            | Self::InvalidArgument {
                source: _source,
                span: _span,
            }
            | Self::InvalidInput {
                source: _source,
                span: _span,
            } => None,
            Self::NotAFullObjectId {
                value: _value,
                span: _span,
            } => None,
            Self::MissingExpect { span: _span } => None,
        }
    }
}

/// The stable code for a git-layer error.
const fn git_code(error: &GixError) -> &'static str {
    match error {
        GixError::NotFound {
            path: _path,
            source: _source,
        } => NOT_A_REPOSITORY_CODE,
        GixError::Open {
            path: _path,
            source: _source,
        } => OPEN_FAILED_CODE,
        GixError::RepositoryGone { git_dir: _git_dir } => REPOSITORY_GONE_CODE,
        GixError::Head(_) => BAD_REVISION_CODE,
        GixError::Revision {
            expression: _expression,
            source: _source,
        } => BAD_REVISION_CODE,
        GixError::Shorten {
            expression: _expression,
            source: _source,
        } => BAD_REVISION_CODE,
        GixError::NotACommit {
            expression: _expression,
            kind: _kind,
        } => NOT_A_COMMIT_CODE,
        GixError::PathOutsideRepository { path: _path } => PATH_OUTSIDE_REPOSITORY_CODE,
        GixError::Pathspec {
            spec: _spec,
            source: _source,
        } => INVALID_PATHSPEC_CODE,
        GixError::PathspecAttributes { spec: _spec } => INVALID_PATHSPEC_CODE,
        GixError::Walk(_) => WALK_FAILED_CODE,
        GixError::Commit {
            id: _id,
            source: _source,
        } => COMMIT_UNREADABLE_CODE,
        GixError::Timestamp { id: _id } => BAD_TIMESTAMP_CODE,
        GixError::Spawn(_) => IO_CODE,
        GixError::WalkerPanicked => WALKER_PANICKED_CODE,
        GixError::UnbornHead { name: _name } => UNBORN_HEAD_CODE,
        GixError::RefRead(_) => REF_READ_FAILED_CODE,
        // `DanglingRef` is a failure to read HEAD's resolution, so it shares the ref-read code.
        GixError::DanglingRef { name: _name } => REF_READ_FAILED_CODE,
        GixError::RefMoved {
            name: _name,
            expected: _expected,
            actual: _actual,
        } => REF_MOVED_CODE,
        GixError::RefUpdate {
            name: _name,
            source: _source,
        } => REF_UPDATE_FAILED_CODE,
        GixError::InvalidOid { value: _value } => INVALID_REVISION_CODE,
        GixError::InvalidRefName {
            name: _name,
            detail: _detail,
        } => INVALID_REF_NAME_CODE,
        GixError::ShortLen {
            len: _len,
            detail: _detail,
        } => INVALID_SHORT_LEN_CODE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::cdt::{Handle, Repository};

    use nu_plugin_gix_core::{RefExpect, RefTarget, RepositoryLocation};
    use nu_protocol::ShellError;
    use rstest::rstest;

    use std::{io, path::Path};

    const MISSING_PATH: &str = "/nonexistent/gix-test";
    const REF_NAME: &str = "refs/heads/main";
    const SHORTHAND_NAME: &str = "main";

    /// The object id the reference actually held after a concurrent move.
    const ACTUAL_OID: &str = "fedcba9876543210fedcba9876543210fedcba98";

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error("Test failure: {0}")]
        Failure(String),
    }

    /// The error `RepositoryLocation::discover` gives for a path outside any repository.
    fn missing_repository() -> Result<GixError, TestError> {
        RepositoryLocation::discover(Path::new(MISSING_PATH))
            .err()
            .ok_or_else(|| TestError::Failure("expected an error".into()))
    }

    /// A stand-in `gix::Error` to use as the `source` of a git failure.
    fn gix_error() -> gix::Error {
        gix::Error::from_error(io::Error::other("simulated git failure"))
    }

    /// The failure the engine reports when it cannot answer a request.
    fn engine_failure() -> ShellError {
        ShellError::NushellFailed {
            msg: "engine unavailable".to_owned(),
        }
    }

    /// Unit tests for the `into_labeled` mapping an evaluated script cannot observe:
    /// the exact span each error points at and the label/help mapping of variants the
    /// command surface cannot raise.
    ///
    /// `WrongInput` is here because the engine rejects non-handle values before they
    /// reach a command, and no command produces a second handle type to pipe in yet.
    mod into_labeled {
        use super::*;

        #[rstest]
        fn points_wrong_input_at_the_input() {
            let input_span = Span::new(3, 9);
            let err = CommandError::WrongInput(WrongInput {
                expected: <Repository as Handle>::TYPE_NAME,
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
        fn points_an_invalid_side_type_at_the_value() {
            let input_span = Span::new(3, 9);
            let labeled = CommandError::InvalidSideType {
                got: "int".to_owned(),
                span: input_span,
            }
            .into_labeled(Span::test_data());

            assert_eq!(
                labeled.labels.first().map(|label| label.span),
                Some(input_span)
            );
            assert_eq!(
                labeled.help.as_deref(),
                Some("use `head`, `index`, `worktree`, or a revision")
            );
        }

        #[rstest]
        fn points_an_empty_revision_at_the_revision() {
            let revision_span = Span::new(3, 9);
            let labeled = CommandError::EmptyRevision {
                span: revision_span,
            }
            .into_labeled(Span::test_data());

            assert_eq!(
                labeled.labels.first().map(|label| label.span),
                Some(revision_span)
            );
            assert_eq!(
                labeled.help.as_deref(),
                Some("provide a non-empty revision expression")
            );
        }

        #[rstest]
        fn labels_an_empty_revision_with_its_own_text() {
            let labeled = CommandError::EmptyRevision {
                span: Span::test_data(),
            }
            .into_labeled(Span::test_data());

            assert_eq!(
                labeled.labels.first().map(|label| label.text.as_str()),
                Some("the revision expression is empty")
            );
        }

        #[rstest]
        fn labels_an_invalid_object_id_with_its_own_text() {
            let labeled = CommandError::InvalidObjectId {
                expression: "0123456789abcdef0123456789abcdef0123456z".to_owned(),
                span: Span::test_data(),
            }
            .into_labeled(Span::test_data());

            assert_eq!(
                labeled.labels.first().map(|label| label.text.as_str()),
                Some("contains characters that are not hexadecimal digits")
            );
        }

        #[rstest]
        fn helps_an_invalid_object_id_with_both_hex_cases() {
            let labeled = CommandError::InvalidObjectId {
                expression: "0123456789abcdef0123456789abcdef0123456z".to_owned(),
                span: Span::test_data(),
            }
            .into_labeled(Span::test_data());

            assert_eq!(
                labeled.help.as_deref(),
                Some("use only the digits 0-9 and the letters a-f or A-F")
            );
        }

        #[rstest]
        fn points_a_non_full_object_id_at_the_value() {
            let value_span = Span::new(3, 9);
            let labeled = CommandError::NotAFullObjectId {
                value: "0123456".to_owned(),
                span: value_span,
            }
            .into_labeled(Span::test_data());

            assert_eq!(
                labeled.labels.first().map(|label| label.span),
                Some(value_span)
            );
        }

        #[rstest]
        fn points_an_invalid_side_record_at_the_record() {
            let record_span = Span::new(3, 9);
            let labeled = CommandError::InvalidSideRecord { span: record_span }
                .into_labeled(Span::test_data());

            assert_eq!(
                labeled.labels.first().map(|label| label.span),
                Some(record_span)
            );
            assert_eq!(
                labeled.help.as_deref(),
                Some("use a record with exactly one `rev` field")
            );
        }

        #[rstest]
        fn points_an_invalid_side_field_at_the_field_value() {
            let field_span = Span::new(3, 9);
            let labeled = CommandError::InvalidSideField {
                field: "rev",
                got: "int".to_owned(),
                span: field_span,
            }
            .into_labeled(Span::test_data());

            assert_eq!(
                labeled.labels.first().map(|label| label.span),
                Some(field_span)
            );
            assert_eq!(
                labeled.help.as_deref(),
                Some("the `rev` field must contain a revision string")
            );
        }

        #[rstest]
        fn reports_a_deleted_repository_with_its_message() {
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
        fn points_a_handle_whose_repository_is_gone_at_the_handle() {
            let handle_span = Span::new(3, 9);

            let labeled = CommandError::HandleRepositoryGone {
                git_dir: PathBuf::from("/gone/.git"),
                span: handle_span,
                source: GixError::RepositoryGone {
                    git_dir: PathBuf::from("/gone/.git"),
                },
            }
            .into_labeled(Span::test_data());

            assert_eq!(labeled.labels.first().map(|l| l.span), Some(handle_span));

            assert_eq!(
                labeled.msg,
                "the git repository at '/gone/.git' behind this handle was moved or deleted"
            );

            assert_eq!(
                labeled.labels.first().map(|l| l.text.as_str()),
                Some("this handle's repository is no longer at its location")
            );

            assert_eq!(
                labeled.help.as_deref(),
                Some("run `gix open` again to get a handle to the repository's current location")
            );
        }

        #[rstest]
        fn points_a_handle_that_cannot_be_reopened_at_the_handle() {
            let handle_span = Span::new(3, 9);

            let labeled = CommandError::HandleOpenFailed {
                path: PathBuf::from(MISSING_PATH),
                span: handle_span,
                source: GixError::Open {
                    path: PathBuf::from(MISSING_PATH),
                    source: gix_error(),
                },
            }
            .into_labeled(Span::test_data());

            assert_eq!(labeled.labels.first().map(|l| l.span), Some(handle_span));

            assert_eq!(
                labeled.msg,
                format!(
                    "could not reopen the git repository at '{MISSING_PATH}' behind this handle"
                )
            );

            assert_eq!(
                labeled.labels.first().map(|l| l.text.as_str()),
                Some("this handle's repository can no longer be opened; it may be corrupt")
            );
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
        fn points_an_open_failure_at_the_flag() {
            let flag_span = Span::new(3, 9);

            let err = CommandError::OpenFailed {
                path: PathBuf::from(MISSING_PATH),
                span: flag_span,
                source: GixError::Open {
                    path: PathBuf::from(MISSING_PATH),
                    source: gix_error(),
                },
            };

            let labeled = err.into_labeled(Span::test_data());

            assert_eq!(labeled.labels.first().map(|l| l.span), Some(flag_span));
            assert_eq!(labeled.code.as_deref(), Some(OPEN_FAILED_CODE));
        }

        #[rstest]
        fn points_a_non_repository_at_the_flag_with_its_code_and_label() -> Result<(), TestError> {
            let flag_span = Span::new(3, 9);
            let labeled = CommandError::NotARepository {
                path: PathBuf::from(MISSING_PATH),
                span: flag_span,
                source: missing_repository()?,
            }
            .into_labeled(Span::test_data());

            assert_eq!(labeled.labels.first().map(|l| l.span), Some(flag_span));
            assert_eq!(
                labeled.labels.first().map(|l| l.text.as_str()),
                Some("no git repository found at this path")
            );
            assert_eq!(labeled.code.as_deref(), Some(NOT_A_REPOSITORY_CODE));

            Ok(())
        }

        #[rstest]
        fn points_a_located_failure_at_its_span() {
            let revision_span = Span::new(3, 9);
            let labeled = CommandError::Located {
                span: revision_span,
                source: GixError::Revision {
                    expression: "nope".to_owned(),
                    source: gix_error(),
                },
            }
            .into_labeled(Span::test_data());

            assert_eq!(labeled.labels.first().map(|l| l.span), Some(revision_span));
        }

        #[rstest]
        fn labels_an_unresolved_revision_with_its_reason() {
            let labeled = CommandError::Located {
                span: Span::test_data(),
                source: GixError::Revision {
                    expression: "nope".to_owned(),
                    source: gix_error(),
                },
            }
            .into_labeled(Span::test_data());

            assert_eq!(
                labeled.labels.first().map(|l| l.text.as_str()),
                Some(UNRESOLVED_REVISION_LABEL)
            );
        }

        #[rstest]
        fn labels_a_non_commit_revision_with_its_reason() {
            let labeled = CommandError::Located {
                span: Span::test_data(),
                source: GixError::NotACommit {
                    expression: "HEAD^{tree}".to_owned(),
                    kind: "tree".to_owned(),
                },
            }
            .into_labeled(Span::test_data());

            assert_eq!(
                labeled.labels.first().map(|l| l.text.as_str()),
                Some(NOT_A_COMMIT_LABEL)
            );
        }

        #[rstest]
        fn labels_a_path_outside_the_repository_with_its_reason() {
            let labeled = CommandError::Located {
                span: Span::test_data(),
                source: GixError::PathOutsideRepository {
                    path: "../outside".to_owned(),
                },
            }
            .into_labeled(Span::test_data());

            assert_eq!(
                labeled.labels.first().map(|l| l.text.as_str()),
                Some(PATH_OUTSIDE_REPOSITORY_LABEL)
            );
        }

        #[rstest]
        fn labels_an_out_of_range_short_length_with_its_reason() {
            let labeled = CommandError::Located {
                span: Span::test_data(),
                source: GixError::ShortLen {
                    len: 3,
                    detail: "expected 4..=40".to_owned(),
                },
            }
            .into_labeled(Span::test_data());

            assert_eq!(
                labeled.labels.first().map(|l| l.text.as_str()),
                Some(SHORT_LEN_OUT_OF_RANGE_LABEL)
            );
        }

        #[rstest]
        #[case::invalid_name(GixError::InvalidRefName {
            name: SHORTHAND_NAME.to_owned(),
            detail: "expected a full name".to_owned(),
        })]
        #[case::moved(GixError::RefMoved {
            name: REF_NAME.to_owned(),
            expected: RefExpect::Missing,
            actual: Some(RefTarget::Object(ACTUAL_OID.to_owned())),
        })]
        fn points_a_ref_update_failure_at_its_span(#[case] source: GixError) {
            let span = Span::new(3, 9);
            let labeled = CommandError::Located { span, source }.into_labeled(Span::test_data());

            assert_eq!(labeled.labels.first().map(|l| l.span), Some(span));
        }

        #[rstest]
        fn labels_an_invalid_ref_name_with_its_reason() {
            let labeled = CommandError::Located {
                span: Span::test_data(),
                source: GixError::InvalidRefName {
                    name: SHORTHAND_NAME.to_owned(),
                    detail: "expected a full name".to_owned(),
                },
            }
            .into_labeled(Span::test_data());

            assert_eq!(
                labeled.labels.first().map(|l| l.text.as_str()),
                Some(INVALID_REF_NAME_LABEL)
            );
        }

        #[rstest]
        fn labels_a_moved_reference_with_its_reason() {
            let labeled = CommandError::Located {
                span: Span::test_data(),
                source: GixError::RefMoved {
                    name: REF_NAME.to_owned(),
                    expected: RefExpect::Missing,
                    actual: Some(RefTarget::Object(ACTUAL_OID.to_owned())),
                },
            }
            .into_labeled(Span::test_data());

            assert_eq!(
                labeled.labels.first().map(|l| l.text.as_str()),
                Some(REF_MOVED_LABEL)
            );
        }

        #[rstest]
        fn keeps_the_located_headline_out_of_the_help() {
            let labeled = CommandError::Located {
                span: Span::test_data(),
                source: GixError::Revision {
                    expression: "nope".to_owned(),
                    source: gix_error(),
                },
            }
            .into_labeled(Span::test_data());

            assert_eq!(labeled.msg, "could not resolve revision 'nope'");
            assert!(
                labeled
                    .help
                    .as_deref()
                    .is_none_or(|help| !help.contains("could not resolve revision")),
                "got {:?}",
                labeled.help
            );
        }

        #[rstest]
        fn points_a_negative_limit_at_its_span() {
            let limit_span = Span::new(3, 9);
            let labeled = CommandError::NegativeLimit {
                limit: -1,
                span: limit_span,
            }
            .into_labeled(Span::test_data());

            assert_eq!(
                labeled.labels.first().map(|label| label.span),
                Some(limit_span)
            );
        }

        #[rstest]
        fn labels_a_negative_limit_with_its_code_and_reason() {
            const NEGATIVE_LIMIT_REASON: &str = "expected a non-negative integer";

            let labeled = CommandError::NegativeLimit {
                limit: -1,
                span: Span::test_data(),
            }
            .into_labeled(Span::test_data());

            assert_eq!(labeled.code.as_deref(), Some(NEGATIVE_LIMIT_CODE));
            assert_eq!(
                labeled.labels.first().map(|label| label.text.as_str()),
                Some(NEGATIVE_LIMIT_REASON)
            );
        }

        #[rstest]
        fn points_an_argument_error_at_its_span() {
            let argument_span = Span::new(3, 9);
            let labeled = CommandError::InvalidArgument {
                source: Box::new(engine_failure()),
                span: argument_span,
            }
            .into_labeled(Span::test_data());

            assert_eq!(
                labeled.labels.first().map(|label| label.span),
                Some(argument_span)
            );
        }

        #[rstest]
        fn has_no_label_for_a_timestamp_error() {
            let err = CommandError::Git(GixError::Timestamp {
                id: "abc".to_owned(),
            });

            let labeled = err.into_labeled(Span::test_data());

            assert!(labeled.labels.is_empty());
        }

        const ARGUMENT_SPAN: Span = Span { start: 3, end: 9 };

        #[rstest]
        #[case::not_a_full_object_id(
            CommandError::NotAFullObjectId {
                value: "0123456".to_owned(),
                span: ARGUMENT_SPAN,
            },
            NOT_A_FULL_OBJECT_ID_CODE
        )]
        #[case::invalid_object_id(
            CommandError::InvalidObjectId {
                expression: "0123456789abcdef0123456789abcdef0123456z".to_owned(),
                span: ARGUMENT_SPAN,
            },
            INVALID_REVISION_CODE
        )]
        #[case::empty_revision(
            CommandError::EmptyRevision {
                span: ARGUMENT_SPAN,
            },
            INVALID_REVISION_CODE
        )]
        #[case::missing_expect(
            CommandError::MissingExpect {
                span: ARGUMENT_SPAN,
            },
            MISSING_EXPECT_CODE
        )]
        #[case::negative_short_len(
            CommandError::NegativeShortLen {
                len: -1,
                span: ARGUMENT_SPAN,
            },
            INVALID_SHORT_LEN_CODE
        )]
        #[case::negative_skip(
            CommandError::NegativeSkip {
                skip: -1,
                span: ARGUMENT_SPAN,
            },
            NEGATIVE_SKIP_CODE
        )]
        fn carries_the_code_and_the_span_of_each_argument_error(
            #[case] error: CommandError,
            #[case] code: &str,
        ) {
            let labeled = error.into_labeled(Span::test_data());

            assert_eq!(labeled.code.as_deref(), Some(code));
            assert_eq!(
                labeled.labels.first().map(|label| label.span),
                Some(ARGUMENT_SPAN)
            );
        }
    }

    /// Unit tests for the variant-to-code mapping of errors that no script-level test asserts.
    mod codes {
        use super::*;

        // No command parses sides yet, so these variants cannot be triggered by a script.
        #[rstest]
        #[case::invalid_side_type(
            CommandError::InvalidSideType {
                got: "int".to_owned(),
                span: Span::test_data(),
            },
            INVALID_SIDE_CODE
        )]
        #[case::invalid_side_record(
            CommandError::InvalidSideRecord {
                span: Span::test_data(),
            },
            INVALID_SIDE_CODE
        )]
        #[case::invalid_side_field(
            CommandError::InvalidSideField {
                field: "rev",
                got: "int".to_owned(),
                span: Span::test_data(),
            },
            INVALID_SIDE_CODE
        )]
        fn side_errors_carry_stable_codes(#[case] error: CommandError, #[case] code: &str) {
            let labeled = error.into_labeled(Span::test_data());

            assert_eq!(labeled.code.as_deref(), Some(code));
        }

        // The engine cannot be made to fail from a script, so these variants are checked directly.
        #[rstest]
        #[case::current_dir(
            CommandError::CurrentDir {
                source: Box::new(engine_failure()),
                span: Span::test_data(),
            },
            CURRENT_DIR_CODE
        )]
        #[case::help_unavailable(
            CommandError::HelpUnavailable {
                source: Box::new(engine_failure()),
                span: Span::test_data(),
            },
            HELP_UNAVAILABLE_CODE
        )]
        fn engine_errors_carry_stable_codes(#[case] error: CommandError, #[case] code: &str) {
            let labeled = error.into_labeled(Span::test_data());

            assert_eq!(labeled.code.as_deref(), Some(code));
        }

        #[rstest]
        #[case::open(
            CommandError::from(GixError::Open {
                path: PathBuf::from(MISSING_PATH),
                source: gix_error(),
            }),
            OPEN_FAILED_CODE
        )]
        #[case::revision(
            CommandError::from(GixError::Revision {
                expression: "nope".to_owned(),
                source: gix_error(),
            }),
            BAD_REVISION_CODE
        )]
        #[case::shorten(
            CommandError::from(GixError::Shorten {
                expression: "nope".to_owned(),
                source: gix_error(),
            }),
            BAD_REVISION_CODE
        )]
        #[case::head(CommandError::from(GixError::Head(gix_error())), BAD_REVISION_CODE)]
        #[case::walk(CommandError::from(GixError::Walk(gix_error())), WALK_FAILED_CODE)]
        #[case::commit(
            CommandError::from(GixError::Commit {
                id: "abc".to_owned(),
                source: gix_error(),
            }),
            COMMIT_UNREADABLE_CODE
        )]
        #[case::repository_gone(
            CommandError::from(GixError::RepositoryGone {
                git_dir: PathBuf::from("/gone/.git"),
            }),
            REPOSITORY_GONE_CODE
        )]
        #[case::spawn(
            CommandError::from(GixError::Spawn(io::Error::other("no thread"))),
            IO_CODE
        )]
        #[case::walker_panicked(CommandError::from(GixError::WalkerPanicked), WALKER_PANICKED_CODE)]
        #[case::ref_read(
            CommandError::from(GixError::RefRead(gix_error())),
            REF_READ_FAILED_CODE
        )]
        #[case::dangling_ref(
            CommandError::from(GixError::DanglingRef {
                name: "refs/heads/main".to_owned(),
            }),
            REF_READ_FAILED_CODE
        )]
        #[case::ref_moved(
            CommandError::from(GixError::RefMoved {
                name: "refs/heads/main".to_owned(),
                expected: RefExpect::Missing,
                actual: Some(RefTarget::Object(ACTUAL_OID.to_owned())),
            }),
            REF_MOVED_CODE
        )]
        #[case::ref_update(
            CommandError::from(GixError::RefUpdate {
                name: "refs/heads/main".to_owned(),
                source: gix_error(),
            }),
            REF_UPDATE_FAILED_CODE
        )]
        #[case::invalid_ref_name(
            CommandError::from(GixError::InvalidRefName {
                name: "main".to_owned(),
                detail: "missing the `refs/` prefix".to_owned(),
            }),
            INVALID_REF_NAME_CODE
        )]
        #[case::pathspec(
            CommandError::from(GixError::Pathspec {
                spec: ":(nonsense)x".to_owned(),
                source: gix_error(),
            }),
            INVALID_PATHSPEC_CODE
        )]
        #[case::pathspec_attributes(
            CommandError::from(GixError::PathspecAttributes {
                spec: ":(attr:linguist-vendored)x".to_owned(),
            }),
            INVALID_PATHSPEC_CODE
        )]
        #[case::short_len(
            CommandError::from(GixError::ShortLen {
                len: 3,
                detail: "below the minimum of 4".to_owned(),
            }),
            INVALID_SHORT_LEN_CODE
        )]
        #[case::not_a_commit(
            CommandError::from(GixError::NotACommit {
                expression: "HEAD^{tree}".to_owned(),
                kind: "tree".to_owned(),
            }),
            NOT_A_COMMIT_CODE
        )]
        #[case::path_outside_repository(
            CommandError::from(GixError::PathOutsideRepository {
                path: "../outside".to_owned(),
            }),
            PATH_OUTSIDE_REPOSITORY_CODE
        )]
        fn git_errors_carry_stable_codes(#[case] error: CommandError, #[case] code: &str) {
            let labeled = error.into_labeled(Span::test_data());

            assert_eq!(labeled.code.as_deref(), Some(code));
        }

        #[rstest]
        fn located_shorten_failures_carry_the_git_code() {
            let labeled = CommandError::Located {
                span: Span::test_data(),
                source: GixError::Shorten {
                    expression: "nope".to_owned(),
                    source: gix_error(),
                },
            }
            .into_labeled(Span::test_data());

            assert_eq!(labeled.code.as_deref(), Some(BAD_REVISION_CODE));
        }

        #[rstest]
        fn unopenable_handles_carry_the_open_failed_code() {
            let handle_span = Span::test_data();
            let unopenable = CommandError::HandleOpenFailed {
                path: PathBuf::from(MISSING_PATH),
                span: handle_span,
                source: GixError::Open {
                    path: PathBuf::from(MISSING_PATH),
                    source: gix_error(),
                },
            };

            assert_eq!(
                unopenable.into_labeled(handle_span).code.as_deref(),
                Some(OPEN_FAILED_CODE)
            );
        }
    }

    mod documentation {
        use super::*;

        use crate::GixPlugin;

        use nu_plugin::Plugin as _;

        const CODES_LINE_PREFIX: &str = "Error codes: ";

        /// The codes a command's `extra_description` lists after `Error codes: `.
        fn listed_codes<'a>(
            name: &str,
            extra_description: &'a str,
        ) -> Result<Vec<&'a str>, TestError> {
            extra_description
                .lines()
                .find_map(|line| line.strip_prefix(CODES_LINE_PREFIX))
                .ok_or_else(|| TestError::Failure(format!("`{name}` lists no `Error codes:` line")))
                .map(|codes| codes.split(", ").collect())
        }

        #[rstest]
        fn every_documented_code_is_known() -> Result<(), TestError> {
            for command in GixPlugin::default().commands() {
                for code in listed_codes(command.name(), command.extra_description())? {
                    assert!(
                        ALL_CODES.contains(&code),
                        "`{code}` is documented but not in ALL_CODES"
                    );
                }
            }

            Ok(())
        }
    }
}
