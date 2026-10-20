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
//! An invalid `gix config get` key points at the key (`gix::invalid-config-key`); a value that is
//! not UTF-8 points at the key too (`gix::config-value-not-utf8`). A value that cannot be read as
//! the `--type` given points at the key (`gix::invalid-config-value`), a path value that is not
//! UTF-8 points at the key (`gix::config-path-not-utf8`), and an unknown `--type` value points at
//! the value (`gix::unknown-config-type`). `gix config list` takes no key, so every failure to list
//! an entry points at the handle it was given: a value or path that is not UTF-8 uses the codes
//! above, a subsection name that is not UTF-8 is `gix::config-subsection-not-utf8`, and a section
//! that cannot be read back as its names and values is `gix::config-section-unreadable`.
//!
//! A list key whose values an empty value (`name =`) all removed points at the key
//! (`gix::config-key-reset`) when `gix config get` asks for it; `gix config list` leaves such keys out.
//!
//! A valueless key read as an `int`, `path` or `string` points at the key (`gix::config-value-missing`),
//! and a `%(prefix)/` path whose install directory is unknown points at the key too
//! (`gix::install-dir-unavailable`). A variable and a subsection with one name in one section is
//! `gix::config-name-clash`, and a section whose parse events break the grammar of a variable is
//! `gix::config-section-malformed`; both point at the key for `gix config get` and at the handle for
//! `gix config list`.
//!
//! The messages of the configuration errors name where the value came from: its file, as in
//! `(in /home/u/.gitconfig)`, or else its source kind, as in `(from the environment)`. The codes and
//! labels do not change.
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
const INVALID_CONFIG_KEY_CODE: &str = "gix::invalid-config-key";
const CONFIG_VALUE_NOT_UTF8_CODE: &str = "gix::config-value-not-utf8";
const CONFIG_VALUE_CONVERSION_CODE: &str = "gix::invalid-config-value";
const CONFIG_PATH_NOT_UTF8_CODE: &str = "gix::config-path-not-utf8";
const UNKNOWN_CONFIG_TYPE_CODE: &str = "gix::unknown-config-type";
const CONFIG_SUBSECTION_NOT_UTF8_CODE: &str = "gix::config-subsection-not-utf8";
const CONFIG_SECTION_UNREADABLE_CODE: &str = "gix::config-section-unreadable";
const CONFIG_KEY_RESET_CODE: &str = "gix::config-key-reset";
const CONFIG_VALUE_MISSING_CODE: &str = "gix::config-value-missing";
const INSTALL_DIR_UNAVAILABLE_CODE: &str = "gix::install-dir-unavailable";
const CONFIG_NAME_CLASH_CODE: &str = "gix::config-name-clash";
const CONFIG_SECTION_MALFORMED_CODE: &str = "gix::config-section-malformed";

/// Start of the help line that lists a command's error codes.
const CODES_LINE_PREFIX: &str = "Error codes: ";
/// Separator of the codes in that line.
const CODES_SEPARATOR: &str = ", ";

/// Codes `gix config list` can raise; `gix config get` can raise them too.
pub(super) const CONFIG_LIST_CODES: &[&str] = &[
    WRONG_INPUT_CODE,
    CONFIG_VALUE_CONVERSION_CODE,
    CONFIG_VALUE_NOT_UTF8_CODE,
    CONFIG_SUBSECTION_NOT_UTF8_CODE,
    CONFIG_SECTION_UNREADABLE_CODE,
    CONFIG_SECTION_MALFORMED_CODE,
    CONFIG_NAME_CLASH_CODE,
    OPEN_FAILED_CODE,
    REPOSITORY_GONE_CODE,
];
/// Codes only `gix config get` can raise, on top of [`CONFIG_LIST_CODES`].
pub(super) const CONFIG_GET_EXTRA_CODES: &[&str] = &[
    INVALID_ARGUMENT_CODE,
    INVALID_CONFIG_KEY_CODE,
    UNKNOWN_CONFIG_TYPE_CODE,
    CONFIG_VALUE_MISSING_CODE,
    CONFIG_PATH_NOT_UTF8_CODE,
    INSTALL_DIR_UNAVAILABLE_CODE,
    CONFIG_KEY_RESET_CODE,
];

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
    INVALID_CONFIG_KEY_CODE,
    CONFIG_VALUE_NOT_UTF8_CODE,
    CONFIG_VALUE_CONVERSION_CODE,
    CONFIG_PATH_NOT_UTF8_CODE,
    UNKNOWN_CONFIG_TYPE_CODE,
    CONFIG_SUBSECTION_NOT_UTF8_CODE,
    CONFIG_SECTION_UNREADABLE_CODE,
    CONFIG_KEY_RESET_CODE,
    CONFIG_VALUE_MISSING_CODE,
    INSTALL_DIR_UNAVAILABLE_CODE,
    CONFIG_NAME_CLASH_CODE,
    CONFIG_SECTION_MALFORMED_CODE,
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
const INVALID_CONFIG_KEY_LABEL: &str =
    "expected a key like `section.name` or `section.subsection.name`";
