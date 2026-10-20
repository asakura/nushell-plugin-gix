//! Typed access to pipeline input and call arguments: handle-valued input, the raw value a command
//! receives, the opened repository behind a handle, argument reads that point their errors at
//! the argument they came from, and checks shared by commands such as a non-blank revision.

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
        .map_err(|source| handle_failure(source, span))
}

/// Maps a failure to acquire a handle's repository to the error that points at the handle.
fn handle_failure(source: GixError, span: Span) -> CommandError {
    match &source {
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
    }
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

/// The span of the positional argument at `position`, or of the call when it is missing.
#[must_use = "the span locates the argument in an error; discarding it reports nothing"]
pub fn argument_span(call: &EvaluatedCall, position: usize) -> Span {
    call.nth(position).map_or(call.head, |value| value.span())
}

/// Reads the required positional argument at `position`; a read failure points at the argument, or at the call when it is missing.
pub fn positional_arg<T>(call: &EvaluatedCall, position: usize) -> Result<Spanned<T>, CommandError>
where
    T: FromValue,
{
    call.req::<Spanned<T>>(position)
        .map_err(|source| CommandError::InvalidArgument {
            source: Box::new(source),
            span: argument_span(call, position),
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

/// Reads the switch `flag`: `true` when it is given; a read failure points at the flag.
pub fn switch_arg(call: &EvaluatedCall, flag: &str) -> Result<bool, CommandError> {
    call.has_flag(flag)
        .map_err(|source| CommandError::InvalidArgument {
            source: Box::new(source),
            span: call.get_flag_span(flag).unwrap_or(call.head),
        })
}

/// Reads the optional positional argument at `position`; a read failure points at the argument.
pub fn optional_arg<T>(
    call: &EvaluatedCall,
    position: usize,
) -> Result<Option<Spanned<T>>, CommandError>
where
    T: FromValue,
{
    call.opt::<Spanned<T>>(position)
        .map_err(|source| CommandError::InvalidArgument {
            source: Box::new(source),
            span: argument_span(call, position),
        })
}

/// Rejects a blank or whitespace-only revision expression at its own span.
pub fn non_blank_revision(spec: Spanned<String>) -> Result<Spanned<String>, CommandError> {
    if spec.item.trim().is_empty() {
        return Err(CommandError::EmptyRevision { span: spec.span });
    }

    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::cdt::Repository;

    use nu_plugin_gix_core::Registry;
    use nu_plugin_gix_testing_support::{
        FixtureError, FixtureRepo as _, HEAD_FILE, TestRepo, test_repo,
    };
    use nu_protocol::{ListStream, ShellError, Signals};
    use rstest::rstest;

    use std::{assert_matches, fs, io, iter, path::Path};

    const INVALID_INPUT_CODE: &str = "gix::invalid-input";
    const INVALID_ARGUMENT_CODE: &str = "gix::invalid-argument";
    const UPSTREAM_FAILURE: &str = "upstream failed";
    const EXPECT_FLAG: &str = "expect";
    const MESSAGE_FLAG: &str = "message";
    const ALL_SWITCH: &str = "all";
    const FIRST_POSITION: usize = 0;
    const SECOND_POSITION: usize = 1;
    const CALL_SPAN: Span = Span { start: 1, end: 7 };
    const FLAG_SPAN: Span = Span { start: 9, end: 20 };
    const ARGUMENT_SPAN: Span = Span { start: 21, end: 25 };
    const UNREADABLE_VALUE: i64 = 1;
    const PRESENT_VALUE: &str = "main";

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Shell(#[from] ShellError),

        #[error(transparent)]
        Fixture(#[from] FixtureError),

        #[error(transparent)]
        Command(#[from] CommandError),

        #[error(transparent)]
        Git(#[from] GixError),

        #[error(transparent)]
        Io(#[from] io::Error),

        #[error("Test failure: {0}")]
        Failure(String),
    }

    /// Checks `eval` cannot express: the engine's input type check rejects non-custom
    /// pipeline input before the command runs, so the `WrongInput` mapping is only
    /// reachable directly until a command emits a second handle type.
    mod wrong_input_mapping {
        use super::*;

        #[rstest]
        #[case::int(Value::int(1, Span::test_data()), "int")]
        #[case::string(Value::string("x", Span::test_data()), "string")]
        fn maps_the_received_type_into_the_error(
            #[case] input: Value,
            #[case] got: &str,
        ) -> Result<(), TestError> {
            let err = handle_from::<Repository>(&input)
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert_matches!(
                &err,
                CommandError::WrongInput(w) if w.got == got,
                "got {err:?}"
            );

            Ok(())
        }
    }

    /// Unit tests for the input read that no script can make fail: the engine type-checks the
    /// pipeline input before a command runs, so only a failing stream reaches this code.
    mod input_value {
        use super::*;

        #[rstest]
        fn reports_a_failing_input_stream_as_invalid_input() -> Result<(), TestError> {
            let span = Span::test_data();

            let failure = Value::error(
                ShellError::NushellFailed {
                    msg: UPSTREAM_FAILURE.to_owned(),
                },
                span,
            );

            let input = ListStream::new(iter::once(failure), span, Signals::empty()).into();

            let err = input_value(input, span)
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert_matches!(&err, CommandError::InvalidInput { .. });

            assert_eq!(
                err.into_labeled(span).code.as_deref(),
                Some(INVALID_INPUT_CODE)
            );

            Ok(())
        }
    }

    /// Handle acquisition: a handle whose repository is gone or cannot be reopened is reported at the
    /// span of the value that carried it; other git-layer failures pass through unchanged.
    mod acquire_handle {
        use super::*;

        const HANDLE_SPAN: Span = Span { start: 30, end: 36 };
        const OTHER_REF: &str = "refs/heads/main";

        /// The handle `gix open` returns for the repository at `path`, registered in `registry`.
        fn handle_for(registry: &Registry, path: &Path) -> Result<Repository, GixError> {
            let (id, location) = registry.discover_and_register(path)?;

            Ok(Repository::new(id, location))
        }

        mod deleted_repository {
            use super::*;

            #[rstest]
            fn reports_the_handle_span(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;
                let registry = Registry::new();
                let handle = handle_for(&registry, repo.path())?;

                fs::remove_dir_all(repo.path())?;

                let err = acquire_handle(&registry, &handle, HANDLE_SPAN)
                    .err()
                    .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

                assert_matches!(
                    &err,
                    CommandError::HandleRepositoryGone { span, .. } if *span == HANDLE_SPAN,
                    "got {err:?}"
                );

                Ok(())
            }
        }

        mod unopenable_rebuild {
            use super::*;

            #[rstest]
            fn reports_the_handle_span(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;
                let handle = handle_for(&Registry::new(), repo.path())?;

                // A fresh registry must rebuild the repository, which fails without its HEAD file.
                fs::remove_file(handle.git_dir.join(HEAD_FILE))?;

                let err = acquire_handle(&Registry::new(), &handle, HANDLE_SPAN)
                    .err()
                    .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

                assert_matches!(
                    &err,
                    CommandError::HandleOpenFailed { span, .. } if *span == HANDLE_SPAN,
                    "got {err:?}"
                );

                Ok(())
            }
        }

        mod other_failures {
            use super::*;

            #[rstest]
            fn pass_through_as_git_errors() {
                let err = handle_failure(
                    GixError::UnbornHead {
                        name: OTHER_REF.to_owned(),
                    },
                    HANDLE_SPAN,
                );

                assert_matches!(
                    &err,
                    CommandError::Git(GixError::UnbornHead { .. }),
                    "got {err:?}"
                );
            }
        }
    }

    /// Spans of arguments: the argument's own span when present, the call's span when not.
    mod argument_span {
        use super::*;

        #[rstest]
        fn returns_the_span_of_a_present_argument() {
            let call = EvaluatedCall::new(CALL_SPAN)
                .with_positional(Value::string(PRESENT_VALUE, ARGUMENT_SPAN));

            assert_eq!(argument_span(&call, FIRST_POSITION), ARGUMENT_SPAN);
        }

        #[rstest]
        #[case::first(FIRST_POSITION)]
        #[case::second(SECOND_POSITION)]
        fn falls_back_to_the_call_span_when_the_argument_is_missing(#[case] position: usize) {
            let call = EvaluatedCall::new(CALL_SPAN);

            assert_eq!(argument_span(&call, position), CALL_SPAN);
        }
    }

    /// Positional reads: a missing argument points at the call, an unreadable one at its value.
    mod positional_arg {
        use super::*;

        #[rstest]
        fn reads_a_present_argument_with_its_span() -> Result<(), TestError> {
            let call = EvaluatedCall::new(CALL_SPAN)
                .with_positional(Value::string(PRESENT_VALUE, ARGUMENT_SPAN));

            let arg = positional_arg::<String>(&call, FIRST_POSITION)?;

            assert_eq!(arg.item, PRESENT_VALUE);
            assert_eq!(arg.span, ARGUMENT_SPAN);

            Ok(())
        }

        #[rstest]
        #[case::first(FIRST_POSITION)]
        #[case::second(SECOND_POSITION)]
        fn reports_a_missing_argument_at_the_call(
            #[case] position: usize,
        ) -> Result<(), TestError> {
            let call = EvaluatedCall::new(CALL_SPAN);

            let err = positional_arg::<String>(&call, position)
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert_matches!(&err, CommandError::InvalidArgument { span, .. } if *span == CALL_SPAN);

            assert_eq!(
                err.into_labeled(Span::test_data()).code.as_deref(),
                Some(INVALID_ARGUMENT_CODE)
            );

            Ok(())
        }

        #[rstest]
        fn reports_an_unreadable_argument_at_its_value() -> Result<(), TestError> {
            let call = EvaluatedCall::new(CALL_SPAN)
                .with_positional(Value::int(UNREADABLE_VALUE, ARGUMENT_SPAN));

            let err = positional_arg::<String>(&call, FIRST_POSITION)
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert_matches!(&err, CommandError::InvalidArgument { span, .. } if *span == ARGUMENT_SPAN);

            assert_eq!(
                err.into_labeled(Span::test_data()).code.as_deref(),
                Some(INVALID_ARGUMENT_CODE)
            );

            Ok(())
        }
    }

    /// Flag reads: an absent flag is `None`, an unreadable one points at the flag.
    mod flag_arg {
        use super::*;

        #[rstest]
        fn reads_an_absent_flag_as_none() -> Result<(), TestError> {
            let call = EvaluatedCall::new(CALL_SPAN);
            let flag = flag_arg::<String>(&call, EXPECT_FLAG)?;

            assert!(flag.is_none(), "got {flag:?}");

            Ok(())
        }

        #[rstest]
        fn reads_a_present_flag_as_its_value() -> Result<(), TestError> {
            let call = EvaluatedCall::new(CALL_SPAN).with_named(
                Spanned {
                    item: EXPECT_FLAG,
                    span: FLAG_SPAN,
                },
                Value::string(PRESENT_VALUE, ARGUMENT_SPAN),
            );

            let flag = flag_arg::<String>(&call, EXPECT_FLAG)?
                .ok_or_else(|| TestError::Failure("expected a flag value".to_owned()))?;

            assert_eq!(flag.item, PRESENT_VALUE);

            Ok(())
        }

        #[rstest]
        #[case::expect(EXPECT_FLAG)]
        #[case::message(MESSAGE_FLAG)]
        fn reports_an_unreadable_flag_as_invalid_argument_at_the_flag(
            #[case] flag: &str,
        ) -> Result<(), TestError> {
            let call = EvaluatedCall::new(CALL_SPAN).with_named(
                Spanned {
                    item: flag,
                    span: FLAG_SPAN,
                },
                Value::int(UNREADABLE_VALUE, Span::test_data()),
            );

            let err = flag_arg::<String>(&call, flag)
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert_matches!(&err, CommandError::InvalidArgument { span, .. } if *span == FLAG_SPAN);

            assert_eq!(
                err.into_labeled(Span::test_data()).code.as_deref(),
                Some(INVALID_ARGUMENT_CODE)
            );

            Ok(())
        }
    }

    mod switch_arg {
        use super::*;

        #[rstest]
        fn reads_an_absent_switch_as_false() -> Result<(), TestError> {
            let call = EvaluatedCall::new(CALL_SPAN);
            let set = switch_arg(&call, ALL_SWITCH)?;

            assert!(!set, "got {set:?}");

            Ok(())
        }

        #[rstest]
        fn reads_a_present_switch_as_true() -> Result<(), TestError> {
            let call = EvaluatedCall::new(CALL_SPAN).with_flag(Spanned {
                item: ALL_SWITCH,
                span: FLAG_SPAN,
            });
            let set = switch_arg(&call, ALL_SWITCH)?;

            assert!(set, "got {set:?}");

            Ok(())
        }

        #[rstest]
        fn reports_an_unreadable_switch_at_the_flag() -> Result<(), TestError> {
            let call = EvaluatedCall::new(CALL_SPAN).with_named(
                Spanned {
                    item: ALL_SWITCH,
                    span: FLAG_SPAN,
                },
                Value::int(UNREADABLE_VALUE, Span::test_data()),
            );

            let err = switch_arg(&call, ALL_SWITCH)
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert_matches!(&err, CommandError::InvalidArgument { span, .. } if *span == FLAG_SPAN);

            Ok(())
        }
    }

    /// Optional positional reads: an absent argument is `None`, an unreadable one points at the argument.
    mod optional_arg {
        use super::*;

        #[rstest]
        fn reads_an_absent_argument_as_none() -> Result<(), TestError> {
            let call = EvaluatedCall::new(CALL_SPAN);
            let arg = optional_arg::<String>(&call, FIRST_POSITION)?;

            assert!(arg.is_none(), "got {arg:?}");

            Ok(())
        }

        #[rstest]
        fn reads_a_present_argument_with_its_span() -> Result<(), TestError> {
            let call = EvaluatedCall::new(CALL_SPAN)
                .with_positional(Value::string(PRESENT_VALUE, ARGUMENT_SPAN));

            let arg = optional_arg::<String>(&call, FIRST_POSITION)?
                .ok_or_else(|| TestError::Failure("expected an argument value".to_owned()))?;

            assert_eq!(arg.item, PRESENT_VALUE);
            assert_eq!(arg.span, ARGUMENT_SPAN);

            Ok(())
        }

        #[rstest]
        fn reports_an_unreadable_argument_at_its_value() -> Result<(), TestError> {
            let call = EvaluatedCall::new(CALL_SPAN)
                .with_positional(Value::int(UNREADABLE_VALUE, ARGUMENT_SPAN));

            let err = optional_arg::<String>(&call, FIRST_POSITION)
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert_matches!(&err, CommandError::InvalidArgument { span, .. } if *span == ARGUMENT_SPAN);

            assert_eq!(
                err.into_labeled(Span::test_data()).code.as_deref(),
                Some(INVALID_ARGUMENT_CODE)
            );

            Ok(())
        }
    }

    /// Exact-span checks of the revision guard, whose failures never reach a script.
    mod non_blank_revision {
        use super::*;

        #[rstest]
        #[case::empty("")]
        #[case::whitespace(" \t\n")]
        fn rejects_a_blank_revision_at_its_span(#[case] input: &str) -> Result<(), TestError> {
            let spec = Spanned {
                item: input.to_owned(),
                span: ARGUMENT_SPAN,
            };

            let err = non_blank_revision(spec)
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert_matches!(
                &err,
                CommandError::EmptyRevision { span } if *span == ARGUMENT_SPAN,
                "got {err:?}"
            );

            Ok(())
        }
    }
}
