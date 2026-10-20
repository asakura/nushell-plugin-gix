//! `gix config`: read the layered git configuration.

use crate::GixPlugin;
use crate::cdt::{Handle, Repository};
use crate::commands::config_known::{KnownKey, Multi, TABLE, ValueKind};
use crate::commands::config_read::{self, Group};
use crate::commands::error::{CONFIG_GET_EXTRA_CODES, CONFIG_LIST_CODES, error_codes_line};
use crate::commands::{
    CATEGORY, CommandError, acquire_handle, flag_arg, handle_from, help_call, positional_arg,
};

use nu_plugin::{EngineInterface, EvaluatedCall, SimplePluginCommand};
use nu_plugin_gix_core::{self as git, ConfigType, Registry};
use nu_protocol::{
    Category, Example, LabeledError, Signature, Span, Spanned, SyntaxShape, Type, Value, record,
};

use std::sync::LazyLock;

const CONFIG_COMMAND: &str = "gix config";
pub(super) const GET_COMMAND: &str = "gix config get";
pub(super) const LIST_COMMAND: &str = "gix config list";
const KEY_ARG: &str = "key";
const KEY_POSITION: usize = 0;
const TYPE_FLAG: &str = "type";
const TYPE_SHORT: char = 't';
const BOOL_TYPE: &str = "bool";
const INT_TYPE: &str = "int";
const PATH_TYPE: &str = "path";
const STRING_TYPE: &str = "string";

/// Sample values shown as the output of the command examples.
const EXAMPLE_USER_NAME: &str = "Ada";
const EXAMPLE_EDITOR: &str = "nano";
const EXAMPLE_REFSPEC: &str = "+refs/heads/*:refs/remotes/origin/*";
const EXAMPLE_BARE: bool = false;
const CORE_SECTION: &str = "core";
const USER_SECTION: &str = "user";
const BARE_VARIABLE: &str = "bare";
const EDITOR_VARIABLE: &str = "editor";
const NAME_VARIABLE: &str = "name";

/// Help text before the list of bool keys.
const HELP_BOOL_INTRO: &str = "Values come from the system, global, local and worktree configuration and their includes. \
        A key set in several layers has the value of the highest-priority layer. Writes are not supported.\n\
        Without --type, a key of the plugin's known-key table is converted to the kind listed there: bool \
        (true, yes, on, false, no, off), int (as git reads integers, such as 1, 0x10 or 2k) or text. Every other \
        key is text, so `user.name = 1k` gives the string \"1k\". The bool keys are ";
/// Help text between the bool and the int key lists.
const HELP_INT_INTRO: &str = "; the int keys are ";
/// Help text between the int key list and the list of path keys.
const HELP_PATH_INTRO: &str = "; the path keys, read as text unless --type path expands them, are ";
/// Help text between the path key list and the list of multi-value keys.
const HELP_LIST_INTRO: &str = ". \
        A valueless key (a name without `=`) is null without --type, where git prints an empty string.\n\
        Git has no marker for keys that take several values, so the plugin keeps a fixed table of them: ";
/// Help text between the multi-value keys and the keys that an empty value resets.
const HELP_RESET_INTRO: &str = ", and the lists that an empty value resets: ";
/// Help text after the multi-value key list: list rules and how subsections are written.
const HELP_REST_LISTS: &str = ". \
        Without --type, a key on that table is always a list of its values in load order across all layers, \
        even with one value, and each value is converted the same way; an empty value of a reset list removes the \
        entries before it. Every other key is a single value: the value of its last occurrence, even when it is set \
        several times. Key names are matched without regard to case; the lists above show them in lowercase, and \
        a name in angle brackets, such as `<name>`, stands for any subsection name, written in brackets \
        when the subsection is optional.\n";
/// Help text on keys whose values were all reset away.
const HELP_REST_RESET: &str = "\
        A key whose values were all reset away is an error, `gix::config-key-reset`, where git prints an empty \
        string; a section or subsection holding only such keys is an empty list.\n";
