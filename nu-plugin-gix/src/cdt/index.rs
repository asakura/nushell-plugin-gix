//! The Nushell-side custom value for a scratch index file owned by the handle.

use crate::cdt::Handle;

use nu_protocol::{CustomValue, ShellError, Span, Value, record};
use serde::{Deserialize, Serialize};

use std::{any::Any, path::PathBuf};

/// Handle to a scratch index file, used to build trees without touching the repository's own index.
///
/// The file lives in the plugin's scratch directory and belongs to the handle. The
/// handle carries the repository's git directory so commands can rebuild the
/// repository after a plugin restart. Two handles are equal when they name the same
/// index file of the same repository.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Index {
    /// Key of this handle's entry in the plugin-side registry; only meaningful to the plugin process that issued it.
    pub id: u64,
    /// Git directory of the repository the index belongs to.
    pub git_dir: PathBuf,
    /// The scratch index file.
    pub path: PathBuf,
    /// Number of entries in the index when the handle was issued.
    pub entries: u64,
    /// Hex id of the tree the index was last written as, if it has been written.
    pub tree: Option<String>,
}

impl PartialEq for Index {
    fn eq(&self, other: &Self) -> bool {
        self.git_dir == other.git_dir && self.path == other.path
    }
}

impl Eq for Index {}

impl Index {
    /// Wraps the handle as a Nushell value spanning `span`.
    #[must_use = "returns the custom value; it is not emitted anywhere"]
    pub fn into_value(self, span: Span) -> Value {
        Value::custom(Box::new(self), span)
    }
}

impl Handle for Index {
    const TYPE_NAME: &'static str = "git index";
    const OBTAINED_BY: &'static str = "create one with `gix index`";
}

#[typetag::serde]
impl CustomValue for Index {
    fn clone_value(&self, span: Span) -> Value {
        self.clone().into_value(span)
    }

    fn type_name(&self) -> String {
        Self::TYPE_NAME.to_owned()
    }

    fn to_base_value(&self, span: Span) -> Result<Value, ShellError> {
        let tree = self
            .tree
            .as_ref()
            .map_or_else(|| Value::nothing(span), |id| Value::string(id, span));

        Ok(Value::record(
            record! {
                "path" => Value::string(self.path.to_string_lossy(), span),
                "entries" => Value::int(i64::try_from(self.entries).unwrap_or(i64::MAX), span),
                "tree" => tree,
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

    fn index() -> Index {
        Index {
            id: 3,
            git_dir: PathBuf::from("/tmp/r/.git"),
            path: PathBuf::from("/tmp/scratch/index-3"),
            entries: 12,
            tree: Some("abc123".to_owned()),
        }
    }

    mod to_base_value {
        use super::*;

        #[rstest]
        fn is_a_record_of_path_entries_and_tree() -> Result<(), TestError> {
            let value = index().to_base_value(Span::test_data())?;
            let record = value.as_record()?;
            let columns: Vec<&String> = record.columns().collect();

            assert_eq!(columns, ["path", "entries", "tree"]);

            let entries = record
                .get("entries")
                .ok_or_else(|| TestError::Failure("missing entries column".to_owned()))?
                .as_int()?;

            assert_eq!(entries, 12);

            Ok(())
        }

        #[rstest]
        fn shows_nothing_for_an_unwritten_tree() -> Result<(), TestError> {
            let unwritten = Index {
                tree: None,
                ..index()
            };
            let value = unwritten.to_base_value(Span::test_data())?;
            let tree = value
                .as_record()?
                .get("tree")
                .ok_or_else(|| TestError::Failure("missing tree column".to_owned()))?
                .clone();

            assert!(tree.is_nothing());

            Ok(())
        }
    }

    mod eq {
        use super::*;

        #[rstest]
        fn ignores_the_registry_id() {
            let other = Index {
                id: 99,
                entries: 0,
                tree: None,
                ..index()
            };

            assert_eq!(index(), other);
        }

        #[rstest]
        fn differs_for_another_file() {
            let other = Index {
                path: PathBuf::from("/tmp/scratch/index-4"),
                ..index()
            };

            assert_ne!(index(), other);
        }
    }

    mod type_name {
        use super::*;

        #[rstest]
        fn is_git_index() {
            assert_eq!(index().type_name(), "git index");
        }
    }

    mod clone_value {
        use super::*;

        #[rstest]
        fn round_trips_as_custom_value() -> Result<(), TestError> {
            let cloned = index().clone_value(Span::test_data());
            let custom = cloned.as_custom_value()?;

            assert_eq!(custom.as_any().downcast_ref::<Index>(), Some(&index()));

            Ok(())
        }
    }
}