const CONFIG_VALUE_NOT_UTF8_LABEL: &str = "the value of this key is not valid UTF-8";
const CONFIG_VALUE_CONVERSION_LABEL: &str = "the value of this key cannot be read as this type";
const CONFIG_PATH_NOT_UTF8_LABEL: &str = "the path in the value of this key is not valid UTF-8";
const UNKNOWN_CONFIG_TYPE_LABEL: &str = "expected `bool`, `int`, `path` or `string`";
const CONFIG_SUBSECTION_NOT_UTF8_LABEL: &str =
    "the subsection name of this section is not valid UTF-8";
const CONFIG_SECTION_UNREADABLE_LABEL: &str =
    "the variables of this section could not be read back as names and values";
const CONFIG_SECTION_MALFORMED_LABEL: &str =
    "the variables of this section could not be read: its parse events are out of order";
const CONFIG_KEY_RESET_LABEL: &str = "an empty value reset this key, so it has no value";
const CONFIG_KEY_RESET_HELP: &str =
    "`name =` clears a list in git; set the key again to give it a value";
const CONFIG_VALUE_MISSING_LABEL: &str = "this key has no value, so it cannot be read as this type";
const INSTALL_DIR_UNAVAILABLE_LABEL: &str =
    "this path uses %(prefix)/, which needs the install directory of git";
