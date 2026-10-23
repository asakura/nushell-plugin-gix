//! Error type for the git layer; knows nothing about Nushell.

use crate::config::{ConfigKey, ConfigType, Origin, SectionHeader, VariableName};
use crate::refs::{RefExpect, RefTarget};

use gix::config::parse::Error as ParseError;

use std::{io, path::PathBuf, str::Utf8Error, sync::Arc};

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

    /// A revision expression names an object that does not peel to a commit, such as a tree or a blob.
    #[error("revision '{expression}' names a {kind}, not a commit")]
    NotACommit {
        /// The revision expression as written.
        expression: String,
        /// Kind of the object it names, such as `tree` or `blob`.
        kind: String,
    },

    /// A path names a location outside the repository's worktree.
    #[error("path '{path}' is outside the repository")]
    PathOutsideRepository {
        /// The path as written.
        path: String,
    },

    /// A path given to the history filter is not a valid git pathspec.
    #[error("invalid pathspec '{spec}'")]
    Pathspec {
        /// The pathspec as written.
        spec: String,
        /// The underlying `gix` error.
        #[source]
        source: gix::Error,
    },

    /// A pathspec uses attribute magic (`:(attr:...)`), which the history filter cannot evaluate.
    #[error("pathspec '{spec}' uses attribute magic, which is not supported")]
    PathspecAttributes {
        /// The pathspec as written.
        spec: String,
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

    /// The background thread that walks history panicked, so the history may be incomplete.
    #[error("the commit walker thread panicked")]
    WalkerPanicked,

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

    /// A configuration key is not of the form `section.name` or `section.subsection.name`.
    #[error("'{key}' is not a valid configuration key")]
    InvalidConfigKey {
        /// The key as written.
        key: String,
    },

    /// The value of a configuration key is not valid UTF-8.
    #[error("the value of configuration key '{key}' is not valid UTF-8 ({origin})")]
    ConfigValueNotUtf8 {
        /// The key that was read.
        key: ConfigKey,
        /// The decoding failure.
        #[source]
        source: Utf8Error,
        /// Where the value was read from.
        origin: Arc<Origin>,
    },

    /// The value of a configuration key cannot be read as the requested type.
    #[error("the value of configuration key '{key}' cannot be read as {kind} ({origin})")]
    ConfigValueConversion {
        /// The key that was read.
        key: ConfigKey,
        /// The type the value was read as.
        kind: ConfigType,
        /// The underlying `gix` error.
        #[source]
        source: gix::Error,
        /// Where the value was read from.
        origin: Arc<Origin>,
    },

    /// A configuration key has no value (it was written without `=`), but a value of the requested type was needed.
    #[error("configuration key '{key}' has no value, but it was read as {kind} ({origin})")]
    ConfigValueMissing {
        /// The key that was read.
        key: ConfigKey,
        /// The type the value was needed as.
        kind: ConfigType,
        /// Where the valueless key was read from.
        origin: Arc<Origin>,
    },

    /// A path uses `%(prefix)/` but the install directory of git, the directory of the running binary, could not be
    /// determined.
    #[error(
        "the path of configuration key '{key}' uses %(prefix)/ but the install directory of git is unknown ({origin})"
    )]
    InstallDirUnavailable {
        /// The key that was read.
        key: ConfigKey,
        /// Why the install directory could not be determined.
        #[source]
        source: Arc<gix::Error>,
        /// Where the path was read from.
        origin: Arc<Origin>,
    },

    /// The path in a configuration key is not valid UTF-8.
    #[error("the path in configuration key '{key}' is not valid UTF-8 ({origin})")]
    ConfigPathNotUtf8 {
        /// The key that was read.
        key: ConfigKey,
        /// Where the path was read from.
        origin: Arc<Origin>,
    },

    /// The values of a list-valued configuration key were all removed by an empty value
    /// (`name =`), so the key has no value.
    #[error(
        "configuration key '{key}' was reset ({origin}): an empty value removed all its values, so it has none"
    )]
    ConfigKeyReset {
        /// The key that was read.
        key: ConfigKey,
        /// Where the emptying value was read from.
        origin: Arc<Origin>,
    },

    /// The subsection of a configuration key is not valid UTF-8.
    #[error("the subsection of configuration key '{key}' is not valid UTF-8 ({origin})")]
    ConfigSubsectionNotUtf8 {
        /// The key that was read; its subsection is the part that is not UTF-8.
        key: ConfigKey,
        /// The decoding failure.
        #[source]
        source: Utf8Error,
        /// Where the key was read from.
        origin: Arc<Origin>,
    },

    /// A configuration section could not be read back as its variable names and values, because its bytes
    /// cannot be parsed again.
    #[error("the variables of configuration section '{section}' could not be read")]
    ConfigSectionUnreadable {
        /// The section, with its subsection when it has one.
        section: SectionHeader,
        /// The parse failure.
        #[source]
        source: ParseError,
    },

    /// The parse events of a configuration section broke the grammar gix guarantees, so a variable would be lost.
    #[error("the variables of configuration section '{section}' could not be read: {fault}")]
    ConfigSectionMalformed {
        /// The section, with its subsection when it has one.
        section: SectionHeader,
        /// The event that broke the grammar.
        fault: SectionFault,
    },
}