/// Help text on keys without a dot, which name sections, and on subsections.
const HELP_REST_GROUPS: &str = "\
        Without --type, a key without a dot names a section: the record of its variables and subsections is \
        returned, nested as in `gix config list`, or null when no layer sets it. A dotted key that no layer sets \
        as a variable names a subsection: `remote.origin` returns the record of `[remote \"origin\"]`, and a \
        subsection name may contain dots. A variable and a subsection of one name in one section are an error, \
        `gix::config-name-clash`, when the section is read; reading the variable itself still works. Groups are \
        never returned with --type.\n";
/// Help text on the `--type` conversions and the errors they raise.
const HELP_REST_TYPED: &str = "\
        With --type, the value is converted as git does for that type: bool (yes/on/true, no/off/false, \
        integers; a valueless key is true), int (k, m and g suffixes are powers of 1024), path (~ and %(prefix)/ \
        expanded; values set in untrusted repository files are ignored; an `:(optional)` path that does not exist \
        is null) or string (the raw value; git has no string type, so this is an extension of this plugin). With --type only the last value of a key is read, never a list, even \
        for a key on the list above. A valueless key is an error, `gix::config-value-missing`, for int and path, \
        where git fails, and for string, which git does not have. A key whose values were all reset away is an error with \
        --type too.\n";
/// Help text on the errors of values that are not UTF-8 and of source names.
const HELP_REST_ERRORS: &str = "\
        A value that is not UTF-8 is an error only for a read that reaches it.\n\
        An error names the file its value came from, or else its source kind, such as `environment`.";
/// The `gix config list` help before its `Error codes:` line.
const LIST_HELP_TEXT: &str = "Values come from the system, global, local and worktree configuration and their includes. \
        A key set in several layers has the value of the highest-priority layer.\n\
        The record nests by section: each section is a column whose record holds its variables, and a \
        subsection is a record inside its section, so `remote.origin.url` is `remote`, then `origin`, \
        then `url`. Section and variable names are lowercase. A subsection keeps its case, except in a \
        legacy `[section.subsection]` header, where it is lowercase too.\n\
        Each value is converted as `gix config get` converts it without --type: a bool, an int or a \
        string; a valueless key is null, also as an element of a list. A value of a known bool or int key that does not \
        convert is an error, `gix::invalid-config-value`. A key on the fixed list of multi-value keys \
        (see `gix config get`) is a list of its values in load order across all layers, and a key whose values were \
        all reset away is unset, so it is left out; any other key is the value of its last occurrence. A variable \
        and a subsection of one name in one section are an error, `gix::config-name-clash`, since git has no groups, \
        and it fails the whole listing. Columns are alphabetical, not in load order, and no scope or file is \
        reported.\n\
        An error names the file its value came from, or else its source kind, such as `environment`.";
/// Separator of the key names in the help lists.
const HELP_KEY_SEPARATOR: &str = ", ";

/// The `gix config get` help, with the key lists generated from the known-key table.
static GET_HELP: LazyLock<String> = LazyLock::new(get_help);

/// The `gix config list` help, with the generated `Error codes:` line.
static LIST_HELP: LazyLock<String> = LazyLock::new(|| {
    [
        LIST_HELP_TEXT,
        "\n",
        &error_codes_line(&[CONFIG_LIST_CODES]),
    ]
    .concat()
});

/// Parent of the `gix config` subcommands; prints their help listing.
#[derive(Clone, Copy, Debug)]
pub struct Config;

impl SimplePluginCommand for Config {
    type Plugin = GixPlugin;

