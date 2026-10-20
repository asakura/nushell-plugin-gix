//! Where a configuration occurrence comes from: its file, or else the kind of source it was read from.

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

/// Where a configuration occurrence comes from: the file it was read from, or else the kind of source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    /// The file the occurrence was read from, if it has one.
    pub path: Option<PathBuf>,
    /// The kind of source it was read from, such as local or environment.
    pub source: Source,
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
}
