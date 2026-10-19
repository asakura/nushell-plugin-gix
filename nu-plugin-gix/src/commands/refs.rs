//! `gix ref`: inspect and update references.

use crate::GixPlugin;
use crate::cdt::{Handle, Repository};
use crate::commands::{
    CATEGORY, CommandError, acquire_handle, argument_span, flag_arg, handle_from, help_call,
    positional_arg, require_full_oid,
};

use nu_plugin::{EngineInterface, EvaluatedCall, SimplePluginCommand};
use nu_plugin_gix_core::{self as git, GixError, RefExpect, Registry};
use nu_protocol::{
    Category, Example, LabeledError, Record, Signature, Span, Spanned, SyntaxShape, Type, Value,
    record,
};

const REF_COMMAND: &str = "gix ref";
const HEAD_COMMAND: &str = "gix ref head";
const LIST_COMMAND: &str = "gix ref list";
const SET_COMMAND: &str = "gix ref set";

const PREFIX_ARG: &str = "prefix";
const PREFIX_POSITION: usize = 0;
const NAME_ARG: &str = "name";
const NAME_POSITION: usize = 0;
const NEW_OID_ARG: &str = "new-oid";
const NEW_OID_POSITION: usize = 1;
const EXPECT_FLAG: &str = "expect";
const MESSAGE_FLAG: &str = "message";
const MESSAGE_SHORT: char = 'm';
const NAME_COLUMN: &str = "name";
const OID_COLUMN: &str = "oid";
const SYMBOLIC_TARGET_COLUMN: &str = "symbolic_target";

/// The reflog message recorded when `--message` is not given.
const DEFAULT_SET_MESSAGE: &str = "gix ref set";

/// Sample values shown as the output of the command examples.
const EXAMPLE_MAIN_REF: &str = "refs/heads/main";
const EXAMPLE_TOPIC_REF: &str = "refs/heads/topic";
const EXAMPLE_TAG_REF: &str = "refs/tags/v1.0.0";
const EXAMPLE_OID: &str = "0123456789abcdef0123456789abcdef01234567";
const EXAMPLE_NEW_OID: &str = "89abcdef0123456789abcdef0123456789abcdef";

/// Parent of the `gix ref` subcommands; prints their help listing.
#[derive(Clone, Copy, Debug)]
pub struct Ref;

impl SimplePluginCommand for Ref {
    type Plugin = GixPlugin;

    fn name(&self) -> &'static str {
        REF_COMMAND
    }

    fn description(&self) -> &'static str {
        "Inspect and update references: HEAD state, listing, and compare-and-swap updates."
    }

    fn extra_description(&self) -> &'static str {
        "You must use one of the following subcommands. Using this command as-is will only produce this help message.\n\
        Error codes: gix::help-unavailable"
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![Example {
            example: "gix ref",
            description: "Show the help listing of the ref subcommands",
            result: None,
        }]
    }

    fn signature(&self) -> Signature {
        Signature::build(REF_COMMAND)
            .input_output_type(Type::Nothing, Type::String)
            .category(Category::Custom(CATEGORY.into()))
    }

    fn run(
        &self,
        _plugin: &GixPlugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: &Value,
    ) -> Result<Value, LabeledError> {
        help_call(engine, call.head).map_err(|e| e.into_labeled(call.head))
    }
}

/// Shows where `HEAD` points: its object id and branch, or a null branch when detached.
#[derive(Clone, Copy, Debug)]
pub struct RefHead;

impl SimplePluginCommand for RefHead {
    type Plugin = GixPlugin;

    fn name(&self) -> &'static str {
        HEAD_COMMAND
    }

    fn description(&self) -> &'static str {
        "Show the object id and branch `HEAD` points at; the branch is null when detached."
    }

    fn extra_description(&self) -> &'static str {
        "Error codes: gix::wrong-input, gix::repository-gone, gix::open-failed, gix::bad-revision, \
        gix::unborn-head, gix::ref-read-failed"
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "gix open | gix ref head",
                description: "Show the object id and branch that HEAD points at",
                result: Some(Value::test_record(record! {
                    OID_COLUMN => Value::test_string(EXAMPLE_OID),
                    NAME_COLUMN => Value::test_string(EXAMPLE_MAIN_REF),
                })),
            },
            Example {
                example: "gix open | gix ref head | get name",
                description: "Show only the branch that HEAD points at, which is null when detached",
                result: Some(Value::test_string(EXAMPLE_MAIN_REF)),
            },
        ]
    }

    fn signature(&self) -> Signature {
        Signature::build(HEAD_COMMAND)
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
        head_call(plugin.registry(), call, input).map_err(|e| e.into_labeled(call.head))
    }
}

