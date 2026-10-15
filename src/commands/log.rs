//! `gix log`: stream the commits of an opened repository as a Nushell `ListStream`.

use crate::{
    cdt::Repository,
    git::{self, CommitInfo, GixError},
};

use nu_plugin::{EngineInterface, EvaluatedCall, PluginCommand};
use nu_protocol::{
    Category, LabeledError, ListStream, PipelineData, Record, ShellError, Signature, Span,
    SyntaxShape, Type, Value, record,
};

use super::{super::GixPlugin, CommandError};

const LIMIT_FLAG: &str = "limit";

/// Streams commits reachable from HEAD of a `git repository` value, so large histories are never fully buffered.
#[derive(Clone, Copy, Debug)]
pub struct Log;

impl PluginCommand for Log {
    type Plugin = GixPlugin;

    fn name(&self) -> &'static str {
        "gix log"
    }

    fn description(&self) -> &'static str {
        "List the commits of a repository opened with `gix open`, newest first."
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .named(
                LIMIT_FLAG,
                SyntaxShape::Int,
                "maximum number of commits to return",
                Some('n'),
            )
            .input_output_type(Type::Custom("git repository".into()), Type::table())
            .category(Category::Custom("gix".into()))
    }

    fn run(
        &self,
        plugin: &GixPlugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        let span = call.head;
        let input = input.into_value(span)?;
        let repo = repository_from(&input)?;

        let limit = call
            .get_flag::<i64>(LIMIT_FLAG)?
            .map(parse_limit)
            .transpose()
            .map_err(|e| e.into_labeled(span))?;

        let snapshot = plugin
            .registry()
            .acquire(repo.id, &repo.location())
            .map_err(|e| CommandError::from(e).into_labeled(span))?;

        let commits =
            git::log(snapshot, limit).map_err(|e| CommandError::from(e).into_labeled(span))?;
        let values = commits.map(move |commit| commit_value(commit, span));

        Ok(ListStream::new(values, span, engine.signals().clone()).into())
    }
}

/// Extracts the repository handle from pipeline input.
fn repository_from(input: &Value) -> Result<&Repository, LabeledError> {
    input
        .as_custom_value()
        .ok()
        .and_then(|custom| custom.as_any().downcast_ref::<Repository>())
        .ok_or_else(|| {
            LabeledError::new("expected a `git repository` value")
                .with_label(format!("got {}", input.get_type()), input.span())
                .with_help("pipe the output of `gix open` into this command")
        })
}

fn parse_limit(limit: i64) -> Result<usize, CommandError> {
    usize::try_from(limit).map_err(|_not_unsigned| CommandError::NegativeLimit(limit))
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
        "hash" => Value::string(commit.id, span),
        "author" => Value::string(commit.author, span),
        "email" => Value::string(commit.email, span),
        "date" => Value::date(commit.time, span),
        "message" => Value::string(commit.summary, span),
    };

    Value::record(record, span)
}

#[cfg(test)]
mod tests {
    use super::*;

    use nu_protocol::ShellError;
    use rstest::rstest;

    use std::path::PathBuf;

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Shell(#[from] ShellError),

        #[error(transparent)]
        Command(#[from] CommandError),

        #[error("Test failure: {0}")]
        Failure(String),
    }

    mod parse_limit {
        use super::*;

        #[rstest]
        #[case::zero(0, 0)]
        #[case::positive(25, 25)]
        fn accepts_non_negative(
            #[case] input: i64,
            #[case] expected: usize,
        ) -> Result<(), TestError> {
            assert_eq!(parse_limit(input)?, expected);

            Ok(())
        }

        #[rstest]
        fn rejects_negative() {
            assert!(matches!(
                parse_limit(-1),
                Err(CommandError::NegativeLimit(-1))
            ));
        }
    }

    mod repository_from {
        use super::*;

        #[rstest]
        fn accepts_gix_repository() -> Result<(), TestError> {
            let handle = Repository {
                id: 1,
                git_dir: PathBuf::from("/x/.git"),
                worktree: None,
                path: PathBuf::from("/tmp/r"),
            };

            let input = handle.clone().into_value(Span::test_data());
            let found = repository_from(&input).map_err(|e| TestError::Failure(e.to_string()))?;

            assert_eq!(found, &handle);

            Ok(())
        }

        #[rstest]
        fn rejects_other_values() -> Result<(), TestError> {
            let input = Value::int(1, Span::test_data());

            let err = repository_from(&input)
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert_eq!(err.msg, "expected a `git repository` value");

            Ok(())
        }
    }

    mod commit_value {
        use super::*;

        #[rstest]
        fn turns_errors_into_error_values() {
            let value = commit_value(
                Err(GixError::Timestamp {
                    id: "abc".to_owned(),
                }),
                Span::test_data(),
            );

            assert!(matches!(value, Value::Error { .. }));
        }
    }

    mod commit_record {
        use super::*;

        #[rstest]
        fn has_expected_columns() -> Result<(), TestError> {
            let time = chrono::DateTime::from_timestamp(0, 0)
                .ok_or_else(|| TestError::Failure("bad timestamp".to_owned()))?
                .fixed_offset();

            let commit = CommitInfo {
                id: "abc".to_owned(),
                author: "Ada".to_owned(),
                email: "ada@example.com".to_owned(),
                time,
                summary: "msg".to_owned(),
            };

            let value = commit_record(commit, Span::test_data());
            let columns: Vec<&String> = value.as_record()?.columns().collect();

            assert_eq!(columns, ["hash", "author", "email", "date", "message"]);

            Ok(())
        }
    }
}