    fn name(&self) -> &'static str {
        CONFIG_COMMAND
    }

    fn description(&self) -> &'static str {
        "Read the layered git configuration."
    }

    fn extra_description(&self) -> &'static str {
        "You must use one of the following subcommands. Using this command as-is will only produce this help message.\n\
        Error codes: gix::help-unavailable"
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![Example {
            example: "gix config",
            description: "Show the help listing of the config subcommands",
            result: None,
        }]
    }

    fn signature(&self) -> Signature {
        Signature::build(CONFIG_COMMAND)
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

/// Reads one key, or one section or subsection as a record, from the layered git configuration of the
/// repository a handle names; null when the key is unset, and an error when a reset removed all the values
/// of a key.
#[derive(Clone, Copy, Debug)]
pub struct ConfigGet;

impl SimplePluginCommand for ConfigGet {
    type Plugin = GixPlugin;

    fn name(&self) -> &'static str {
        GET_COMMAND
    }

    fn description(&self) -> &'static str {
        "Read one key, or one section as a record, from the layered git configuration; null when it is not set, and an error when a reset removed all the values of a key."
    }

    fn extra_description(&self) -> &'static str {
        GET_HELP.as_str()
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "gix open | gix config get user.name",
                description: "Read the user name from the configuration",
                result: Some(Value::test_string(EXAMPLE_USER_NAME)),
            },
            Example {
                example: "gix open | gix config get core.editor",
                description: "Read the configured editor; null when no layer sets it",
                result: None,
            },
            Example {
                example: "gix open | gix config get core.bare --type bool",
                description: "Read core.bare as a boolean",
                result: None,
            },
            Example {
                example: "gix open | gix config get remote.origin.fetch",
                description: "Read the fetch refspecs of a remote as a list",
                result: Some(Value::test_list(vec![Value::test_string(EXAMPLE_REFSPEC)])),
            },
            Example {
                example: "gix open | gix config get core",
                description: "Read the whole core section as a record",
                result: Some(Value::test_record(record! {
                    BARE_VARIABLE => Value::test_bool(EXAMPLE_BARE),
                    EDITOR_VARIABLE => Value::test_string(EXAMPLE_EDITOR),
                })),
            },
        ]
    }

    fn signature(&self) -> Signature {
        Signature::build(GET_COMMAND)
            .required(
                KEY_ARG,
                SyntaxShape::String,
                "configuration key, e.g. `user.name`, `remote.origin` or `core`",
            )
            .named(
                TYPE_FLAG,
                SyntaxShape::String,
                "convert the value as git does for bool, int and path; string (the raw value) is a plugin extension",
                Some(TYPE_SHORT),
            )
            .input_output_type(
                Type::Custom(<Repository as Handle>::TYPE_NAME.into()),
                Type::OneOf(Box::new([
                    Type::String,
                    Type::Int,
                    Type::Bool,
                    Type::Nothing,
                    Type::record(),
                    Type::List(Box::new(Type::Any)),
                ])),
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
        get_call(plugin.registry(), call, input).map_err(|e| e.into_labeled(call.head))
    }
}

/// Lists the whole layered git configuration as one nested record: each section is a column, and a subsection
/// is a record inside its section.
#[derive(Clone, Copy, Debug)]
pub struct ConfigList;

impl SimplePluginCommand for ConfigList {
    type Plugin = GixPlugin;

    fn name(&self) -> &'static str {
        LIST_COMMAND
    }

    fn description(&self) -> &'static str {
        "List the whole layered git configuration as a nested record."
    }

    fn extra_description(&self) -> &'static str {
        LIST_HELP.as_str()
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "gix open | gix config list",
                description: "List the whole configuration as a nested record",
                result: Some(Value::test_record(record! {
                    CORE_SECTION => Value::test_record(record! {
                        EDITOR_VARIABLE => Value::test_string(EXAMPLE_EDITOR),
                    }),
                    USER_SECTION => Value::test_record(record! {
                        NAME_VARIABLE => Value::test_string(EXAMPLE_USER_NAME),
                    }),
                })),
            },
            Example {
                example: "gix open | gix config list | get core.editor",
                description: "Read the configured editor from the listed configuration",
                result: Some(Value::test_string(EXAMPLE_EDITOR)),
            },
        ]
    }

    fn signature(&self) -> Signature {
        Signature::build(LIST_COMMAND)
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
        list_call(plugin.registry(), call, input).map_err(|e| e.into_labeled(call.head))
    }
}

