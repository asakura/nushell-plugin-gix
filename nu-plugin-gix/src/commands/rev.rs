//! `gix rev`: resolve a revision expression to object metadata.

use crate::GixPlugin;
use crate::cdt::{Handle, Repository};
use crate::commands::{
    CATEGORY, CommandError, acquire_handle, flag_arg, handle_from, positional_arg,
};

use nu_plugin::{EngineInterface, EvaluatedCall, SimplePluginCommand};
use nu_plugin_gix_core::{self as git, GixError, Registry, RevInfo};
use nu_protocol::{
    Category, Example, LabeledError, Record, Signature, Span, Spanned, SyntaxShape, Type, Value,
    record,
};

const COMMAND_NAME: &str = "gix rev";
const SPEC_ARG: &str = "spec";
const SPEC_POSITION: usize = 0;
const SHORT_LEN_FLAG: &str = "short-len";
const SHORT_LEN_SHORT: char = 's';
const OID_COLUMN: &str = "oid";
const KIND_COLUMN: &str = "kind";
const TREE_COLUMN: &str = "tree";
const SHORT_COLUMN: &str = "short";

/// Sample values shown as the output of the command examples.
const EXAMPLE_OID: &str = "0123456789abcdef0123456789abcdef01234567";
const EXAMPLE_TREE: &str = "89abcdef0123456789abcdef0123456789abcdef";
const EXAMPLE_SHORT: &str = "0123456";
const EXAMPLE_LONG_SHORT: &str = "0123456789ab";
const EXAMPLE_COMMIT_KIND: &str = "commit";
const EXAMPLE_TREE_KIND: &str = "tree";

/// Resolves a revision expression against a repository handle from `gix open`.
#[derive(Clone, Copy, Debug)]
pub struct Rev;

impl SimplePluginCommand for Rev {
    type Plugin = GixPlugin;

    fn name(&self) -> &'static str {
        COMMAND_NAME
    }

    fn description(&self) -> &'static str {
        "Resolve a revision expression to its object id, kind, tree, and short id."
    }

    fn extra_description(&self) -> &'static str {
        "The `tree` column is null for blobs, which have no tree.\n\
        Error codes: gix::wrong-input, gix::invalid-argument, gix::invalid-revision, \
        gix::invalid-short-len, gix::open-failed, gix::repository-gone, gix::bad-revision"
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "gix open | gix rev HEAD",
                description: "Resolve HEAD of the current repository to its commit id, kind, tree, and short id",
                result: Some(example_rev(
                    EXAMPLE_OID,
                    EXAMPLE_COMMIT_KIND,
                    Some(EXAMPLE_TREE),
                    EXAMPLE_SHORT,
                )),
            },
            Example {
                example: "gix open | gix rev main --short-len 12",
                description: "Resolve the branch `main`, abbreviating its id to 12 hexadecimal characters",
                result: Some(example_rev(
                    EXAMPLE_OID,
                    EXAMPLE_COMMIT_KIND,
                    Some(EXAMPLE_TREE),
                    EXAMPLE_LONG_SHORT,
                )),
            },
            Example {
                example: "gix open | gix rev 'HEAD^{tree}'",
                description: "Resolve the tree that HEAD points to",
                result: Some(example_rev(
                    EXAMPLE_TREE,
                    EXAMPLE_TREE_KIND,
                    Some(EXAMPLE_TREE),
                    EXAMPLE_SHORT,
                )),
            },
        ]
    }

    fn signature(&self) -> Signature {
        Signature::build(COMMAND_NAME)
            .required(
                SPEC_ARG,
                SyntaxShape::String,
                "revision expression to resolve, e.g. `HEAD`, a branch, a tag, or an object id",
            )
            .named(
                SHORT_LEN_FLAG,
                SyntaxShape::Int,
                "number of hexadecimal characters in the `short` column",
                Some(SHORT_LEN_SHORT),
            )
            .input_output_type(
                Type::Custom(<Repository as Handle>::TYPE_NAME.into()),
                Type::record(),
            )
            .category(Category::Custom(CATEGORY.into()))
    }

    fn run(
        &self,
        plugin: &GixPlugin,
        _engine: &EngineInterface,
        call: &EvaluatedCall,
        input: &Value,
    ) -> Result<Value, LabeledError> {
        rev_call(plugin.registry(), call, input).map_err(|e| e.into_labeled(call.head))
    }
}

