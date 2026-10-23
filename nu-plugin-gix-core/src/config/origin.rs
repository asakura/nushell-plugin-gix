//! Where a configuration occurrence comes from: its file, or else the kind of source it was read from, and the git
//! [`Scope`] that source belongs to.

use gix::config::{Source, file::Metadata};

use std::{fmt, path::PathBuf};

const GIT_INSTALLATION: &str = "git installation";
const SYSTEM: &str = "system";
const XDG: &str = "xdg";
const USER: &str = "user";
const LOCAL: &str = "local";
const WORKTREE: &str = "worktree";
const ENVIRONMENT: &str = "environment";
const COMMAND_LINE: &str = "command line";
const API: &str = "api";
const ENVIRONMENT_OVERRIDE: &str = "environment override";
const SYSTEM_SCOPE: &str = "system";
const GLOBAL_SCOPE: &str = "global";
const LOCAL_SCOPE: &str = "local";
const WORKTREE_SCOPE: &str = "worktree";
const COMMAND_SCOPE: &str = "command";

/// Where a configuration occurrence comes from: the file it was read from, or else the kind of source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    /// The file the occurrence was read from, if it has one.
    pub path: Option<PathBuf>,
    /// The kind of source it was read from, such as local or environment.
    pub source: Source,
}

/// git's word for the kind of file a setting comes from (`git config --show-scope`); `User` and the XDG file are both
/// global; command line, environment and API settings are command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The system-wide file, or git's installation defaults.
    System,
    /// The user's file, or the XDG file: both are global.
    Global,
    /// The repository's own config file.
    Local,
    /// The worktree's config file.
    Worktree,
    /// A command line, environment or API setting.
    Command,
}

impl Scope {
    /// git's word for this scope, as `git config --show-scope` prints it.
    #[must_use = "the word is the only result of this function"]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => SYSTEM_SCOPE,
            Self::Global => GLOBAL_SCOPE,
            Self::Local => LOCAL_SCOPE,
            Self::Worktree => WORKTREE_SCOPE,
            Self::Command => COMMAND_SCOPE,
        }
    }
}

impl Origin {
    /// The origin of the section described by `meta`.
    #[must_use = "the origin is only useful when it is stored with an occurrence or an error"]
    pub fn new(meta: &Metadata) -> Self {
        Self {
            path: meta.path.clone(),
            source: meta.source,
        }
    }

    /// The git scope of the source this origin was read from.
    #[must_use = "the scope is the only result of this function"]
    pub const fn scope(&self) -> Scope {
        match self.source {
            Source::GitInstallation | Source::System => Scope::System,
            Source::Git | Source::User => Scope::Global,
            Source::Local => Scope::Local,
            Source::Worktree => Scope::Worktree,
            Source::Env | Source::Cli | Source::Api | Source::EnvOverride => Scope::Command,
        }
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.path {
            Some(path) => write!(f, "in {}", path.display()),
            None => write!(f, "from the {}", source_name(self.source)),
        }
    }
}

/// The name of a kind of configuration source, as the origin of an occurrence shows it.
#[must_use = "the name is the only result of this function"]
const fn source_name(source: Source) -> &'static str {
    match source {
        Source::GitInstallation => GIT_INSTALLATION,
        Source::System => SYSTEM,
        Source::Git => XDG,
        Source::User => USER,
        Source::Local => LOCAL,
        Source::Worktree => WORKTREE,
        Source::Env => ENVIRONMENT,
        Source::Cli => COMMAND_LINE,
        Source::Api => API,
        Source::EnvOverride => ENVIRONMENT_OVERRIDE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use rstest::rstest;

    mod source_name {
        use super::*;

        #[rstest]
        #[case::git_installation(Source::GitInstallation, "git installation")]
        #[case::system(Source::System, "system")]
        #[case::xdg(Source::Git, "xdg")]
        #[case::user(Source::User, "user")]
        #[case::local(Source::Local, "local")]
        #[case::worktree(Source::Worktree, "worktree")]
        #[case::environment(Source::Env, "environment")]
        #[case::command_line(Source::Cli, "command line")]
        #[case::api(Source::Api, "api")]
        #[case::environment_override(Source::EnvOverride, "environment override")]
        fn names_each_source(#[case] source: Source, #[case] name: &str) {
            assert_eq!(source_name(source), name);
        }
    }

    mod display {
        use super::*;

        #[rstest]
        #[case::file(Some("/p/config"), Source::Local, "in /p/config")]
        #[case::environment(None, Source::Env, "from the environment")]
        fn prints_the_file_or_else_the_source(
            #[case] path: Option<&str>,
            #[case] source: Source,
            #[case] expected: &str,
        ) {
            let origin = Origin {
                path: path.map(PathBuf::from),
                source,
            };

            assert_eq!(format!("{origin}"), expected);
        }
    }

    mod scope {
        use super::*;

        #[rstest]
        #[case::git_installation(Source::GitInstallation, Scope::System)]
        #[case::system(Source::System, Scope::System)]
        #[case::xdg(Source::Git, Scope::Global)]
        #[case::user(Source::User, Scope::Global)]
        #[case::local(Source::Local, Scope::Local)]
        #[case::worktree(Source::Worktree, Scope::Worktree)]
        #[case::environment(Source::Env, Scope::Command)]
        #[case::command_line(Source::Cli, Scope::Command)]
        #[case::api(Source::Api, Scope::Command)]
        #[case::environment_override(Source::EnvOverride, Scope::Command)]
        fn maps_each_source(#[case] source: Source, #[case] expected: Scope) {
            let origin = Origin { path: None, source };

            assert_eq!(origin.scope(), expected);
        }

        #[rstest]
        #[case::system(Scope::System, "system")]
        #[case::global(Scope::Global, "global")]
        #[case::local(Scope::Local, "local")]
        #[case::worktree(Scope::Worktree, "worktree")]
        #[case::command(Scope::Command, "command")]
        fn as_str_words(#[case] scope: Scope, #[case] word: &str) {
            assert_eq!(scope.as_str(), word);
        }
    }
}