/// What `get` reads, decided from the key argument and `--type` before the repository is opened.
enum Request {
    /// One key, read as the type `--type` names.
    Typed(git::ConfigKey, ConfigType),
    /// A section, such as `core`, read as a record.
    Section(git::SectionName),
    /// A dotted key without `--type`: the variable it names when one is set, otherwise the subsection it names.
    Subsection {
        /// The key the argument parses to, read as a variable first.
        key: git::ConfigKey,
        /// The subsection the argument names after its first dot.
        subsection: git::Subsection,
    },
}

/// Reads the pipeline input, then lists the configuration of the repository it names.
fn list_call(
    registry: &Registry,
    call: &EvaluatedCall,
    input: &Value,
) -> Result<Value, CommandError> {
    let repo = handle_from::<Repository>(input)?;

    run_list(registry, repo, call.head, input.span())
}

/// Builds the layered configuration of `repository` as one nested record, in which each section is a
/// column. The record spans `span`.
///
/// The registry supplies the opened repository, rebuilding it when it is not cached. A handle whose
/// repository is gone or cannot be opened is reported at `handle_span`, the value that carried it,
/// and so is a configuration that cannot be read, including a value that is not UTF-8.
fn run_list(
    registry: &Registry,
    repository: &Repository,
    span: Span,
    handle_span: Span,
) -> Result<Value, CommandError> {
    let opened = acquire_handle(registry, repository, handle_span)?;

    let config = git::Config::read(&opened).map_err(|source| CommandError::Located {
        span: handle_span,
        source,
    })?;

    config_read::read_all(&config, span).map_err(|source| CommandError::Located {
        span: handle_span,
        source,
    })
}

/// Reads and checks the key argument, then reads the key from the repository the pipeline input names.
///
/// The argument is settled first, so a bad key is reported whatever the input or the repository is. Without
/// `--type` a key without a dot names a section, so it is not checked as a key.
fn get_call(
    registry: &Registry,
    call: &EvaluatedCall,
    input: &Value,
) -> Result<Value, CommandError> {
    let key = positional_arg::<String>(call, KEY_POSITION)?;
    let ty = config_type(call)?;
    let request = parse_request(&key, ty)?;
    let repo = handle_from::<Repository>(input)?;

    run_get(registry, repo, &key, request, call.head, input.span())
}

/// Reads the `--type` flag: `None` when it is absent, otherwise the type it names.
///
/// # Errors
///
/// Returns [`CommandError::UnknownConfigType`] when the flag names none of the four types.
fn config_type(call: &EvaluatedCall) -> Result<Option<ConfigType>, CommandError> {
    let Some(given) = flag_arg::<String>(call, TYPE_FLAG)? else {
        return Ok(None);
    };

    let Spanned { item, span } = given;

    match item.as_str() {
        BOOL_TYPE => Ok(Some(ConfigType::Scalar(git::ScalarType::Bool))),
        INT_TYPE => Ok(Some(ConfigType::Scalar(git::ScalarType::Int))),
        PATH_TYPE => Ok(Some(ConfigType::Path)),
        STRING_TYPE => Ok(Some(ConfigType::Scalar(git::ScalarType::String))),
        _ => Err(CommandError::UnknownConfigType { given: item, span }),
    }
}

/// Parses the key argument once into what to read: a key with `--type` (a key without a dot is invalid), otherwise
/// a section, or a subsection whose dotted text is also a key.
///
/// # Errors
///
/// Returns [`git::GixError::InvalidConfigKey`], at the key span, for a key that does not parse.
fn parse_request(key: &Spanned<String>, ty: Option<ConfigType>) -> Result<Request, CommandError> {
    match ty {
        Some(ty) => Ok(Request::Typed(parse_key(key)?, ty)),
        None => match git::GroupPath::parse(&key.item).map_err(|source| CommandError::Located {
            span: key.span,
            source,
        })? {
            git::GroupPath::Section(section) => Ok(Request::Section(section)),
            git::GroupPath::Subsection(_, subsection) => Ok(Request::Subsection {
                key: parse_key(key)?,
                subsection,
            }),
        },
    }
}