/// Reads the pipeline input, then reports where `HEAD` of the repository it names points.
fn head_call(
    registry: &Registry,
    call: &EvaluatedCall,
    input: &Value,
) -> Result<Value, CommandError> {
    let repo = handle_from::<Repository>(input)?;

    run_head(registry, repo, call.head, input.span())
}

/// Reports where `HEAD` of `repository` points: its object id, and its branch or a null branch
/// when detached.
///
/// The registry supplies the opened repository, rebuilding it when it is not cached. A handle whose
/// repository is gone or cannot be opened is reported at `handle_span`, the value that carried it.
/// The record spans `span`.
fn run_head(
    registry: &Registry,
    repository: &Repository,
    span: Span,
    handle_span: Span,
) -> Result<Value, CommandError> {
    let opened = acquire_handle(registry, repository, handle_span)?;
    let info = git::head(&opened)?;

    let name = info
        .name
        .map_or_else(|| Value::nothing(span), |name| Value::string(name, span));

    let record: Record = record! {
        OID_COLUMN => Value::string(info.oid, span),
        NAME_COLUMN => name,
    };

    Ok(Value::record(record, span))
}

/// Lists references whose full name starts with the optional prefix, sorted by name.
#[derive(Clone, Copy, Debug)]
pub struct RefList;

impl SimplePluginCommand for RefList {
    type Plugin = GixPlugin;

    fn name(&self) -> &'static str {
        LIST_COMMAND
    }

    fn description(&self) -> &'static str {
        "List references as a table of name, object id, and symbolic target."
    }

    fn extra_description(&self) -> &'static str {
        "Error codes: gix::wrong-input, gix::invalid-argument, gix::repository-gone, \
        gix::open-failed, gix::ref-read-failed"
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "gix open | gix ref list",
                description: "List every reference of the repository, sorted by name",
                result: Some(Value::test_list(vec![
                    example_ref_row(EXAMPLE_MAIN_REF),
                    example_ref_row(EXAMPLE_TOPIC_REF),
                    example_ref_row(EXAMPLE_TAG_REF),
                ])),
            },
            Example {
                example: "gix open | gix ref list heads",
                description: "List the branches, using the shorthand prefix `heads`",
                result: Some(Value::test_list(vec![
                    example_ref_row(EXAMPLE_MAIN_REF),
                    example_ref_row(EXAMPLE_TOPIC_REF),
                ])),
            },
            Example {
                example: "gix open | gix ref list refs/tags | get name",
                description: "List the full names of the tags, using the prefix `refs/tags`",
                result: Some(Value::test_list(vec![Value::test_string(EXAMPLE_TAG_REF)])),
            },
        ]
    }

    fn signature(&self) -> Signature {
        Signature::build(LIST_COMMAND)
            .optional(
                PREFIX_ARG,
                SyntaxShape::String,
                "only list references whose full name starts with this prefix, e.g. `heads` or `refs/tags`",
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
        _engine: &EngineInterface,
        call: &EvaluatedCall,
        input: &Value,
    ) -> Result<Value, LabeledError> {
        list_call(plugin.registry(), call, input).map_err(|e| e.into_labeled(call.head))
    }
}

/// Reads the pipeline input and the optional prefix, then lists the references they name.
fn list_call(
    registry: &Registry,
    call: &EvaluatedCall,
    input: &Value,
) -> Result<Value, CommandError> {
    let repo = handle_from::<Repository>(input)?;
    let prefix = prefix_arg(call)?;

    run_list(registry, repo, &prefix, call.head, input.span())
}

/// Lists the references under `prefix` in `repository`, one row each, sorted by name. An empty
/// prefix lists every reference. The rows span `span`.
///
/// The registry supplies the opened repository, rebuilding it when it is not cached. A handle whose
/// repository is gone or cannot be opened is reported at `handle_span`, the value that carried it.
fn run_list(
    registry: &Registry,
    repository: &Repository,
    prefix: &str,
    span: Span,
    handle_span: Span,
) -> Result<Value, CommandError> {
    let opened = acquire_handle(registry, repository, handle_span)?;
    let rows = git::list(&opened, prefix)?
        .iter()
        .map(|info| ref_record(info, span))
        .collect::<Vec<_>>();

    Ok(Value::list(rows, span))
}