/// The parse events of one configuration section that break the grammar of a variable: a name followed by no value,
/// or a value without a name.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SectionFault {
    /// A variable name was started and no value event ended it: another name began, or the events of the section ended.
    #[error("the variable '{0}' has no value")]
    NameWithoutValue(VariableName),
    /// A value event (`KeyValueSeparator`, `ValueNotDone`, `Value`, `ValueDone`) arrived with no variable name pending.
    #[error("a value has no variable name")]
    ValueWithoutName,
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::{ScalarType, SectionName, Subsection, VariableName};

    use gix::config::Source;
    use rstest::rstest;

    use std::{error::Error as _, slice, str};

    #[derive(thiserror::Error, Debug)]
    enum TestError {
        #[error("Test failure: {0}")]
        Failure(String),
    }

    fn key() -> ConfigKey {
        ConfigKey::new(SectionName::from("core"), None, VariableName::from("bare"))
    }

    fn origin(path: &str) -> Origin {
        Origin {
            path: Some(PathBuf::from(path)),
            source: Source::Local,
        }
    }

    fn utf8_error() -> Result<Utf8Error, TestError> {
        match str::from_utf8(slice::from_ref(&0xff_u8)) {
            Ok(_) => Err(TestError::Failure("0xff decoded as UTF-8".to_owned())),
            Err(error) => Ok(error),
        }
    }

    mod config_messages {
        use super::*;

        #[rstest]
        fn name_the_key_and_the_origin() -> Result<(), TestError> {
            let errors = [
                GixError::ConfigValueNotUtf8 {
                    key: key(),
                    source: utf8_error()?,
                    origin: Arc::new(origin("/p/config")),
                },
                GixError::ConfigValueConversion {
                    key: key(),
                    kind: ConfigType::Scalar(ScalarType::Bool),
                    source: gix::Error::from_error(io::Error::other("not a boolean")),
                    origin: Arc::new(origin("/p/config")),
                },
                GixError::ConfigValueMissing {
                    key: key(),
                    kind: ConfigType::Scalar(ScalarType::Int),
                    origin: Arc::new(origin("/p/config")),
                },
                GixError::ConfigPathNotUtf8 {
                    key: key(),
                    origin: Arc::new(origin("/p/config")),
                },
                GixError::ConfigKeyReset {
                    key: key(),
                    origin: Arc::new(origin("/p/config")),
                },
                GixError::ConfigSubsectionNotUtf8 {
                    key: key(),
                    source: utf8_error()?,
                    origin: Arc::new(origin("/p/config")),
                },
            ];

            for error in &errors {
                let message = error.to_string();
                assert!(message.contains("core.bare"), "{message}");
                assert!(message.contains("in /p/config"), "{message}");
            }
            Ok(())
        }

        #[rstest]
        fn valueless_names_the_kind() {
            let error = GixError::ConfigValueMissing {
                key: key(),
                kind: ConfigType::Path,
                origin: Arc::new(origin("/p/config")),
            };

            assert!(error.to_string().contains(&format!("{}", ConfigType::Path)));
        }

        #[rstest]
        fn install_dir_unavailable_names_key_and_origin() {
            let error = GixError::InstallDirUnavailable {
                key: key(),
                source: Arc::new(gix::Error::from_error(io::Error::other(
                    "current_exe is unreadable",
                ))),
                origin: Arc::new(origin("/p/config")),
            };
            let message = error.to_string();

            assert!(message.contains("core.bare"), "{message}");
            assert!(message.contains("in /p/config"), "{message}");
        }

        #[rstest]
        #[case::name_without_value(
            SectionFault::NameWithoutValue(VariableName::from("answer")),
            "answer"
        )]
        #[case::value_without_name(SectionFault::ValueWithoutName, "value")]
        fn section_malformed_message_names_the_variable(
            #[case] fault: SectionFault,
            #[case] expected: &str,
        ) {
            let error = GixError::ConfigSectionMalformed {
                section: SectionHeader {
                    section: SectionName::from("gixtest"),
                    subsection: None,
                },
                fault,
            };

            let message = error.to_string();
            assert!(message.contains("gixtest"), "{message}");
            assert!(message.contains(expected), "{message}");
            assert!(error.source().is_none(), "{message}");
        }

        #[rstest]
        fn subsection_key_renders_its_subsection() {
            let error = GixError::ConfigPathNotUtf8 {
                key: ConfigKey::new(
                    SectionName::from("remote"),
                    Some(Subsection::from(b"origin")),
                    VariableName::from("url"),
                ),
                origin: Arc::new(origin("/p/config")),
            };

            assert!(error.to_string().contains("remote.origin.url"));
        }
    }
}
