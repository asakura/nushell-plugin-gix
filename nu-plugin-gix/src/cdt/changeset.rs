//! The Nushell-side custom value for the result of comparing two sides of a repository.

use crate::cdt::Handle;

use nu_plugin_gix_core::Side;
use nu_protocol::{CustomValue, ShellError, Span, Value, record};
use serde::{Deserialize, Serialize};

use std::{any::Any, path::PathBuf};

/// How a path differs between the two sides.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeKind {
    /// Present only on the `to` side.
    Added,
    /// Present on both sides with different content.
    Modified,
    /// Present only on the `from` side.
    Deleted,
}

impl ChangeKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Modified => "modified",
            Self::Deleted => "deleted",
        }
    }
}

/// One changed path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    /// Path relative to the work tree root.
    pub path: String,
    /// How the path differs.
    pub kind: ChangeKind,
}

/// Handle to the result of comparing two sides, held so a script can filter, split and stage it without recomputing.
///
/// The handle carries the changed paths, so it prints and compares without any
/// plugin-side state and survives a plugin restart. Heavier data, such as hunks,
/// is cached plugin-side and rebuilt from `from`, `to` and `base`. `base` stamps the
/// state the comparison was computed from, so a later write can detect that it went stale.
/// Two handles are equal when they compare the same sides and found the same changes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Changeset {
    /// Key of this handle's entry in the plugin-side registry; only meaningful to the plugin process that issued it.
    pub id: u64,
    /// Git directory of the repository the comparison was made in.
    pub git_dir: PathBuf,
    /// The side changes are measured from.
    pub from: Side,
    /// The side changes are measured to.
    pub to: Side,
    /// Stamp of the compared state: what a write must still find to apply the changeset.
    pub base: String,
    /// The changed paths, sorted by path.
    pub changes: Vec<Change>,
}

impl PartialEq for Changeset {
    fn eq(&self, other: &Self) -> bool {
        self.from == other.from && self.to == other.to && self.changes == other.changes
    }
}

impl Eq for Changeset {}

impl Changeset {
    /// Wraps the handle as a Nushell value spanning `span`.
    #[must_use = "returns the custom value; it is not emitted anywhere"]
    pub fn into_value(self, span: Span) -> Value {
        Value::custom(Box::new(self), span)
    }
}

impl Handle for Changeset {
    const TYPE_NAME: &'static str = "git changeset";
    const OBTAINED_BY: &'static str = "create one with `gix changes`";
}

#[typetag::serde]
impl CustomValue for Changeset {
    fn clone_value(&self, span: Span) -> Value {
        self.clone().into_value(span)
    }

    fn type_name(&self) -> String {
        Self::TYPE_NAME.to_owned()
    }

    fn to_base_value(&self, span: Span) -> Result<Value, ShellError> {
        let rows = self
            .changes
            .iter()
            .map(|change| {
                Value::record(
                    record! {
                        "path" => Value::string(change.path.as_str(), span),
                        "status" => Value::string(change.kind.label(), span),
                        "from" => Value::string(self.from.to_string(), span),
                        "to" => Value::string(self.to.to_string(), span),
                    },
                    span,
                )
            })
            .collect();

        Ok(Value::list(rows, span))
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

    fn changeset() -> Changeset {
        Changeset {
            id: 5,
            git_dir: PathBuf::from("/tmp/r/.git"),
            from: Side::Head,
            to: Side::Worktree,
            base: "abc".to_owned(),
            changes: vec![
                Change {
                    path: "a.txt".to_owned(),
                    kind: ChangeKind::Modified,
                },
                Change {
                    path: "b.txt".to_owned(),
                    kind: ChangeKind::Added,
                },
            ],
        }
    }

    mod to_base_value {
        use super::*;

        #[rstest]
        fn is_a_table_with_a_row_per_change() -> Result<(), TestError> {
            let value = changeset().to_base_value(Span::test_data())?;
            let rows = value.as_list()?;

            assert_eq!(rows.len(), 2);

            let status = rows
                .first()
                .ok_or_else(|| TestError::Failure("missing first row".to_owned()))?
                .as_record()?
                .get("status")
                .ok_or_else(|| TestError::Failure("missing status column".to_owned()))?
                .as_str()?
                .to_owned();

            assert_eq!(status, "modified");

            Ok(())
        }

        #[rstest]
        fn is_empty_without_changes() -> Result<(), TestError> {
            let empty = Changeset {
                changes: Vec::new(),
                ..changeset()
            };
            let value = empty.to_base_value(Span::test_data())?;

            assert_eq!(value.as_list()?.len(), 0);

            Ok(())
        }
    }

    mod eq {
        use super::*;

        #[rstest]
        fn ignores_id_and_base_stamp() {
            let other = Changeset {
                id: 99,
                base: "def".to_owned(),
                ..changeset()
            };

            assert_eq!(changeset(), other);
        }

        #[rstest]
        #[case::other_side(Changeset { to: Side::Index, ..changeset() })]
        #[case::no_changes(Changeset { changes: Vec::new(), ..changeset() })]
        fn differs_when_sides_or_content_differ(#[case] other: Changeset) {
            assert_ne!(changeset(), other);
        }
    }

    mod type_name {
        use super::*;

        #[rstest]
        fn is_git_changeset() {
            assert_eq!(changeset().type_name(), "git changeset");
        }
    }

    mod clone_value {
        use super::*;

        #[rstest]
        fn round_trips_as_custom_value() -> Result<(), TestError> {
            let cloned = changeset().clone_value(Span::test_data());
            let custom = cloned.as_custom_value()?;

            assert_eq!(
                custom.as_any().downcast_ref::<Changeset>(),
                Some(&changeset())
            );

            Ok(())
        }
    }
}