/// Reads the optional prefix; a blank prefix lists every reference. A read failure points at the argument.
fn prefix_arg(call: &EvaluatedCall) -> Result<String, CommandError> {
    let prefix = call
        .opt::<Spanned<String>>(PREFIX_POSITION)
        .map_err(|source| CommandError::InvalidArgument {
            source: Box::new(source),
            span: argument_span(call, PREFIX_POSITION),
        })?;

    Ok(prefix
        .filter(|prefix| !prefix.item.trim().is_empty())
        .map_or_else(String::new, |prefix| prefix.item))
}

/// Moves a reference with a mandatory compare-and-swap expectation and a reflog entry.
#[derive(Clone, Copy, Debug)]
pub struct RefSet;

impl SimplePluginCommand for RefSet {
    type Plugin = GixPlugin;

    fn name(&self) -> &'static str {
        SET_COMMAND
    }

    fn description(&self) -> &'static str {
        "Move a reference to a new object id, failing when it does not hold the expected value."
    }

    fn extra_description(&self) -> &'static str {
        "Error codes: gix::wrong-input, gix::invalid-argument, gix::invalid-revision, \
        gix::not-a-full-object-id, gix::invalid-ref-name, gix::missing-expect, \
        gix::repository-gone, gix::open-failed, gix::ref-read-failed, gix::ref-moved, \
        gix::ref-update-failed"
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "gix open | gix ref set refs/heads/topic 0123456789abcdef0123456789abcdef01234567 --expect missing",
                description: "Create `refs/heads/topic` at the given object id, failing if it already exists",
                result: Some(example_ref_row_at(EXAMPLE_TOPIC_REF, EXAMPLE_OID)),
            },
            Example {
                example: "gix open | gix ref set refs/heads/main 89abcdef0123456789abcdef0123456789abcdef --expect 0123456789abcdef0123456789abcdef01234567 --message 'fast-forward'",
                description: "Move `refs/heads/main` to the new object id, failing unless it still holds the expected one, and record the reflog message",
                result: Some(example_ref_row_at(EXAMPLE_MAIN_REF, EXAMPLE_NEW_OID)),
            },
        ]
    }

    fn signature(&self) -> Signature {
        Signature::build(SET_COMMAND)
            .required(
                NAME_ARG,
                SyntaxShape::String,
                "full name of the reference to update, e.g. `refs/heads/main`",
            )
            .required(
                NEW_OID_ARG,
                SyntaxShape::String,
                "full object id the reference is moved to",
            )
            .named(
                EXPECT_FLAG,
                SyntaxShape::String,
                "full object id the ref must currently hold, or `missing`",
                None,
            )
            .named(
                MESSAGE_FLAG,
                SyntaxShape::String,
                "reflog message for the update",
                Some(MESSAGE_SHORT),
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
        set_call(plugin.registry(), call, input).map_err(|e| e.into_labeled(call.head))
    }
}

/// The validated arguments of one `gix ref set`.
#[derive(Debug)]
struct SetUpdate {
    /// Full name of the reference to move, with the span of its argument.
    name: Spanned<String>,
    /// Full object id the reference is moved to.
    new_oid: String,
    /// The value the reference must currently hold.
    expect: RefExpect,
    /// Span of the `--expect` value.
    expect_span: Span,
    /// Reflog message recorded for the move.
    message: String,
}

/// Reads and validates the arguments, then moves the reference they name in the repository the
/// pipeline input names.
fn set_call(
    registry: &Registry,
    call: &EvaluatedCall,
    input: &Value,
) -> Result<Value, CommandError> {
    let span = call.head;
    let repo = handle_from::<Repository>(input)?;

    let name = positional_arg::<String>(call, NAME_POSITION)?;

    let new_oid = positional_arg::<String>(call, NEW_OID_POSITION)?;
    let new_oid = parse_new_oid(new_oid)?;

    let expect = flag_arg::<String>(call, EXPECT_FLAG)?;
    let (expect, expect_span) = parse_expect(expect, span)?;

    let message = flag_arg::<String>(call, MESSAGE_FLAG)?;
    let message = message.map_or_else(|| DEFAULT_SET_MESSAGE.to_owned(), |message| message.item);

    let update = SetUpdate {
        name,
        new_oid,
        expect,
        expect_span,
        message,
    };

    run_set(registry, repo, &update, span, input.span())
}