const CONFIG_NAME_CLASH_LABEL: &str = "this section has a variable and a subsection with this name";

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
///
/// The four variants that carry a path box their source so the error stays small; build them with the constructors.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    /// The git layer failed.
    #[error(transparent)]
    Git(#[from] GixError),

    /// The git layer rejected an argument: a revision it could not resolve, shorten or that names
    /// no commit, a short length out of range, a reference name that is not valid, a reference
    /// that moved, a `--paths` entry outside the repository, or a `--paths` entry that is not a
    /// supported pathspec, a `gix config get` key that is malformed, a value that is not UTF-8,
    /// a value that cannot be read as the requested `--type`, a path value that is not UTF-8, or a
    /// `gix config list` entry that cannot be listed (a value, path or subsection that is not UTF-8,
    /// or a section that cannot be read back), a valueless key read as a typed value, a `%(prefix)/`
    /// path whose install directory is unknown, a variable and a subsection with one name, or a
    /// section whose parse events break the grammar of a variable.
    #[error("{source}")]
    Located {
        /// Span of the argument the failure is attributed to, chosen by the command that raised it:
        /// the value of the flag or argument, or for `gix config list` the handle it was given.
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
        source: Box<GixError>,
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
        source: Box<GixError>,
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
        source: Box<GixError>,
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
        source: Box<GixError>,
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

    /// A `--type` value names no configuration value type.
    #[error("'{given}' is not a configuration value type")]
    UnknownConfigType {
        /// The value as written.
        given: String,
        /// Span of the `--type` value.
        span: Span,
    },
}

impl CommandError {
    /// `--repo` names a path that is not inside a git repository; `span` is the flag's value.
    #[must_use = "the error is the only result of this constructor"]
    pub fn not_a_repository(path: PathBuf, span: Span, source: GixError) -> Self {
        Self::NotARepository {
            path,
            span,
            source: Box::new(source),
        }
    }

    /// `--repo` names a repository that was found but cannot be opened.
    #[must_use = "the error is the only result of this constructor"]
    pub fn open_failed(path: PathBuf, span: Span, source: GixError) -> Self {
        Self::OpenFailed {
            path,
            span,
            source: Box::new(source),
        }
    }

    /// The repository behind a handle was moved or deleted; `span` is the handle value.
    #[must_use = "the error is the only result of this constructor"]
    pub fn handle_repository_gone(git_dir: PathBuf, span: Span, source: GixError) -> Self {
        Self::HandleRepositoryGone {
            git_dir,
            span,
            source: Box::new(source),
        }
    }

    /// The repository behind a handle could not be reopened; `span` is the handle value.
    #[must_use = "the error is the only result of this constructor"]
    pub fn handle_open_failed(path: PathBuf, span: Span, source: GixError) -> Self {
        Self::HandleOpenFailed {
            path,
            span,
            source: Box::new(source),
        }
    }

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
            Self::UnknownConfigType {
                given: _given,
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
            Self::UnknownConfigType {
                given: _given,
                span: _span,
            } => UNKNOWN_CONFIG_TYPE_CODE,
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
            Self::UnknownConfigType {
                span,
                given: _given,
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
                GixError::InvalidConfigKey { key: _key } => Some(INVALID_CONFIG_KEY_LABEL),
                GixError::ConfigValueNotUtf8 {
                    key: _key,
                    source: _source,
                    origin: _origin,
                } => Some(CONFIG_VALUE_NOT_UTF8_LABEL),
                GixError::ConfigValueConversion {
                    key: _key,
                    kind: _kind,
                    source: _source,
                    origin: _origin,
                } => Some(CONFIG_VALUE_CONVERSION_LABEL),
                GixError::ConfigValueMissing {
                    key: _key,
                    kind: _kind,
                    origin: _origin,
                } => Some(CONFIG_VALUE_MISSING_LABEL),
                GixError::InstallDirUnavailable {
                    key: _key,
                    source: _source,
                    origin: _origin,
                } => Some(INSTALL_DIR_UNAVAILABLE_LABEL),
                GixError::ConfigPathNotUtf8 {
                    key: _key,
                    origin: _origin,
                } => Some(CONFIG_PATH_NOT_UTF8_LABEL),
                GixError::ConfigSubsectionNotUtf8 {
                    key: _key,
                    source: _source,
                    origin: _origin,
                } => Some(CONFIG_SUBSECTION_NOT_UTF8_LABEL),
                GixError::ConfigNameClash {
                    section: _section,
                    name: _name,
                    variable_origin: _variable_origin,
                    subsection_origin: _subsection_origin,
                } => Some(CONFIG_NAME_CLASH_LABEL),
                GixError::ConfigSectionUnreadable {
                    section: _section,
                    source: _source,
                } => Some(CONFIG_SECTION_UNREADABLE_LABEL),
                GixError::ConfigSectionMalformed {
                    section: _section,
                    fault: _fault,
                } => Some(CONFIG_SECTION_MALFORMED_LABEL),
                GixError::ConfigKeyReset {
                    key: _key,
                    origin: _origin,
                } => Some(CONFIG_KEY_RESET_LABEL),
                GixError::NotFound {
                    path: _path,
                    source: _source,
                }
                | GixError::Open {
                    path: _path,
                    source: _source,
                } => None,
                GixError::RepositoryGone { git_dir: _git_dir } => None,
                GixError::Head(_)
                | GixError::Walk(_)
                | GixError::Spawn(_)
                | GixError::RefRead(_)
                | GixError::WalkerPanicked => None,
                GixError::Commit {
                    id: _id,
                    source: _source,
                } => None,
                GixError::Timestamp { id: _id } => None,
                GixError::UnbornHead { name: _name } | GixError::DanglingRef { name: _name } => {
                    None
                }
                GixError::RefUpdate {
                    name: _name,
                    source: _source,
                } => None,
                GixError::InvalidOid { value: _value } => None,
            },
            Self::UnknownConfigType {
                given: _given,
                span: _span,
            } => Some(UNKNOWN_CONFIG_TYPE_LABEL),
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
                source,
            } => {
                if matches!(
                    source,
                    GixError::ConfigKeyReset {
                        key: _key,
                        origin: _origin,
                    }
                ) {
                    Some(CONFIG_KEY_RESET_HELP)
                } else {
                    None
                }
            }
            Self::UnknownConfigType {
                given: _given,
                span: _span,
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
#[expect(
    clippy::too_many_lines,
    reason = "every variant is named with explicit fields: `{ .. }` trips rest_pattern_accessible_field"
)]
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
        }
        | GixError::Shorten {
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
        // `DanglingRef` is a failure to read HEAD's resolution, so it shares the ref-read code.
        GixError::RefRead(_) => REF_READ_FAILED_CODE,
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
        GixError::InvalidConfigKey { key: _key } => INVALID_CONFIG_KEY_CODE,
        GixError::ConfigValueNotUtf8 {
            key: _key,
            source: _source,
            origin: _origin,
        } => CONFIG_VALUE_NOT_UTF8_CODE,
        GixError::ConfigValueConversion {
            key: _key,
            kind: _kind,
            source: _source,
            origin: _origin,
        } => CONFIG_VALUE_CONVERSION_CODE,
        GixError::ConfigPathNotUtf8 {
            key: _key,
            origin: _origin,
        } => CONFIG_PATH_NOT_UTF8_CODE,
        GixError::ConfigValueMissing {
            key: _key,
            kind: _kind,
            origin: _origin,
        } => CONFIG_VALUE_MISSING_CODE,
        GixError::InstallDirUnavailable {
            key: _key,
            source: _source,
            origin: _origin,
        } => INSTALL_DIR_UNAVAILABLE_CODE,
        GixError::ConfigSubsectionNotUtf8 {
            key: _key,
            source: _source,
            origin: _origin,
        } => CONFIG_SUBSECTION_NOT_UTF8_CODE,
        GixError::ConfigNameClash {
            section: _section,
            name: _name,
            variable_origin: _variable_origin,
            subsection_origin: _subsection_origin,
        } => CONFIG_NAME_CLASH_CODE,
        GixError::ConfigSectionUnreadable {
            section: _section,
            source: _source,
        } => CONFIG_SECTION_UNREADABLE_CODE,
        GixError::ConfigSectionMalformed {
            section: _section,
            fault: _fault,
        } => CONFIG_SECTION_MALFORMED_CODE,
        GixError::ConfigKeyReset {
            key: _key,
            origin: _origin,
        } => CONFIG_KEY_RESET_CODE,
    }
}

/// The help line `Error codes: a, b, c` listing the codes of all `groups` in order.
#[must_use = "the line is the help text; discarding it drops the documentation"]
pub(super) fn error_codes_line(groups: &[&[&str]]) -> String {
    [CODES_LINE_PREFIX, &groups.concat().join(CODES_SEPARATOR)].concat()
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
