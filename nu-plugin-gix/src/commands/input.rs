//! Typed access to handle-valued pipeline input.

use crate::cdt::{self, Handle, Repository};
use crate::commands::{CommandError, error::WrongInput};

use nu_plugin::EvaluatedCall;
use nu_plugin_gix_core::{GixError, OpenRepository, Registry};
use nu_protocol::{FromValue, PipelineData, Span, Spanned, Value};

use std::sync::Arc;

/// Borrows the pipeline input as handle type `H`, or fails with an error spanning the input.
pub fn handle_from<H>(input: &Value) -> Result<&H, CommandError>
where
    H: Handle,
{
    cdt::downcast::<H>(input).ok_or_else(|| {
        CommandError::WrongInput(WrongInput {
            expected: H::TYPE_NAME,
            hint: H::OBTAINED_BY,
            got: input.get_type().to_string(),
            span: input.span(),
        })
    })
}

/// Opens the repository a pipeline `handle` names, rebuilding it when the registry does not cache it.
///
/// A handle whose repository is gone or can no longer be opened is reported at `span`, the value
/// that carried the handle; other git-layer failures are passed through as [`CommandError::Git`].
pub fn acquire_handle(
    registry: &Registry,
    handle: &Repository,
    span: Span,
) -> Result<Arc<OpenRepository>, CommandError> {
    registry
        .acquire(handle.id, &handle.location())
        .map_err(|source| match &source {
            GixError::RepositoryGone { git_dir } => CommandError::HandleRepositoryGone {
                git_dir: git_dir.clone(),
                span,
                source,
            },
            GixError::Open {
                path,
                source: _source,
            } => CommandError::HandleOpenFailed {
                path: path.clone(),
                span,
                source,
            },
            _ => CommandError::Git(source),
        })
}

/// Collects the pipeline input into one value; a failing input stream is reported at `span`, the call that received it.
pub fn input_value(input: PipelineData, span: Span) -> Result<Value, CommandError> {
    input
        .into_value(span)
        .map_err(|source| CommandError::InvalidInput {
            source: Box::new(source),
            span,
        })
}

/// Reads the optional flag `flag`; a read failure points at the flag.
pub fn flag_arg<T>(call: &EvaluatedCall, flag: &str) -> Result<Option<Spanned<T>>, CommandError>
where
    T: FromValue,
{
    call.get_flag::<Spanned<T>>(flag)
        .map_err(|source| CommandError::InvalidArgument {
            source: Box::new(source),
            span: call.get_flag_span(flag).unwrap_or(call.head),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::cdt::Repository;

    use nu_protocol::Span;
    use rstest::rstest;

    use std::path::PathBuf;

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error("Test failure: {0}")]
        Failure(String),
    }

    fn repo() -> Repository {
        Repository {
            id: 1,
            path: PathBuf::from("/tmp/r"),
            git_dir: PathBuf::from("/tmp/r/.git"),
            worktree: None,
        }
    }

    mod handle_from {
        use super::*;

        #[rstest]
        fn accepts_the_expected_handle() -> Result<(), TestError> {
            let input = repo().into_value(Span::test_data());
            let found =
                handle_from::<Repository>(&input).map_err(|e| TestError::Failure(e.to_string()))?;

            assert_eq!(found, &repo());

            Ok(())
        }

        #[rstest]
        #[case::int(Value::int(1, Span::test_data()), "int")]
        #[case::string(Value::string("x", Span::test_data()), "string")]
        fn rejects_other_values(#[case] input: Value, #[case] got: &str) -> Result<(), TestError> {
            let err = handle_from::<Repository>(&input)
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert!(matches!(&err, CommandError::WrongInput(w) if w.got == got));

            Ok(())
        }
    }
}
