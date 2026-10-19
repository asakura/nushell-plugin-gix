//! `gix log`: stream the commits of an opened repository as a Nushell `ListStream`.

use crate::GixPlugin;
use crate::cdt::{Handle, Repository};
use crate::commands::{CATEGORY, CommandError, acquire_handle, flag_arg, handle_from, input_value};

use nu_plugin::{EngineInterface, EvaluatedCall, PluginCommand};
use nu_plugin_gix_core::{self as git, CommitInfo, GixError, Registry};
use nu_protocol::{
    Category, Example, LabeledError, ListStream, PipelineData, Record, ShellError, Signals,
    Signature, Span, Spanned, SyntaxShape, Type, Value, record,
};

const COMMAND_NAME: &str = "gix log";
const LIMIT_FLAG: &str = "limit";
const HASH_COLUMN: &str = "hash";
const AUTHOR_COLUMN: &str = "author";
const EMAIL_COLUMN: &str = "email";
const DATE_COLUMN: &str = "date";
/// Holds only the first line of the commit message.
const SUMMARY_COLUMN: &str = "summary";

/// Streams commits reachable from HEAD of a `git repository` value, so large histories are never fully buffered.
#[derive(Clone, Copy, Debug)]
pub struct Log;

impl PluginCommand for Log {
    type Plugin = GixPlugin;

    fn name(&self) -> &'static str {
        COMMAND_NAME
    }

    fn description(&self) -> &'static str {
        "List the commits reachable from HEAD of a repository opened with `gix open`, newest first by commit time."
    }

    fn extra_description(&self) -> &'static str {
        "Each row's `summary` is only the first line of the commit message.\n\
        Error codes: gix::wrong-input, gix::invalid-input, gix::invalid-argument, \
        gix::negative-limit, gix::open-failed, gix::repository-gone, gix::bad-revision, \
        gix::unborn-head, gix::walk-failed, gix::commit-unreadable, gix::bad-timestamp, gix::io"
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "gix open | gix log",
                description: "List the commits reachable from HEAD of the current repository, newest first",
                result: None,
            },
            Example {
                example: "gix open | gix log --limit 10",
                description: "List the ten most recent commits",
                result: None,
            },
        ]
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .named(
                LIMIT_FLAG,
                SyntaxShape::Int,
                "maximum number of commits to return",
                Some('n'),
            )
            .input_output_type(
                Type::Custom(<Repository as Handle>::TYPE_NAME.into()),
                Type::table(),
            )
            .category(Category::Custom(CATEGORY.into()))
    }

    fn run(
        &self,
        plugin: &GixPlugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        log_call(plugin.registry(), engine, call, input).map_err(|e| e.into_labeled(call.head))
    }
}

/// Reads the pipeline input, `--limit` and the engine signals from `call`, then streams the commits they select.
fn log_call(
    registry: &Registry,
    engine: &EngineInterface,
    call: &EvaluatedCall,
    input: PipelineData,
) -> Result<PipelineData, CommandError> {
    let span = call.head;
    let input = input_value(input, span)?;
    let repo = handle_from::<Repository>(&input)?;
    let limit = flag_arg::<i64>(call, LIMIT_FLAG)?
        .map(parse_limit)
        .transpose()?;

    let stream = run_log(
        registry,
        repo,
        limit,
        span,
        input.span(),
        engine.signals().clone(),
    )?;

    Ok(stream.into())
}

/// Streams the commits reachable from HEAD of the repository `repository` names, newest first, at most `limit` of them.
///
/// The registry supplies the opened repository, rebuilding it from the handle's location when it is not cached.
/// A handle whose repository is gone is reported at `handle_span`, the value that carried it.
/// Each commit becomes a row spanning `span`; the walk runs in the background and is bounded by the stream.
fn run_log(
    registry: &Registry,
    repository: &Repository,
    limit: Option<usize>,
    span: Span,
    handle_span: Span,
    signals: Signals,
) -> Result<ListStream, CommandError> {
    let opened = acquire_handle(registry, repository, handle_span)?;
    let commits = git::log(opened, limit)?;
    let values = commits.map(move |commit| commit_value(commit, span));

    Ok(ListStream::new(values, span, signals))
}

/// Converts the requested limit; negative values are rejected at the span of the value, and a
/// value beyond `usize::MAX` saturates to it, which the walk reads as no limit.
fn parse_limit(limit: Spanned<i64>) -> Result<usize, CommandError> {
    if limit.item.is_negative() {
        Err(CommandError::NegativeLimit {
            limit: limit.item,
            span: limit.span,
        })
    } else {
        Ok(usize::try_from(limit.item).unwrap_or(usize::MAX))
    }
}