/// Reads the pipeline input and the arguments from `call`, then resolves the revision they name.
fn rev_call(
    registry: &Registry,
    call: &EvaluatedCall,
    input: &Value,
) -> Result<Value, CommandError> {
    let repo = handle_from::<Repository>(input)?;

    let short_len = flag_arg::<i64>(call, SHORT_LEN_FLAG)?
        .map(parse_short_len)
        .transpose()?;

    let spec = parse_spec(positional_arg::<String>(call, SPEC_POSITION)?)?;

    run_rev(registry, repo, &spec, short_len, call.head, input.span())
}

/// Resolves the revision `spec` in the repository `repository` names and renders it as a record.
///
/// The registry supplies the opened repository, rebuilding it when it is not cached. A handle whose
/// repository is gone or cannot be opened is reported at `handle_span`, the value that carried it.
/// A revision that does not resolve, or a short length the repository cannot honour, is reported at
/// the span of its own argument. The record spans `span`.
fn run_rev(
    registry: &Registry,
    repository: &Repository,
    spec: &Spanned<String>,
    short_len: Option<Spanned<usize>>,
    span: Span,
    handle_span: Span,
) -> Result<Value, CommandError> {
    let opened = acquire_handle(registry, repository, handle_span)?;
    let short_len_span = short_len.as_ref().map_or(spec.span, |len| len.span);

    let info = git::resolve(&opened, &spec.item, short_len.map(|len| len.item))
        .map_err(|source| resolve_failure(source, spec.span, short_len_span))?;

    Ok(rev_record(info, span))
}

/// Attributes a git failure to resolve the call's arguments: an out-of-range `--short-len` to
/// `short_len_span`, and anything else to the revision at `spec_span`.
///
/// The git layer reports an out-of-range short length only for a length it was given. When no
/// length was given the caller passes the revision's span as `short_len_span`, so the attribution
/// needs no optional span.
const fn resolve_failure(source: GixError, spec_span: Span, short_len_span: Span) -> CommandError {
    let span = match &source {
        GixError::ShortLen {
            len: _len,
            detail: _detail,
        } => short_len_span,
        _ => spec_span,
    };

    CommandError::Located { span, source }
}

/// Rejects blank or whitespace-only revision expressions at their own span.
fn parse_spec(spec: Spanned<String>) -> Result<Spanned<String>, CommandError> {
    if spec.item.trim().is_empty() {
        return Err(CommandError::EmptyRevision { span: spec.span });
    }

    Ok(spec)
}

/// Converts the requested short length; negative values are rejected at the span of the value.
/// A value beyond `usize::MAX` saturates to it, so the git layer reports it as an out-of-range
/// short length.
fn parse_short_len(len: Spanned<i64>) -> Result<Spanned<usize>, CommandError> {
    if len.item.is_negative() {
        Err(CommandError::NegativeShortLen {
            len: len.item,
            span: len.span,
        })
    } else {
        Ok(Spanned {
            item: usize::try_from(len.item).unwrap_or(usize::MAX),
            span: len.span,
        })
    }
}

/// A sample `{ oid, kind, tree, short }` record; a `None` tree renders as null.
fn example_rev(oid: &str, kind: &str, tree: Option<&str>, short: &str) -> Value {
    Value::test_record(record! {
        OID_COLUMN => Value::test_string(oid),
        KIND_COLUMN => Value::test_string(kind),
        TREE_COLUMN => tree.map_or_else(Value::test_nothing, Value::test_string),
        SHORT_COLUMN => Value::test_string(short),
    })
}

