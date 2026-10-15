//! The Nushell-side custom value returned by `gix open`; a handle of on-disk locations with no `gix` types.

use crate::git::RepositoryLocation;

use nu_protocol::{CustomValue, ShellError, Span, Value, record};
use serde::{Deserialize, Serialize};

use std::{any::Any, path::PathBuf};

/// Handle to an opened git repository.
///
/// Only on-disk locations are stored: `gix::Repository` is not serializable, and
/// Nushell round-trips plugin custom values through MessagePack. Commands re-open
/// the repository from these fields, so the handle survives a plugin restart.
/// Two handles are equal when they name the same git directory.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Repository {
    /// Directory that opens the repository: its work tree, or the git directory when bare.
    pub path: PathBuf,
    /// The repository's git directory.
    pub git_dir: PathBuf,
    /// The checked-out work tree, or `None` for a bare repository.
    pub work_tree: Option<PathBuf>,
}

impl PartialEq for Repository {
    fn eq(&self, other: &Self) -> bool {
        self.git_dir == other.git_dir
    }
}

impl Eq for Repository {}

impl From<RepositoryLocation> for Repository {
    fn from(location: RepositoryLocation) -> Self {
        Self {
            path: location.root().to_owned(),
            git_dir: location.git_dir,
            work_tree: location.work_tree,
        }
    }
}

impl Repository {
    /// Wraps `path` as a Nushell value spanning `span`.
    #[must_use = "returns the custom value; it is not emitted anywhere"]
    pub fn into_value(self, span: Span) -> Value {
        Value::custom(Box::new(self), span)
    }
}

#[typetag::serde]
impl CustomValue for Repository {
    fn clone_value(&self, span: Span) -> Value {
        self.clone().into_value(span)
    }

    fn type_name(&self) -> String {
        "git repository".to_owned()
    }

    fn to_base_value(&self, span: Span) -> Result<Value, ShellError> {
        let work_tree = self.work_tree.as_ref().map_or_else(
            || Value::nothing(span),
            |dir| Value::string(dir.to_string_lossy(), span),
        );

        Ok(Value::record(
            record! {
                "path" => Value::string(self.path.to_string_lossy(), span),
                "git_dir" => Value::string(self.git_dir.to_string_lossy(), span),
                "work_tree" => work_tree,
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
            path: PathBuf::from("/tmp/some/repo"),
            git_dir: PathBuf::from("/tmp/some/repo/.git"),
            work_tree: Some(PathBuf::from("/tmp/some/repo")),
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
                work_tree: None,
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
