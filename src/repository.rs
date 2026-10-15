//! The custom Nushell value returned by `gix open` and consumed by the other commands.

use nu_protocol::{CustomValue, ShellError, Span, Value, record};
use serde::{Deserialize, Serialize};

use std::{any::Any, path::PathBuf};

/// Handle to an opened git repository.
///
/// Only the path is stored: `gix::Repository` is not serializable, and Nushell
/// round-trips plugin custom values through MessagePack. Commands re-open the
/// repository from this path on every invocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GixRepository {
    /// Directory the repository was opened from.
    pub path: PathBuf,
}

impl GixRepository {
    /// Wraps `path` as a Nushell value spanning `span`.
    #[must_use = "returns the custom value; it is not emitted anywhere"]
    pub fn into_value(self, span: Span) -> Value {
        Value::custom(Box::new(self), span)
    }
}

#[typetag::serde]
impl CustomValue for GixRepository {
    fn clone_value(&self, span: Span) -> Value {
        self.clone().into_value(span)
    }

    fn type_name(&self) -> String {
        "git repository".to_owned()
    }

    fn to_base_value(&self, span: Span) -> Result<Value, ShellError> {
        Ok(Value::record(
            record! { "path" => Value::string(self.path.to_string_lossy(), span) },
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

    fn repo() -> GixRepository {
        GixRepository {
            path: PathBuf::from("/tmp/some/repo"),
        }
    }

    mod to_base_value {
        use super::*;

        #[rstest]
        fn is_a_record_with_the_path() -> Result<(), TestError> {
            let value = repo().to_base_value(Span::test_data())?;
            let record = value.as_record()?;

            let path = record
                .get("path")
                .ok_or_else(|| TestError::Failure("missing path column".to_owned()))?
                .as_str()?;

            assert_eq!(path, "/tmp/some/repo");

            Ok(())
        }
    }

    mod clone_value {
        use super::*;

        #[rstest]
        fn round_trips_as_custom_value() -> Result<(), TestError> {
            let cloned = repo().clone_value(Span::test_data());
            let custom = cloned.as_custom_value()?;

            assert_eq!(
                custom.as_any().downcast_ref::<GixRepository>(),
                Some(&repo())
            );

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