/// Moves the reference `update` names in `repository` to its new object id, requiring it to hold
/// the expected value, and renders the moved reference as a row. The row spans `span`.
///
/// The registry supplies the opened repository, rebuilding it when it is not cached. A handle whose
/// repository is gone or cannot be opened is reported at `handle_span`, the value that carried it.
fn run_set(
    registry: &Registry,
    repository: &Repository,
    update: &SetUpdate,
    span: Span,
    handle_span: Span,
) -> Result<Value, CommandError> {
    let opened = acquire_handle(registry, repository, handle_span)?;

    let info = git::set(
        &opened,
        &update.name.item,
        &update.new_oid,
        update.expect,
        &update.message,
    )
    .map_err(|source| set_failure(source, update.name.span, update.expect_span))?;

    Ok(ref_record(&info, span))
}

/// Attributes a git failure to update a reference: an invalid name to the name argument at
/// `name_span`, a reference that moved to the `--expect` value at `expect_span`, and anything else
/// to no argument.
fn set_failure(source: GixError, name_span: Span, expect_span: Span) -> CommandError {
    let span = match &source {
        GixError::InvalidRefName {
            name: _name,
            detail: _detail,
        } => name_span,
        GixError::RefMoved {
            name: _name,
            expected: _expected,
            actual: _actual,
        } => expect_span,
        _ => return source.into(),
    };

    CommandError::Located { span, source }
}

/// Validates the new target: it must be a full hexadecimal object id.
fn parse_new_oid(new_oid: Spanned<String>) -> Result<String, CommandError> {
    require_full_oid(&new_oid.item, new_oid.span)?;

    Ok(new_oid.item)
}

/// Parses the mandatory expectation: `missing`, or a full object id the ref must currently hold.
/// Returns it with the span of the `--expect` value.
fn parse_expect(
    expect: Option<Spanned<String>>,
    span: Span,
) -> Result<(RefExpect, Span), CommandError> {
    let Some(expect) = expect else {
        return Err(CommandError::MissingExpect { span });
    };

    if expect.item == RefExpect::MISSING {
        return Ok((RefExpect::Missing, expect.span));
    }

    require_full_oid(&expect.item, expect.span)?;

    Ok((RefExpect::parse(&expect.item)?, expect.span))
}

/// A sample `{ name, oid, symbolic_target }` row for a direct reference at the example object id.
fn example_ref_row(name: &str) -> Value {
    example_ref_row_at(name, EXAMPLE_OID)
}

/// A sample `{ name, oid, symbolic_target }` row for a direct reference at `oid`.
fn example_ref_row_at(name: &str, oid: &str) -> Value {
    Value::test_record(record! {
        NAME_COLUMN => Value::test_string(name),
        OID_COLUMN => Value::test_string(oid),
        SYMBOLIC_TARGET_COLUMN => Value::test_nothing(),
    })
}