/// Maps one streamed commit to a table row, or to an error value that ends the stream.
fn commit_value(commit: Result<CommitInfo, GixError>, span: Span) -> Value {
    match commit {
        Ok(commit) => commit_record(commit, span),
        Err(err) => Value::error(
            ShellError::LabeledError(Box::new(CommandError::from(err).into_labeled(span))),
            span,
        ),
    }
}

fn commit_record(commit: CommitInfo, span: Span) -> Value {
    let record: Record = record! {
        HASH_COLUMN => Value::string(commit.id, span),
        AUTHOR_COLUMN => Value::string(commit.author, span),
        EMAIL_COLUMN => Value::string(commit.email, span),
        DATE_COLUMN => Value::date(commit.time, span),
        SUMMARY_COLUMN => Value::string(commit.summary, span),
    };

    Value::record(record, span)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fixtures::{RepoInputError, plugin_test, repo_input};

    use miette::Diagnostic as _;
    use nu_plugin_gix_testing_support::{
        AssertionError, FixtureError, HEAD_FILE, TestRepo, UnbornTestRepo, assert_label_in_script,
        eval_labeled_error, eval_value, first_row, record_column, test_repo, unborn_repo,
    };
    use nu_plugin_test_support::PluginTest;
    use rstest::rstest;

    use std::{assert_matches, fs, io};

    const EXPECTED_ERROR: &str = "expected an error";
    const NEGATIVE_LIMIT: i64 = -1;
    const NEGATIVE_LIMIT_CODE: &str = "gix::negative-limit";
    const NEGATIVE_LIMIT_SCRIPT: &str = "gix log --limit -1";
    const LIMIT_SCRIPT: &str = "gix log --limit 1";
    const LIMITED_ROW_COUNT: usize = 1;
    const UNBORN_HEAD_CODE: &str = "gix::unborn-head";
    const BAD_TIMESTAMP_CODE: &str = "gix::bad-timestamp";
    const BAD_TIMESTAMP_ID: &str = "abc";
    const LIMIT_SHORT_FLAG: char = 'n';
    const LIMIT_VALUE_SPAN: Span = Span { start: 10, end: 12 };
    const ERROR_SPAN: Span = Span { start: 40, end: 47 };
    const HANDLE_SPAN: Span = Span { start: 21, end: 27 };
    const ROW_COLUMNS: [&str; 5] = [
        HASH_COLUMN,
        AUTHOR_COLUMN,
        EMAIL_COLUMN,
        DATE_COLUMN,
        SUMMARY_COLUMN,
    ];

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Shell(#[from] Box<ShellError>),

        #[error(transparent)]
        Assertion(#[from] AssertionError),

        #[error(transparent)]
        Fixture(#[from] FixtureError),

        #[error(transparent)]
        Input(#[from] RepoInputError),

        #[error(transparent)]
        Git(#[from] GixError),

        #[error(transparent)]
        Io(#[from] io::Error),

        #[error("Test failure: {0}")]
        Failure(String),
    }

    impl From<ShellError> for TestError {
        fn from(error: ShellError) -> Self {
            Self::Shell(Box::new(error))
        }
    }

    mod signature {
        use super::*;

        #[rstest]
        fn declares_the_name_limit_flag_and_repository_input() {
            let signature = Log.signature();
            let limit_short = signature
                .get_long_flag(LIMIT_FLAG)
                .and_then(|flag| flag.short);

            assert_eq!(signature.name, COMMAND_NAME);
            assert_eq!(limit_short, Some(LIMIT_SHORT_FLAG));
            assert!(
                signature.input_output_types.contains(&(
                    Type::Custom(<Repository as Handle>::TYPE_NAME.into()),
                    Type::table(),
                )),
                "got {:?}",
                signature.input_output_types
            );
        }
    }

    mod log {
        use super::*;

        mod rows {
            use super::*;

            #[rstest]
            fn expose_the_documented_columns_with_a_date(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;

                let table = eval_value(&mut plugin, COMMAND_NAME, repo_input(repo.path())?)?;
                let row = first_row(&table)?;
                let record = row.as_record()?;
                let columns: Vec<&str> = record.columns().map(String::as_str).collect();
                let date = record_column(row, DATE_COLUMN)?;

                assert_eq!(columns, ROW_COLUMNS);
                assert_matches!(date, Value::Date { .. });

                Ok(())
            }
        }

        mod limit {
            use super::*;

            #[rstest]
            fn forwards_the_flag_value_so_only_one_row_comes_back(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;

                let table = eval_value(&mut plugin, LIMIT_SCRIPT, repo_input(repo.path())?)?;

                assert_eq!(table.as_list()?.len(), LIMITED_ROW_COUNT);

                Ok(())
            }
        }

        mod errors {
            use super::*;

            #[rstest]
            fn fail_with_a_code_and_a_label_inside_the_script(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;

                let labeled = eval_labeled_error(
                    &mut plugin,
                    NEGATIVE_LIMIT_SCRIPT,
                    repo_input(repo.path())?,
                )?;

                assert_eq!(labeled.code.as_deref(), Some(NEGATIVE_LIMIT_CODE));
                assert_label_in_script!(&labeled, NEGATIVE_LIMIT_SCRIPT);

                Ok(())
            }

            #[rstest]
            fn reports_an_unborn_head_with_its_code(
                plugin_test: Result<PluginTest, FixtureError>,
                unborn_repo: Result<UnbornTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = unborn_repo?;

                let error = plugin
                    .eval_with(COMMAND_NAME, repo_input(repo.path())?)
                    .err()
                    .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.into()))?;
                let code = error.code().map(|code| code.to_string());

                assert_eq!(code.as_deref(), Some(UNBORN_HEAD_CODE));

                Ok(())
            }
        }
    }

    mod commit_value {
        use super::*;

        #[rstest]
        fn turns_a_bad_timestamp_into_an_error_value_with_code_span_and_message()
        -> Result<(), TestError> {
            let commit = Err(GixError::Timestamp {
                id: BAD_TIMESTAMP_ID.to_owned(),
            });
            let value = commit_value(commit, ERROR_SPAN);

            let Value::Error { error, .. } = &value else {
                return Err(TestError::Failure(format!(
                    "expected an error value, got {value:?}"
                )));
            };
            let code = error.code().map(|code| code.to_string());

            assert_eq!(code.as_deref(), Some(BAD_TIMESTAMP_CODE));
            assert_eq!(value.span(), ERROR_SPAN);
            assert!(!error.to_string().is_empty(), "the error has no message");

            Ok(())
        }
    }

    mod parse_limit {
        use super::*;

        #[rstest]
        fn rejects_a_negative_limit_at_its_value() -> Result<(), TestError> {
            let limit = Spanned {
                item: NEGATIVE_LIMIT,
                span: LIMIT_VALUE_SPAN,
            };

            let err = parse_limit(limit)
                .err()
                .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            assert_matches!(
                &err,
                CommandError::NegativeLimit { span, .. } if *span == LIMIT_VALUE_SPAN,
                "got {err:?}"
            );

            Ok(())
        }
    }

    mod run_log {
        use super::*;

        /// The handle failure `run_log` reports for a broken repository.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum HandleFailure {
            RepositoryGone,
            OpenFailed,
        }

        /// How a test breaks the repository behind a handle.
        #[derive(Clone, Copy, Debug)]
        enum Breakage {
            /// Deletes the repository while the registry still caches it.
            RemoveRepository,
            /// Deletes HEAD, so a fresh registry cannot rebuild the repository.
            RemoveHead,
        }

        #[rstest]
        #[case::repository_gone(Breakage::RemoveRepository, HandleFailure::RepositoryGone)]
        #[case::unopenable_rebuild(Breakage::RemoveHead, HandleFailure::OpenFailed)]
        fn reports_a_broken_handle_at_the_handle_span(
            #[case] breakage: Breakage,
            #[case] expected: HandleFailure,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = test_repo?;
            let registry = Registry::new();
            let (id, location) = registry.discover_and_register(repo.path())?;
            let handle = Repository::new(id, location);

            let lookup = match breakage {
                Breakage::RemoveRepository => {
                    fs::remove_dir_all(repo.path())?;
                    registry
                }
                Breakage::RemoveHead => {
                    fs::remove_file(handle.git_dir.join(HEAD_FILE))?;
                    Registry::new()
                }
            };

            let err = run_log(
                &lookup,
                &handle,
                None,
                Span::test_data(),
                HANDLE_SPAN,
                Signals::EMPTY,
            )
            .err()
            .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.into()))?;

            let (failure, span) = match &err {
                CommandError::HandleRepositoryGone { span, .. } => {
                    (HandleFailure::RepositoryGone, *span)
                }
                CommandError::HandleOpenFailed { span, .. } => (HandleFailure::OpenFailed, *span),
                other => return Err(TestError::Failure(format!("got {other:?}"))),
            };

            assert_eq!((failure, span), (expected, HANDLE_SPAN));

            Ok(())
        }
    }
}