/// Maps a resolved revision to a `{ oid, kind, tree, short }` record; the tree is null for blobs.
fn rev_record(info: RevInfo, span: Span) -> Value {
    let tree = info
        .tree
        .map_or_else(|| Value::nothing(span), |tree| Value::string(tree, span));

    let record: Record = record! {
        OID_COLUMN => Value::string(info.oid, span),
        KIND_COLUMN => Value::string(info.kind.as_str(), span),
        TREE_COLUMN => tree,
        SHORT_COLUMN => Value::string(info.short, span),
    };

    Value::record(record, span)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fixtures::{RepoInputError, plugin_test, repo_input};

    use nu_plugin_gix_core::{RepositoryLocation, RevKind};
    use nu_plugin_gix_testing_support::{
        AssertionError, FixtureError, TestRepo, assert_label_in_script, eval_value, raised_labeled,
        record_column, test_repo,
    };
    use nu_plugin_test_support::PluginTest;
    use nu_protocol::ShellError;
    use rstest::rstest;

    use std::{assert_matches, fs, io, path::PathBuf};

    const HEAD_SPEC: &str = "HEAD";
    const UNKNOWN_SPEC: &str = "not-a-revision";
    const BLANK_SPEC: &str = "''";
    const SHORT_FLAG: &str = "--short-len";
    const NEGATIVE_LEN: i64 = -1;
    const TOO_SHORT_LEN: usize = 3;
    const REQUESTED_SHORT_LEN: usize = 8;
    const SHORT_LEN_DETAIL: &str = "expected 4..=40";
    const EMPTY_REVISION_FACT: &str = "must not be empty";
    const SHORT_LEN_FACT: &str = "short length";
    const UNREADABLE_POSITIONAL: i64 = 1;
    const UNREADABLE_SHORT_LEN: &str = "soon";
    const UNOPENED_GIT_DIR: &str = "unopened.git";
    const HANDLE_ID: u64 = 1;
    const EXPECTED_ERROR: &str = "expected an error";
    const EXPECTED_COLUMNS: [&str; 4] = [OID_COLUMN, KIND_COLUMN, TREE_COLUMN, SHORT_COLUMN];
    const FIXTURE_OID: &str = "abc";
    const FIXTURE_TREE: &str = "def";
    const FIXTURE_SHORT: &str = "ab";
    const INVALID_REVISION_CODE: &str = "gix::invalid-revision";
    const BAD_REVISION_CODE: &str = "gix::bad-revision";
    const INVALID_SHORT_LEN_CODE: &str = "gix::invalid-short-len";
    const REPOSITORY_GONE_CODE: &str = "gix::repository-gone";
    const CALL_SPAN: Span = Span { start: 1, end: 7 };
    const FLAG_SPAN: Span = Span { start: 9, end: 20 };
    const ARGUMENT_SPAN: Span = Span { start: 21, end: 25 };
    const REVISION_SPAN: Span = Span { start: 8, end: 12 };
    const SHORT_LEN_VALUE_SPAN: Span = Span { start: 25, end: 26 };
    const DANGLING_REF: &str = "refs/heads/loop";

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
        Io(#[from] io::Error),

        #[error("Test failure: {0}")]
        Failure(String),
    }

    impl From<ShellError> for TestError {
        fn from(error: ShellError) -> Self {
            Self::Shell(Box::new(error))
        }
    }

    /// The script `gix rev <args>` evaluated against the fixture input.
    fn rev_script(args: &str) -> String {
        format!("{COMMAND_NAME} {args}")
    }

    mod signature {
        use super::*;

        #[rstest]
        fn declares_the_name_short_len_flag_and_repository_input() {
            let signature = Rev.signature();

            let short_len_short = signature
                .get_long_flag(SHORT_LEN_FLAG)
                .and_then(|flag| flag.short);

            assert_eq!(signature.name, COMMAND_NAME);
            assert_eq!(short_len_short, Some(SHORT_LEN_SHORT));

            assert!(
                signature.input_output_types.contains(&(
                    Type::Custom(<Repository as Handle>::TYPE_NAME.into()),
                    Type::record(),
                )),
                "got {:?}",
                signature.input_output_types
            );
        }
    }

    /// End-to-end tests: `gix rev` driven through the plugin protocol against the shared fixtures.
    mod eval {
        use super::*;

        mod resolve {
            use super::*;

            #[rstest]
            fn resolves_head_to_the_fixture_commit(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;
                let script = rev_script(HEAD_SPEC);

                let value = eval_value(&mut plugin, &script, repo_input(repo.path())?)?;

                assert_eq!(
                    record_column(&value, OID_COLUMN)?.as_str()?,
                    repo.head.as_str()
                );

                Ok(())
            }

            #[rstest]
            fn shortens_the_id_to_the_requested_short_length(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;
                let script = rev_script(&format!("{HEAD_SPEC} {SHORT_FLAG} {REQUESTED_SHORT_LEN}"));

                let value = eval_value(&mut plugin, &script, repo_input(repo.path())?)?;
                let oid = record_column(&value, OID_COLUMN)?.as_str()?;
                let short = record_column(&value, SHORT_COLUMN)?.as_str()?;

                assert_eq!(short.chars().count(), REQUESTED_SHORT_LEN);

                assert!(
                    oid.starts_with(short),
                    "short `{short}` is not a prefix of `{oid}`"
                );

                Ok(())
            }
        }

        mod errors {
            use super::*;

            #[rstest]
            fn rejects_a_blank_revision_with_its_code_and_label(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;
                let script = rev_script(BLANK_SPEC);

                let labeled = raised_labeled(&mut plugin, &script, repo_input(repo.path())?)?;

                assert_eq!(labeled.code.as_deref(), Some(INVALID_REVISION_CODE));
                assert_label_in_script(&labeled, &script)?;

                assert!(
                    labeled.msg.contains(EMPTY_REVISION_FACT),
                    "unexpected message: {}",
                    labeled.msg
                );

                Ok(())
            }

            #[rstest]
            fn rejects_an_unresolvable_revision_with_its_code_and_label(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;
                let script = rev_script(UNKNOWN_SPEC);

                let labeled = raised_labeled(&mut plugin, &script, repo_input(repo.path())?)?;

                assert_eq!(labeled.code.as_deref(), Some(BAD_REVISION_CODE));
                assert_label_in_script(&labeled, &script)?;

                assert!(
                    labeled.msg.contains(UNKNOWN_SPEC),
                    "message lacks the revision: {}",
                    labeled.msg
                );

                Ok(())
            }

            #[rstest]
            #[case::negative("-1")]
            #[case::too_short("3")]
            #[case::too_long("9223372036854775807")]
            fn rejects_an_invalid_short_length_with_its_code_and_label(
                #[case] short_len: &str,
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;
                let script = rev_script(&format!("{HEAD_SPEC} {SHORT_FLAG} {short_len}"));

                let labeled = raised_labeled(&mut plugin, &script, repo_input(repo.path())?)?;

                assert_eq!(labeled.code.as_deref(), Some(INVALID_SHORT_LEN_CODE));
                assert_label_in_script(&labeled, &script)?;

                assert!(
                    labeled.msg.contains(SHORT_LEN_FACT),
                    "unexpected message: {}",
                    labeled.msg
                );

                Ok(())
            }

            #[rstest]
            fn reports_a_deleted_repository_with_its_code(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;
                let input = repo_input(repo.path())?;

                fs::remove_dir_all(repo.path())?;

                let labeled = raised_labeled(&mut plugin, &rev_script(HEAD_SPEC), input)?;

                assert_eq!(labeled.code.as_deref(), Some(REPOSITORY_GONE_CODE));
                assert!(!labeled.msg.is_empty(), "the error has no message");

                Ok(())
            }
        }
    }

    /// Exact-span checks of the argument parser, whose failures never reach a script.
    mod parse_spec {
        use super::*;

        #[rstest]
        #[case::empty("")]
        #[case::whitespace(" \t\n")]
        fn rejects_a_blank_revision_at_its_span(#[case] input: &str) -> Result<(), TestError> {
            let spec = Spanned {
                item: input.to_owned(),
                span: ARGUMENT_SPAN,
            };

            let err = parse_spec(spec)
                .err()
                .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            assert_matches!(
                &err,
                CommandError::EmptyRevision { span } if *span == ARGUMENT_SPAN,
                "got {err:?}"
            );

            Ok(())
        }
    }

    mod parse_short_len {
        use super::*;

        #[rstest]
        fn rejects_a_negative_length_at_its_value() -> Result<(), TestError> {
            let short_len = Spanned {
                item: NEGATIVE_LEN,
                span: ARGUMENT_SPAN,
            };

            let err = parse_short_len(short_len)
                .err()
                .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            assert_matches!(
                &err,
                CommandError::NegativeShortLen { span, .. } if *span == ARGUMENT_SPAN,
                "got {err:?}"
            );

            Ok(())
        }
    }

    /// Span attribution of git resolve failures: a bad `--short-len` at its value, anything else at
    /// the revision.
    mod resolve_failure {
        use super::*;

        #[rstest]
        #[case::out_of_range_short_length(
            GixError::ShortLen {
                len: TOO_SHORT_LEN,
                detail: SHORT_LEN_DETAIL.to_owned(),
            },
            SHORT_LEN_VALUE_SPAN
        )]
        #[case::other_failures_at_the_revision(
            GixError::DanglingRef {
                name: DANGLING_REF.to_owned(),
            },
            REVISION_SPAN
        )]
        fn points_a_resolve_failure_at_its_span(#[case] source: GixError, #[case] expected: Span) {
            let err = resolve_failure(source, REVISION_SPAN, SHORT_LEN_VALUE_SPAN);

            assert_matches!(
                &err,
                CommandError::Located { span, .. } if *span == expected,
                "got {err:?}"
            );
        }
    }

    /// Unit tests for the reads no script can make fail: the parser rejects a non-integer
    /// `--short-len` and a missing or non-string revision. Each fails before the repository is opened,
    /// so the handle needs no repository on disk.
    mod reads {
        use super::*;

        /// Pipeline input holding a handle that names no repository on disk.
        fn handle_input() -> Value {
            Repository::new(
                HANDLE_ID,
                RepositoryLocation {
                    git_dir: PathBuf::from(UNOPENED_GIT_DIR),
                    worktree: None,
                },
            )
            .into_value(Span::test_data())
        }

        #[rstest]
        #[case::unreadable_flag(
            EvaluatedCall::new(CALL_SPAN).with_named(
                Spanned {
                    item: SHORT_LEN_FLAG,
                    span: FLAG_SPAN,
                },
                Value::string(UNREADABLE_SHORT_LEN, Span::test_data()),
            ),
            FLAG_SPAN
        )]
        #[case::missing_revision(EvaluatedCall::new(CALL_SPAN), CALL_SPAN)]
        #[case::unreadable_revision(
            EvaluatedCall::new(CALL_SPAN)
                .with_positional(Value::int(UNREADABLE_POSITIONAL, ARGUMENT_SPAN)),
            ARGUMENT_SPAN
        )]
        fn reports_an_unreadable_argument_as_invalid_argument_at_its_span(
            #[case] call: EvaluatedCall,
            #[case] expected: Span,
        ) -> Result<(), TestError> {
            let input = handle_input();

            let err = rev_call(&Registry::new(), &call, &input)
                .err()
                .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            assert_matches!(
                &err,
                CommandError::InvalidArgument { span, .. } if *span == expected,
                "got {err:?}"
            );

            Ok(())
        }
    }

    /// Record rendering for the two kinds of object: a commit with a tree, a blob without one.
    mod rev_record {
        use super::*;

        #[rstest]
        #[case::commit_with_a_tree(RevKind::Commit, Some(FIXTURE_TREE))]
        #[case::blob_without_a_tree(RevKind::Blob, None)]
        fn keeps_the_columns_and_nulls_the_tree_only_when_absent(
            #[case] kind: RevKind,
            #[case] tree: Option<&str>,
        ) -> Result<(), TestError> {
            let info = RevInfo {
                oid: FIXTURE_OID.to_owned(),
                kind,
                tree: tree.map(str::to_owned),
                short: FIXTURE_SHORT.to_owned(),
            };

            let value = rev_record(info, Span::test_data());
            let columns: Vec<&str> = value.as_record()?.columns().map(String::as_str).collect();
            let tree_value = record_column(&value, TREE_COLUMN)?;

            assert_eq!(columns, EXPECTED_COLUMNS);

            match tree {
                Some(text) => assert_eq!(tree_value.as_str()?, text),
                None => assert!(tree_value.is_nothing(), "got {tree_value:?}"),
            }

            Ok(())
        }
    }
}
