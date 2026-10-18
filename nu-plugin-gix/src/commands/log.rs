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
        COMMITTER_EMAIL, COMMITTER_NAME, FIRST_COMMIT_MESSAGE, FORK_FIRST_PARENT_MESSAGE,
        FORK_MERGE_MESSAGE, FORK_ROOT_MESSAGE, FORK_SECOND_PARENT_MESSAGE, FixtureError,
        ForkedTestRepo, HEAD_FILE, SECOND_COMMIT_MESSAGE, TestRepo, UnbornTestRepo, forked_repo,
        test_repo, unborn_repo,
    };
    use nu_plugin_test_support::PluginTest;
    use rstest::rstest;

    use std::{assert_matches, fs, io, path::Path};

    const HISTORY_COMMITS: usize = 2;
    const EXPECTED_ROW: &str = "expected a commit row";
    const EXPECTED_ERROR: &str = "expected an error";
    const NEGATIVE_LIMIT: i64 = -1;
    const NEGATIVE_LIMIT_CODE: &str = "gix::negative-limit";
    const UNBORN_HEAD_CODE: &str = "gix::unborn-head";
    const BAD_TIMESTAMP_CODE: &str = "gix::bad-timestamp";
    const LIMIT_VALUE_SPAN: Span = Span { start: 10, end: 12 };
    const ERROR_SPAN: Span = Span { start: 40, end: 47 };

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Shell(#[from] Box<ShellError>),

        #[error(transparent)]
        Fixture(#[from] FixtureError),

        #[error(transparent)]
        Input(#[from] RepoInputError),

        #[error(transparent)]
        Command(#[from] Box<CommandError>),

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

    impl From<CommandError> for TestError {
        fn from(error: CommandError) -> Self {
            Self::Command(Box::new(error))
        }
    }

    /// The value of one row column.
    fn field<'row>(row: &'row Value, name: &str) -> Result<&'row Value, TestError> {
        row.as_record()?
            .get(name)
            .ok_or_else(|| TestError::Failure(format!("row lacks a `{name}` column")))
    }

    /// The string value of one row column.
    fn column<'row>(row: &'row Value, name: &str) -> Result<&'row str, TestError> {
        Ok(field(row, name)?.as_str()?)
    }

    /// The string values of one column across `rows`, in row order.
    fn column_values<'row>(rows: &'row [Value], name: &str) -> Result<Vec<&'row str>, TestError> {
        rows.iter().map(|row| column(row, name)).collect()
    }

    /// The RFC 3339 rendering of the date column of `row`, which keeps the commit's own offset.
    fn date_of(row: &Value) -> Result<String, TestError> {
        Ok(field(row, DATE_COLUMN)?.as_date()?.to_rfc3339())
    }

    /// The error inside an error value, or a failure when `value` is not one.
    fn error_of(value: &Value) -> Result<&ShellError, TestError> {
        let Value::Error { error, .. } = value else {
            return Err(TestError::Failure(format!(
                "expected an error value, got {value:?}"
            )));
        };

        Ok(error.as_ref())
    }

    /// End-to-end behavior of `gix log` evaluated through `PluginTest`.
    mod log {
        use super::*;

        const ABOVE_HISTORY_LIMIT: i64 = 25;
        const SINGLE_COMMIT_LIMIT: i64 = 1;
        const CAPPED_FORK_LIMIT: i64 = 2;
        const NEGATIVE_LIMIT_MESSAGE: &str = "limit must not be negative, got -1";
        const NEGATIVE_LIMIT_LABEL: &str = "expected a non-negative integer";
        /// The `-1` of `gix log --limit -1`, as byte offsets into that script.
        const NEGATIVE_LIMIT_SPAN: Span = Span { start: 16, end: 18 };
        const SHORT_LIMIT_FLAG: &str = "-n";
        const NON_HANDLE_SCRIPT: &str = "1 | gix log";
        const INPUT_TYPE_MISMATCH_CODE: &str = "nu::parser::input_type_mismatch";
        /// The root of `forked_repo`, which carries the two-commit fixture's committer time in UTC.
        const ROOT_DATE: &str = "2023-11-14T22:13:20+00:00";
        /// The merge of `forked_repo`, whose author time is 1700000300 in a `-05:00` offset.
        const MERGE_DATE: &str = "2023-11-14T17:18:20-05:00";
        const ROW_COLUMNS: [&str; 5] = [
            HASH_COLUMN,
            AUTHOR_COLUMN,
            EMAIL_COLUMN,
            DATE_COLUMN,
            SUMMARY_COLUMN,
        ];

        fn log_script(limit: Option<i64>) -> String {
            limit.map_or_else(
                || COMMAND_NAME.to_owned(),
                |limit| format!("{COMMAND_NAME} --{LIMIT_FLAG} {limit}"),
            )
        }

        fn eval_table(
            plugin: &mut PluginTest,
            script: &str,
            repo: &Path,
        ) -> Result<Value, TestError> {
            let result = plugin
                .eval_with(script, repo_input(repo)?)?
                .into_value(Span::test_data())?;

            Ok(result)
        }

        /// The only row of a single-row table, or a failure naming the expectation.
        fn single_row<'row>(
            table: &'row Value,
            expectation: &str,
        ) -> Result<&'row Value, TestError> {
            table
                .as_list()?
                .first()
                .ok_or_else(|| TestError::Failure(expectation.into()))
        }

        /// The last row of a table, which is the oldest commit of the history.
        fn oldest_row<'table>(
            table: &'table Value,
            expectation: &str,
        ) -> Result<&'table Value, TestError> {
            table
                .as_list()?
                .last()
                .ok_or_else(|| TestError::Failure(expectation.into()))
        }

        /// The error that `gix log --limit -1` raises, or a failure when the negative limit is accepted.
        fn negative_limit_error(
            plugin: &mut PluginTest,
            repo: &TestRepo,
        ) -> Result<ShellError, TestError> {
            let error = plugin
                .eval_with(&log_script(Some(NEGATIVE_LIMIT)), repo_input(repo.path())?)
                .err()
                .ok_or_else(|| {
                    TestError::Failure("expected `gix log --limit -1` to be rejected".into())
                })?;

            Ok(error)
        }

        mod all_commits {
            use super::*;

            #[rstest]
            fn maps_each_commit_to_a_row(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;

                let repo = test_repo?;
                let table = eval_table(&mut plugin, &log_script(None), repo.path())?;
                let rows = table.as_list()?;
                let newest = single_row(&table, EXPECTED_ROW)?;

                assert_eq!(
                    column_values(rows, HASH_COLUMN)?,
                    [repo.head.as_str(), repo.parent.as_str()]
                );
                assert_eq!(
                    column_values(rows, SUMMARY_COLUMN)?,
                    [SECOND_COMMIT_MESSAGE, FIRST_COMMIT_MESSAGE]
                );
                assert_eq!(column(newest, AUTHOR_COLUMN)?, COMMITTER_NAME);
                assert_eq!(column(newest, EMAIL_COLUMN)?, COMMITTER_EMAIL);

                Ok(())
            }
        }

        mod oldest_row {
            use super::*;

            #[rstest]
            fn carries_the_first_commit(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;

                let repo = test_repo?;
                let table = eval_table(&mut plugin, &log_script(None), repo.path())?;
                let oldest = oldest_row(&table, EXPECTED_ROW)?;

                assert_eq!(column(oldest, AUTHOR_COLUMN)?, COMMITTER_NAME);
                assert_eq!(column(oldest, EMAIL_COLUMN)?, COMMITTER_EMAIL);
                assert_eq!(column(oldest, SUMMARY_COLUMN)?, FIRST_COMMIT_MESSAGE);

                Ok(())
            }
        }

        mod date {
            use super::*;

            #[rstest]
            fn carries_each_commit_time_as_a_date_with_its_offset(
                plugin_test: Result<PluginTest, FixtureError>,
                forked_repo: Result<ForkedTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;

                let repo = forked_repo?;
                let table = eval_table(&mut plugin, &log_script(None), repo.path())?;
                let newest = single_row(&table, EXPECTED_ROW)?;
                let oldest = oldest_row(&table, EXPECTED_ROW)?;

                assert_matches!(field(newest, DATE_COLUMN)?, Value::Date { .. });
                assert_eq!(date_of(newest)?, MERGE_DATE);
                assert_eq!(date_of(oldest)?, ROOT_DATE);

                Ok(())
            }
        }

        mod order {
            use super::*;

            #[rstest]
            fn lists_commits_by_commit_time_not_first_parent(
                plugin_test: Result<PluginTest, FixtureError>,
                forked_repo: Result<ForkedTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;

                let repo = forked_repo?;
                let table = eval_table(&mut plugin, &log_script(None), repo.path())?;

                assert_eq!(
                    column_values(table.as_list()?, SUMMARY_COLUMN)?,
                    [
                        FORK_MERGE_MESSAGE,
                        FORK_SECOND_PARENT_MESSAGE,
                        FORK_FIRST_PARENT_MESSAGE,
                        FORK_ROOT_MESSAGE,
                    ]
                );

                Ok(())
            }
        }

        mod columns {
            use super::*;

            #[rstest]
            fn expose_the_documented_column_set(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;

                let repo = test_repo?;
                let table = eval_table(&mut plugin, &log_script(None), repo.path())?;
                let newest = single_row(&table, EXPECTED_ROW)?;
                let got: Vec<&str> = newest.as_record()?.columns().map(String::as_str).collect();

                assert_eq!(got, ROW_COLUMNS);

                Ok(())
            }
        }

        mod limit {
            use super::*;

            #[rstest]
            #[case::zero(0, 0)]
            #[case::above_history(ABOVE_HISTORY_LIMIT, HISTORY_COMMITS)]
            #[case::beyond_usize(i64::MAX, HISTORY_COMMITS)]
            fn caps_the_history_to_the_requested_rows(
                #[case] limit: i64,
                #[case] expected: usize,
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;

                let repo = test_repo?;
                let table = eval_table(&mut plugin, &log_script(Some(limit)), repo.path())?;

                assert_eq!(table.as_list()?.len(), expected);

                Ok(())
            }

            #[rstest]
            fn keeps_the_newest_commit_when_capped(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;

                let repo = test_repo?;

                let table = eval_table(
                    &mut plugin,
                    &log_script(Some(SINGLE_COMMIT_LIMIT)),
                    repo.path(),
                )?;

                let newest = single_row(&table, EXPECTED_ROW)?;

                assert_eq!(column(newest, HASH_COLUMN)?, repo.head.as_str());
                assert_eq!(column(newest, SUMMARY_COLUMN)?, SECOND_COMMIT_MESSAGE);

                Ok(())
            }

            #[rstest]
            fn keeps_the_commits_newest_by_time_when_capped(
                plugin_test: Result<PluginTest, FixtureError>,
                forked_repo: Result<ForkedTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;

                let repo = forked_repo?;

                let table = eval_table(
                    &mut plugin,
                    &log_script(Some(CAPPED_FORK_LIMIT)),
                    repo.path(),
                )?;

                assert_eq!(
                    column_values(table.as_list()?, SUMMARY_COLUMN)?,
                    [FORK_MERGE_MESSAGE, FORK_SECOND_PARENT_MESSAGE]
                );

                Ok(())
            }

            #[rstest]
            fn accepts_the_short_limit_flag(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;

                let repo = test_repo?;
                let script = format!("{COMMAND_NAME} {SHORT_LIMIT_FLAG} {SINGLE_COMMIT_LIMIT}");
                let table = eval_table(&mut plugin, &script, repo.path())?;

                assert_eq!(table.as_list()?.len(), 1);

                Ok(())
            }
        }

        mod negative_limit {
            use super::*;

            #[rstest]
            fn fails_with_message(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;

                let repo = test_repo?;
                let error = negative_limit_error(&mut plugin, &repo)?;

                assert_eq!(error.to_string(), NEGATIVE_LIMIT_MESSAGE);

                Ok(())
            }

            #[rstest]
            fn fails_with_stable_code(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;

                let repo = test_repo?;
                let error = negative_limit_error(&mut plugin, &repo)?;
                let code = error.code().map(|code| code.to_string());

                assert_eq!(code.as_deref(), Some(NEGATIVE_LIMIT_CODE));

                Ok(())
            }

            #[rstest]
            fn labels_the_value_with_its_reason(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;

                let repo = test_repo?;
                let error = negative_limit_error(&mut plugin, &repo)?;

                let ShellError::LabeledError(labeled) = &error else {
                    return Err(TestError::Failure(format!(
                        "expected a labeled error, got {error:?}"
                    )));
                };

                let label = labeled
                    .labels
                    .first()
                    .ok_or_else(|| TestError::Failure("expected a label".into()))?;

                assert_eq!(label.text, NEGATIVE_LIMIT_LABEL);

                Ok(())
            }

            #[rstest]
            fn points_the_label_at_the_value(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;

                let repo = test_repo?;
                let error = negative_limit_error(&mut plugin, &repo)?;

                let ShellError::LabeledError(labeled) = &error else {
                    return Err(TestError::Failure(format!(
                        "expected a labeled error, got {error:?}"
                    )));
                };

                let label = labeled
                    .labels
                    .first()
                    .ok_or_else(|| TestError::Failure("expected a label".into()))?;

                // The engine gives the flag value the span of the whole `--limit -1` argument, so the
                // label covers the value without starting at it.
                assert_eq!(label.span.end, NEGATIVE_LIMIT_SPAN.end);

                assert!(
                    label.span.start <= NEGATIVE_LIMIT_SPAN.start,
                    "label {:?} does not cover the value",
                    label.span
                );

                Ok(())
            }
        }

        mod non_handle_input {
            use super::*;

            #[rstest]
            fn rejects_before_the_command_runs(
                plugin_test: Result<PluginTest, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;

                let err = plugin
                    .eval(NON_HANDLE_SCRIPT)
                    .err()
                    .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.into()))?;

                let ShellError::LabeledError(outer) = &err else {
                    return Err(TestError::Failure(format!(
                        "expected a labeled error, got {err:?}"
                    )));
                };

                let cause = outer
                    .inner
                    .first()
                    .ok_or_else(|| TestError::Failure("expected the engine's cause".into()))?;

                let code = cause.code().map(|code| code.to_string());

                assert_eq!(code.as_deref(), Some(INPUT_TYPE_MISMATCH_CODE));

                Ok(())
            }
        }

        mod unborn_head {
            use super::*;

            #[rstest]
            fn fails_with_stable_code(
                plugin_test: Result<PluginTest, FixtureError>,
                unborn_repo: Result<UnbornTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;

                let repo = unborn_repo?;

                let error = plugin
                    .eval_with(&log_script(None), repo_input(repo.path())?)
                    .err()
                    .ok_or_else(|| {
                        TestError::Failure(
                            "expected `gix log` on an empty repository to fail".into(),
                        )
                    })?;

                let code = error.code().map(|code| code.to_string());

                assert_eq!(code.as_deref(), Some(UNBORN_HEAD_CODE));

                Ok(())
            }
        }
    }

    /// Unit tests for the stream builder, which takes the registry and handle directly, so no engine or call is involved.
    mod run_log {
        use super::*;

        const CAPPED_LIMIT: usize = 1;
        const HANDLE_SPAN: Span = Span { start: 21, end: 27 };

        /// The handle `gix open` returns for the repository at `path`, registered in `registry`.
        fn handle_for(registry: &Registry, path: &Path) -> Result<Repository, GixError> {
            let (id, location) = registry.discover_and_register(path)?;

            Ok(Repository::new(id, location))
        }

        /// The rows `run_log` streams for `repository`, with the test span and no signals.
        fn log_rows(
            registry: &Registry,
            repository: &Repository,
            limit: Option<usize>,
        ) -> Result<Vec<Value>, CommandError> {
            let stream = run_log(
                registry,
                repository,
                limit,
                Span::test_data(),
                HANDLE_SPAN,
                Signals::EMPTY,
            )?;

            Ok(stream.into_iter().collect())
        }

        mod history {
            use super::*;

            #[rstest]
            fn lists_both_commits(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;
                let registry = Registry::new();
                let handle = handle_for(&registry, repo.path())?;
                let rows = log_rows(&registry, &handle, None)?;

                assert_eq!(rows.len(), HISTORY_COMMITS);

                Ok(())
            }

            #[rstest]
            fn lists_newest_commit_first(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;
                let registry = Registry::new();
                let handle = handle_for(&registry, repo.path())?;
                let rows = log_rows(&registry, &handle, None)?;
                let hashes = column_values(&rows, HASH_COLUMN)?;

                assert_eq!(hashes, [repo.head.as_str(), repo.parent.as_str()]);

                Ok(())
            }
        }

        mod limit {
            use super::*;

            #[rstest]
            #[case::unbounded(None, HISTORY_COMMITS)]
            #[case::capped(Some(CAPPED_LIMIT), CAPPED_LIMIT)]
            #[case::zero(Some(0), 0)]
            fn caps_rows_to_the_limit(
                #[case] limit: Option<usize>,
                #[case] expected: usize,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;
                let registry = Registry::new();
                let handle = handle_for(&registry, repo.path())?;

                assert_eq!(log_rows(&registry, &handle, limit)?.len(), expected);

                Ok(())
            }
        }

        mod unborn_head {
            use super::*;

            #[rstest]
            fn reports_unborn_head_as_git_error(
                unborn_repo: Result<UnbornTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = unborn_repo?;
                let registry = Registry::new();
                let handle = handle_for(&registry, repo.path())?;

                let err = log_rows(&registry, &handle, None).err().ok_or_else(|| {
                    TestError::Failure("expected an unborn HEAD to be rejected".into())
                })?;

                assert_matches!(
                    &err,
                    CommandError::Git(GixError::UnbornHead { .. }),
                    "got {err:?}"
                );

                Ok(())
            }
        }

        mod repository_gone {
            use super::*;

            #[rstest]
            fn reports_repository_gone_at_the_handle_span(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;
                let registry = Registry::new();
                let handle = handle_for(&registry, repo.path())?;

                fs::remove_dir_all(repo.path())?;

                let err = log_rows(&registry, &handle, None).err().ok_or_else(|| {
                    TestError::Failure("expected a deleted repository to be rejected".into())
                })?;

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
            fn reports_a_failed_rebuild_as_open_failure_at_the_handle(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;
                let handle = handle_for(&Registry::new(), repo.path())?;

                // A fresh registry must rebuild the repository, which fails without its HEAD file.
                fs::remove_file(handle.git_dir.join(HEAD_FILE))?;

                let err = log_rows(&Registry::new(), &handle, None)
                    .err()
                    .ok_or_else(|| {
                        TestError::Failure(
                            "expected a repository that cannot be rebuilt to be rejected".into(),
                        )
                    })?;

                assert_matches!(
                    &err,
                    CommandError::HandleOpenFailed { span, .. } if *span == HANDLE_SPAN,
                    "got {err:?}"
                );

                Ok(())
            }
        }
    }

    /// Unit tests for the limit conversion: non-negative values pass through unchanged, and a
    /// negative value is rejected at its own span.
    mod parse_limit {
        use super::*;

        #[rstest]
        #[case::zero(0, 0)]
        #[case::typical(1, 1)]
        fn accepts_non_negative_limits(
            #[case] item: i64,
            #[case] expected: usize,
        ) -> Result<(), TestError> {
            let limit = Spanned {
                item,
                span: Span::test_data(),
            };

            assert_eq!(parse_limit(limit)?, expected);

            Ok(())
        }

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

    /// Unit tests for mapping streamed commit results to values; a synthetic git-layer
    /// error cannot be produced through the command surface, so it stays a unit test.
    mod commit_value {
        use super::*;

        const UNREADABLE_COMMIT_ID: &str = "abc";
        const BAD_TIMESTAMP_MESSAGE: &str = "commit abc has an out-of-range timestamp";

        /// A commit whose timestamp cannot be represented, as the git layer reports it.
        fn unreadable_commit() -> GixError {
            GixError::Timestamp {
                id: UNREADABLE_COMMIT_ID.to_owned(),
            }
        }

        #[rstest]
        fn turns_a_bad_timestamp_into_an_error_value_with_its_code() -> Result<(), TestError> {
            let value = commit_value(Err(unreadable_commit()), Span::test_data());
            let code = error_of(&value)?.code().map(|code| code.to_string());

            assert_eq!(code.as_deref(), Some(BAD_TIMESTAMP_CODE));

            Ok(())
        }

        #[rstest]
        fn keeps_the_span_it_is_given() {
            let value = commit_value(Err(unreadable_commit()), ERROR_SPAN);

            assert_eq!(value.span(), ERROR_SPAN);
        }

        #[rstest]
        fn carries_the_timestamp_message() -> Result<(), TestError> {
            let value = commit_value(Err(unreadable_commit()), Span::test_data());

            assert_eq!(error_of(&value)?.to_string(), BAD_TIMESTAMP_MESSAGE);

            Ok(())
        }
    }
}
