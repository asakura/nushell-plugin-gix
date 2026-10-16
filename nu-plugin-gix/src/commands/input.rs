//! Typed access to handle-valued pipeline input.

use crate::cdt::{self, Handle};

use nu_protocol::Value;

use super::{CommandError, error::WrongInput};

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