/// Parses the key argument as a `section.name` or `section.subsection.name` key.
///
/// # Errors
///
/// Returns [`git::GixError::InvalidConfigKey`], at the key span, for a key that does not parse.
fn parse_key(key: &Spanned<String>) -> Result<git::ConfigKey, CommandError> {
    git::ConfigKey::parse(&key.item).map_err(|source| CommandError::Located {
        span: key.span,
        source,
    })
}

/// Reads what `request` names from the configuration of `repository`. Null when unset.
///
/// The registry supplies the opened repository, rebuilding it when it is not cached. A handle whose
/// repository is gone or cannot be opened is reported at `handle_span`, the value that carried it.
/// A value, list or group that cannot be read, and a key whose values a reset removed all of, is reported at
/// the span of the key argument. The value spans `span`.
fn run_get(
    registry: &Registry,
    repository: &Repository,
    key: &Spanned<String>,
    request: Request,
    span: Span,
    handle_span: Span,
) -> Result<Value, CommandError> {
    let opened = acquire_handle(registry, repository, handle_span)?;

    let config = git::Config::read(&opened).map_err(|source| CommandError::Located {
        span: key.span,
        source,
    })?;

    let value = match request {
        Request::Typed(config_key, ty) => config_read::read_typed(
            &config,
            &config_key,
            ty,
            &git::PathContext::of(&opened),
            span,
        ),
        Request::Section(section) => {
            config_read::read_section(&config, &section, span).map(|group| group_value(group, span))
        }
        Request::Subsection {
            key: config_key,
            subsection,
        } => config_read::read_variable(&config, &config_key, span).and_then(|found| {
            found.map_or_else(
                || {
                    config_read::read_subsection(&config, config_key.section(), &subsection, span)
                        .map(|group| group_value(group, span))
                },
                Ok,
            )
        }),
    };

    value.map_err(|source| CommandError::Located {
        span: key.span,
        source,
    })
}

/// The Nushell value of a section or subsection: null when no key of it is set, an empty list when resets
/// removed all its values, otherwise its record.
fn group_value(group: Group, span: Span) -> Value {
    match group {
        Group::Missing => Value::nothing(span),
        Group::AllReset => Value::list(Vec::new(), span),
        Group::Record(record) => Value::record(record, span),
    }
}

/// The long help of `gix config get`; its key lists are the rows of the known-key table.
fn get_help() -> String {
    [
        HELP_BOOL_INTRO,
        &key_list(|row| row.kind == ValueKind::Bool),
        HELP_INT_INTRO,
        &key_list(|row| row.kind == ValueKind::Int),
        HELP_PATH_INTRO,
        &key_list(|row| row.kind == ValueKind::Path),
        HELP_LIST_INTRO,
        &key_list(|row| row.multi == Multi::List),
        HELP_RESET_INTRO,
        &key_list(|row| row.multi == Multi::ListWithReset),
        HELP_REST_LISTS,
        HELP_REST_RESET,
        HELP_REST_GROUPS,
        HELP_REST_TYPED,
        HELP_REST_ERRORS,
        "\n",
        &error_codes_line(&[CONFIG_LIST_CODES, CONFIG_GET_EXTRA_CODES]),
    ]
    .concat()
}

/// The selected rows of the known-key table, in table order, joined by commas.
fn key_list<F>(select: F) -> String
where
    F: Fn(&KnownKey) -> bool,
{
    TABLE
        .iter()
        .filter(|row| select(row))
        .map(KnownKey::to_string)
        .collect::<Vec<_>>()
        .join(HELP_KEY_SEPARATOR)
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
