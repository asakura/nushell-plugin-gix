//! The Nushell-side custom value returned by `gix open`; a handle of on-disk locations with no `gix` types.

use crate::cdt::Handle;
use nu_plugin_gix_core::RepositoryLocation;

use nu_protocol::{CustomValue, ShellError, Span, Value, record};
use serde::{Deserialize, Serialize};

use std::{any::Any, path::PathBuf};

/// Handle to an opened git repository.
///
/// Only on-disk locations are stored: `gix::Repository` is not serializable, and
/// Nushell round-trips plugin custom values through MessagePack. Commands re-open
/// the repository from these fields, so the handle survives a plugin restart.
/// Two handles are equal when they name the same git directory.
///
/// Opened `gix::Repository` state is deliberately not held here: it is not
/// serializable and `nu-plugin-gix` must not touch `gix` types directly. It is cached plugin-side
/// in the git-side [`Registry`](nu_plugin_gix_core::Registry), keyed by `id`; after a plugin
/// restart the cache is empty and is rebuilt from the other fields.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Repository {
    /// Key of this handle's entry in the plugin-side registry; only meaningful to the plugin process that issued it.
    pub id: u64,
    /// Directory that opens the repository: its worktree, or the git directory when bare.
    pub path: PathBuf,
    /// The repository's git directory.
    pub git_dir: PathBuf,
    /// The checked-out worktree, or `None` for a bare repository.
    pub worktree: Option<PathBuf>,
}

impl PartialEq for Repository {
    fn eq(&self, other: &Self) -> bool {
        self.git_dir == other.git_dir
    }
}

impl Eq for Repository {}

impl Repository {
    /// Builds the handle `id` for the repository found at `location`.
    #[must_use = "returns the handle; it is not emitted anywhere"]
    pub fn new(id: u64, location: RepositoryLocation) -> Self {
        Self {
            id,
            path: location.root().to_owned(),
            git_dir: location.git_dir,
            worktree: location.worktree,
        }
    }

    /// The on-disk location this handle names, from which its state can be rebuilt.
    #[must_use = "returns the location; the handle is unchanged"]
    pub fn location(&self) -> RepositoryLocation {
        RepositoryLocation {
            git_dir: self.git_dir.clone(),
            worktree: self.worktree.clone(),
        }
    }

    /// Wraps `path` as a Nushell value spanning `span`.
    #[must_use = "returns the custom value; it is not emitted anywhere"]
    pub fn into_value(self, span: Span) -> Value {
        Value::custom(Box::new(self), span)
    }
}

impl Handle for Repository {
    const TYPE_NAME: &'static str = "git repository";
    const OBTAINED_BY: &'static str = "pipe the output of `gix open` into this command";
}

#[typetag::serde]
impl CustomValue for Repository {
    fn clone_value(&self, span: Span) -> Value {
        self.clone().into_value(span)
    }

    fn notify_plugin_on_drop(&self) -> bool {
        true
    }

    fn type_name(&self) -> String {
        Self::TYPE_NAME.to_owned()
    }

    fn to_base_value(&self, span: Span) -> Result<Value, ShellError> {
        let worktree = self.worktree.as_ref().map_or_else(
            || Value::nothing(span),
            |dir| Value::string(dir.to_string_lossy(), span),
        );

        Ok(Value::record(
            record! {
                "path" => Value::string(self.path.to_string_lossy(), span),
                "git_dir" => Value::string(self.git_dir.to_string_lossy(), span),
                "worktree" => worktree,
            },
            span,
        ))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_mut_any(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use rstest::rstest;

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Shell(#[from] ShellError),

        #[error("Test failure: {0}")]
        Failure(String),
    }

    fn repo() -> Repository {
        Repository {
            id: 7,
            path: PathBuf::from("/tmp/some/repo"),
            git_dir: PathBuf::from("/tmp/some/repo/.git"),
            worktree: Some(PathBuf::from("/tmp/some/repo")),
        }
    }

    mod to_base_value {
        use super::*;

        #[rstest]
        fn is_a_record_with_the_locations() -> Result<(), TestError> {
            let value = repo().to_base_value(Span::test_data())?;
            let record = value.as_record()?;

            let path = record
                .get("path")
                .ok_or_else(|| TestError::Failure("missing path column".to_owned()))?
                .as_str()?;

            assert_eq!(path, "/tmp/some/repo");

            let git_dir = record
                .get("git_dir")
                .ok_or_else(|| TestError::Failure("missing git_dir column".to_owned()))?
                .as_str()?;

            assert_eq!(git_dir, "/tmp/some/repo/.git");

            Ok(())
        }
    }

    mod eq {
        use super::*;

        #[rstest]
        fn compares_git_dir_only() {
            let other = Repository {
                path: PathBuf::from("/elsewhere"),
                worktree: None,
                ..repo()
            };

            assert_eq!(repo(), other);
        }

        #[rstest]
        fn differs_for_other_git_dir() {
            let other = Repository {
                git_dir: PathBuf::from("/tmp/other/.git"),
                ..repo()
            };

            assert_ne!(repo(), other);
        }
    }

    mod location {
        use super::*;

        #[rstest]
        fn round_trips_through_new() {
            let handle = repo();

            assert_eq!(Repository::new(7, handle.location()), handle);
            assert_eq!(Repository::new(7, handle.location()).path, handle.path);
        }
    }

    mod notify_plugin_on_drop {
        use super::*;

        #[rstest]
        fn is_enabled() {
            assert!(repo().notify_plugin_on_drop());
        }
    }

    mod clone_value {
        use super::*;

        #[rstest]
        fn round_trips_as_custom_value() -> Result<(), TestError> {
            let cloned = repo().clone_value(Span::test_data());
            let custom = cloned.as_custom_value()?;

            assert_eq!(custom.as_any().downcast_ref::<Repository>(), Some(&repo()));

            Ok(())
        }
    }

    mod type_name {
        use super::*;

        #[rstest]
        fn is_gix_repository() {
            assert_eq!(repo().type_name(), "git repository");
        }
    }
}