/// Maps one reference to a `{ name, oid, symbolic_target }` row; exactly one id column is null.
fn ref_record(info: &git::RefInfo, span: Span) -> Value {
    let (oid, symbolic_target) = match &info.target {
        git::RefTarget::Object(oid) => (Value::string(oid, span), Value::nothing(span)),
        git::RefTarget::Symbolic(target) => (Value::nothing(span), Value::string(target, span)),
    };

    let record: Record = record! {
        NAME_COLUMN => Value::string(info.name.as_str(), span),
        OID_COLUMN => oid,
        SYMBOLIC_TARGET_COLUMN => symbolic_target,
    };

    Value::record(record, span)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fixtures::{RepoInputError, plugin_test, repo_input};

    use nu_plugin_gix_testing_support::{
        AssertionError, FixtureError, GIT_DIR, MAIN_BRANCH, TaggedTestRepo, TestRepo,
        UnbornTestRepo, assert_label_in_script, eval_value, first_row, raised_labeled,
        record_column, tagged_repo, test_repo, unborn_repo,
    };
    use nu_plugin_test_support::PluginTest;
    use nu_protocol::ShellError;
    use rstest::rstest;

    use std::{assert_matches, fs, io};

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

    const SHA1_HEX: &str = "0123456789abcdef0123456789abcdef01234567";
    const NOT_HEXADECIMAL_OID: &str = "0123456789ABCDEF0123456789ABCDEF0123456z";
    const SHORT_OID: &str = "0123456";
    const TOPIC_BRANCH: &str = "refs/heads/topic";
    const SHORTHAND_REF: &str = "main";
    const HEADS_PREFIX: &str = "refs/heads";
    const TAGS_PREFIX: &str = "refs/tags";
    const SET_SCRIPT: &str = r#"gix ref set "refs/heads/main" "0123456789abcdef0123456789abcdef01234567" --expect missing"#;
    const EXPECT_SPELLING: &str = "--expect";
    const CUSTOM_MESSAGE: &str = "custom set message";
    const LOGS_DIR: &str = "logs";
    const EXPECTED_ERROR: &str = "expected an error";
    const UNBORN_HEAD_CODE: &str = "gix::unborn-head";
    const UNBORN_HEAD_MESSAGE: &str = "no commits yet";
    const REF_MOVED_CODE: &str = "gix::ref-moved";
    const INVALID_REVISION_CODE: &str = "gix::invalid-revision";
    const NOT_A_FULL_OBJECT_ID_CODE: &str = "gix::not-a-full-object-id";
    const INVALID_REF_NAME_CODE: &str = "gix::invalid-ref-name";
    const MISSING_EXPECT_CODE: &str = "gix::missing-expect";
    const REPOSITORY_GONE_CODE: &str = "gix::repository-gone";
    const REF_COLUMNS: [&str; 3] = [NAME_COLUMN, OID_COLUMN, SYMBOLIC_TARGET_COLUMN];
    const CALL_SPAN: Span = Span { start: 1, end: 7 };
    const ARG_SPAN: Span = Span { start: 7, end: 12 };
    const ARGUMENT_SPAN: Span = Span { start: 21, end: 25 };
    const UNREADABLE_VALUE: i64 = 1;

    /// Asserts that `row` describes the direct reference `name` pointing at `oid`.
    ///
    /// Expands in place, so `?` propagates out of the calling test.
    macro_rules! assert_points_at {
        ($row:expr, $name:expr, $oid:expr $(,)?) => {{
            let row: &Value = $row;

            assert_eq!(record_column(row, NAME_COLUMN)?.as_str()?, $name);
            assert_eq!(record_column(row, OID_COLUMN)?.as_str()?, $oid);

            assert!(
                record_column(row, SYMBOLIC_TARGET_COLUMN)?.is_nothing(),
                "got a symbolic target on a direct reference"
            );
        }};
    }

    mod ref_head {
        use super::*;

        #[rstest]
        fn shows_the_head_commit_and_its_branch(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let head = repo.head.as_str();

            let value = eval_value(&mut plugin, HEAD_COMMAND, repo_input(repo.path())?)?;

            assert_eq!(record_column(&value, OID_COLUMN)?.as_str()?, head);
            assert_eq!(record_column(&value, NAME_COLUMN)?.as_str()?, MAIN_BRANCH);

            Ok(())
        }

        #[rstest]
        fn rejects_an_unborn_head_with_its_code_and_message(
            plugin_test: Result<PluginTest, FixtureError>,
            unborn_repo: Result<UnbornTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = unborn_repo?;

            let labeled = raised_labeled(&mut plugin, HEAD_COMMAND, repo_input(repo.path())?)?;

            assert_eq!(labeled.code.as_deref(), Some(UNBORN_HEAD_CODE));

            assert!(
                labeled.msg.contains(UNBORN_HEAD_MESSAGE),
                "unexpected message: {}",
                labeled.msg
            );

            Ok(())
        }
    }

    mod ref_list {
        use super::*;

        #[rstest]
        fn lists_the_branch_pointing_at_the_head_commit(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let head = repo.head.as_str();

            let value = eval_value(&mut plugin, LIST_COMMAND, repo_input(repo.path())?)?;

            assert_eq!(value.as_list()?.len(), 1);
            assert_points_at!(first_row(&value)?, MAIN_BRANCH, head);

            Ok(())
        }

        /// Lists the references under `prefix` in `repo` and returns the name of each row.
        fn names_under(
            plugin: &mut PluginTest,
            repo: &TaggedTestRepo,
            prefix: &str,
        ) -> Result<Vec<String>, TestError> {
            let script = format!(r#"{LIST_COMMAND} "{prefix}""#);
            let value = eval_value(plugin, &script, repo_input(repo.path())?)?;

            let mut names = Vec::new();

            for row in value.as_list()? {
                names.push(record_column(row, NAME_COLUMN)?.as_str()?.to_owned());
            }

            Ok(names)
        }

        #[rstest]
        fn lists_only_the_references_under_the_prefix(
            plugin_test: Result<PluginTest, FixtureError>,
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = tagged_repo?;

            let heads = names_under(&mut plugin, &repo, HEADS_PREFIX)?;
            let tags = names_under(&mut plugin, &repo, TAGS_PREFIX)?;

            assert!(!heads.is_empty(), "no references under `{HEADS_PREFIX}`");
            assert!(!tags.is_empty(), "no references under `{TAGS_PREFIX}`");

            assert!(
                heads.iter().all(|name| name.starts_with(HEADS_PREFIX)),
                "references outside `{HEADS_PREFIX}`: {heads:?}"
            );

            assert!(
                tags.iter().all(|name| name.starts_with(TAGS_PREFIX)),
                "references outside `{TAGS_PREFIX}`: {tags:?}"
            );

            assert!(
                tags.iter().all(|name| !heads.contains(name)),
                "the prefixes share references: {heads:?} and {tags:?}"
            );

            Ok(())
        }
    }

    mod ref_set {
        use super::*;

        /// Which argument of `gix ref set` carries the object id under test.
        #[derive(Debug, Clone, Copy)]
        enum OidArgument {
            New,
            Expect,
        }

        impl OidArgument {
            fn script(self, value: &str, head: &str) -> String {
                match self {
                    Self::New => {
                        format!(r#"{SET_COMMAND} "{MAIN_BRANCH}" "{value}" --expect "{head}""#)
                    }
                    Self::Expect => {
                        format!(r#"{SET_COMMAND} "{MAIN_BRANCH}" "{head}" --expect "{value}""#)
                    }
                }
            }
        }

        #[rstest]
        fn moves_a_reference_when_the_expectation_matches(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let head = repo.head.as_str();
            let parent = repo.parent.as_str();
            let script = format!(r#"{SET_COMMAND} "{MAIN_BRANCH}" "{parent}" --expect "{head}""#);

            let value = eval_value(&mut plugin, &script, repo_input(repo.path())?)?;

            assert_points_at!(&value, MAIN_BRANCH, parent);

            Ok(())
        }

        /// Reads the reflog of the full reference `name` in the fixture repository `repo`.
        fn read_reflog(repo: &TestRepo, name: &str) -> Result<String, io::Error> {
            fs::read_to_string(repo.path().join(GIT_DIR).join(LOGS_DIR).join(name))
        }

        #[rstest]
        #[case::custom_message(Some(CUSTOM_MESSAGE))]
        #[case::default_message(None)]
        fn records_the_message_in_the_reflog(
            #[case] message: Option<&str>,
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let head = repo.head.as_str();
            let parent = repo.parent.as_str();

            let flag =
                message.map_or_else(String::new, |text| format!(r#" --{MESSAGE_FLAG} "{text}""#));

            let script =
                format!(r#"{SET_COMMAND} "{MAIN_BRANCH}" "{parent}" --expect "{head}"{flag}"#);

            _ = eval_value(&mut plugin, &script, repo_input(repo.path())?)?;

            let log = read_reflog(&repo, MAIN_BRANCH)?;
            let expected = message.unwrap_or(DEFAULT_SET_MESSAGE);

            assert!(log.contains(expected), "reflog lacks the message: {log}");

            Ok(())
        }

        #[rstest]
        #[case::new_wrong_length(OidArgument::New, SHORT_OID, NOT_A_FULL_OBJECT_ID_CODE)]
        #[case::expect_not_hexadecimal(
            OidArgument::Expect,
            NOT_HEXADECIMAL_OID,
            INVALID_REVISION_CODE
        )]
        fn rejects_a_malformed_object_id_with_its_code_and_label(
            #[case] argument: OidArgument,
            #[case] value: &str,
            #[case] code: &str,
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let script = argument.script(value, repo.head.as_str());

            let labeled = raised_labeled(&mut plugin, &script, repo_input(repo.path())?)?;

            assert_eq!(labeled.code.as_deref(), Some(code));
            assert_label_in_script(&labeled, &script)?;

            assert!(
                labeled.msg.contains(value),
                "message lacks the value: {}",
                labeled.msg
            );

            Ok(())
        }

        #[rstest]
        fn rejects_a_reference_name_without_the_refs_prefix_with_its_code_and_label(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let head = repo.head.as_str();
            let script = format!(r#"{SET_COMMAND} "{SHORTHAND_REF}" "{head}" --expect missing"#);

            let labeled = raised_labeled(&mut plugin, &script, repo_input(repo.path())?)?;

            assert_eq!(labeled.code.as_deref(), Some(INVALID_REF_NAME_CODE));
            assert_label_in_script(&labeled, &script)?;

            assert!(
                labeled.msg.contains(SHORTHAND_REF),
                "message lacks the name: {}",
                labeled.msg
            );

            Ok(())
        }

        #[rstest]
        fn reports_a_stale_expectation_with_its_code_and_label(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let parent = repo.parent.as_str();

            let script =
                format!(r#"{SET_COMMAND} "{MAIN_BRANCH}" "{parent}" --expect "{SHA1_HEX}""#);

            let labeled = raised_labeled(&mut plugin, &script, repo_input(repo.path())?)?;

            assert_eq!(labeled.code.as_deref(), Some(REF_MOVED_CODE));
            assert_label_in_script(&labeled, &script)?;

            assert!(
                labeled.msg.contains(MAIN_BRANCH),
                "message lacks the reference: {}",
                labeled.msg
            );

            Ok(())
        }

        #[rstest]
        fn reports_a_missing_expectation_with_its_code_and_label(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let head = repo.head.as_str();
            let script = format!(r#"{SET_COMMAND} "{MAIN_BRANCH}" "{head}""#);

            let labeled = raised_labeled(&mut plugin, &script, repo_input(repo.path())?)?;

            assert_eq!(labeled.code.as_deref(), Some(MISSING_EXPECT_CODE));
            assert_label_in_script(&labeled, &script)?;

            assert!(
                labeled.msg.contains(EXPECT_SPELLING),
                "message lacks the flag: {}",
                labeled.msg
            );

            Ok(())
        }
    }

    mod handle {
        use super::*;

        #[rstest]
        #[case::head(HEAD_COMMAND)]
        #[case::list(LIST_COMMAND)]
        #[case::set(SET_SCRIPT)]
        fn reports_a_deleted_repository_with_its_code(
            #[case] script: &str,
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let input = repo_input(repo.path())?;

            fs::remove_dir_all(repo.path())?;

            let labeled = raised_labeled(&mut plugin, script, input)?;

            assert_eq!(labeled.code.as_deref(), Some(REPOSITORY_GONE_CODE));
            assert!(!labeled.msg.is_empty(), "the error has no message");

            Ok(())
        }
    }

    mod ref_record {
        use super::*;

        /// Asserts that `column` of `row` holds `expected`, or is null when `expected` is `None`.
        ///
        /// Expands in place, so `?` propagates out of the calling test.
        macro_rules! assert_column {
            ($row:expr, $column:expr, $expected:expr $(,)?) => {{
                let value = record_column($row, $column)?;

                match $expected {
                    Some(text) => assert_eq!(value.as_str()?, text),
                    None => assert!(value.is_nothing(), "got {value:?} in `{}`", $column),
                }
            }};
        }

        #[rstest]
        #[case::object_target(git::RefTarget::Object(SHA1_HEX.to_owned()), Some(SHA1_HEX), None)]
        #[case::symbolic_target(
            git::RefTarget::Symbolic(MAIN_BRANCH.to_owned()),
            None,
            Some(MAIN_BRANCH)
        )]
        fn keeps_the_columns_and_fills_only_the_column_of_its_target(
            #[case] target: git::RefTarget,
            #[case] oid: Option<&str>,
            #[case] symbolic_target: Option<&str>,
        ) -> Result<(), TestError> {
            let info = git::RefInfo {
                name: TOPIC_BRANCH.to_owned(),
                target,
            };

            let value = ref_record(&info, Span::test_data());
            let columns: Vec<&str> = value.as_record()?.columns().map(String::as_str).collect();

            assert_eq!(columns, REF_COLUMNS);
            assert_column!(&value, OID_COLUMN, oid);
            assert_column!(&value, SYMBOLIC_TARGET_COLUMN, symbolic_target);

            Ok(())
        }
    }

    /// Exact-span checks of the argument parsers, whose failures never reach a script.
    mod parse_helpers {
        use super::*;

        /// The check that rejected an object id argument.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum Fault {
            WrongLength,
            NotHexadecimal,
            MissingExpect,
        }

        /// The check a parse error reports, with the span it points at.
        fn fault_of(err: &CommandError) -> Result<(Fault, Span), TestError> {
            match err {
                CommandError::NotAFullObjectId { span, .. } => Ok((Fault::WrongLength, *span)),
                CommandError::InvalidObjectId { span, .. } => Ok((Fault::NotHexadecimal, *span)),
                CommandError::MissingExpect { span } => Ok((Fault::MissingExpect, *span)),
                other => Err(TestError::Failure(format!("got {other:?}"))),
            }
        }

        mod parse_new_oid {
            use super::*;

            #[rstest]
            #[case::wrong_length(SHORT_OID, Fault::WrongLength)]
            #[case::not_hexadecimal(NOT_HEXADECIMAL_OID, Fault::NotHexadecimal)]
            fn rejects_a_malformed_new_oid_at_its_span(
                #[case] input: &str,
                #[case] expected: Fault,
            ) -> Result<(), TestError> {
                let new_oid = Spanned {
                    item: input.to_owned(),
                    span: ARG_SPAN,
                };

                let err = parse_new_oid(new_oid)
                    .err()
                    .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

                assert_eq!(fault_of(&err)?, (expected, ARG_SPAN));

                Ok(())
            }
        }

        mod parse_expect {
            use super::*;

            #[rstest]
            #[case::wrong_length(Some(SHORT_OID), Fault::WrongLength)]
            #[case::not_hexadecimal(Some(NOT_HEXADECIMAL_OID), Fault::NotHexadecimal)]
            #[case::missing(None, Fault::MissingExpect)]
            fn rejects_a_malformed_or_missing_expectation_at_its_span(
                #[case] input: Option<&str>,
                #[case] expected: Fault,
            ) -> Result<(), TestError> {
                let expect = input.map(|item| Spanned {
                    item: item.to_owned(),
                    span: ARG_SPAN,
                });

                let err = parse_expect(expect, ARG_SPAN)
                    .err()
                    .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

                assert_eq!(fault_of(&err)?, (expected, ARG_SPAN));

                Ok(())
            }
        }
    }

    /// Unit tests for the argument read no script can make fail: the parser rejects a value of
    /// the wrong shape for a `String` argument before the command runs.
    mod arguments {
        use super::*;

        mod prefix_arg {
            use super::*;

            #[rstest]
            fn reports_an_unreadable_prefix_as_invalid_argument_at_its_value()
            -> Result<(), TestError> {
                let call = EvaluatedCall::new(CALL_SPAN)
                    .with_positional(Value::int(UNREADABLE_VALUE, ARGUMENT_SPAN));

                let err = prefix_arg(&call)
                    .err()
                    .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

                assert_matches!(
                    &err,
                    CommandError::InvalidArgument { span, .. } if *span == ARGUMENT_SPAN,
                    "got {err:?}"
                );

                Ok(())
            }
        }
    }

    /// Command metadata that `eval` cannot observe.
    mod signature {
        use super::*;

        #[rstest]
        #[case::parent(Ref, REF_COMMAND)]
        #[case::head(RefHead, HEAD_COMMAND)]
        #[case::list(RefList, LIST_COMMAND)]
        #[case::set(RefSet, SET_COMMAND)]
        fn names_the_command<C>(#[case] command: C, #[case] expected: &str)
        where
            C: SimplePluginCommand<Plugin = GixPlugin>,
        {
            assert_eq!(command.signature().name, expected);
        }

        #[rstest]
        #[case::parent(Ref)]
        #[case::head(RefHead)]
        #[case::list(RefList)]
        #[case::set(RefSet)]
        fn runs_in_the_gix_category<C>(#[case] command: C)
        where
            C: SimplePluginCommand<Plugin = GixPlugin>,
        {
            assert_matches!(
                command.signature().category,
                Category::Custom(category) if category == CATEGORY
            );
        }

        #[rstest]
        #[case::head(RefHead, Type::record())]
        #[case::list(RefList, Type::table())]
        #[case::set(RefSet, Type::record())]
        fn transforms_a_repository_handle<C>(#[case] command: C, #[case] output: Type)
        where
            C: SimplePluginCommand<Plugin = GixPlugin>,
        {
            let signature = command.signature();

            assert_eq!(
                signature.input_output_types,
                vec![(
                    Type::Custom(<Repository as Handle>::TYPE_NAME.into()),
                    output
                )]
            );
        }

        #[rstest]
        fn prints_help_for_the_parent() {
            let signature = SimplePluginCommand::signature(&Ref);

            assert_eq!(
                signature.input_output_types,
                vec![(Type::Nothing, Type::String)]
            );
        }
    }
}
