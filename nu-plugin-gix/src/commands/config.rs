//! `gix config [<key>]`: read the layered git configuration as one nested record, or a section, subsection or variable of it selected by a cell path or a string, converted with `--type`; `gix config list` reads it as a table.

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;

use crate::GixPlugin;
use crate::cdt::{Handle, Repository};
use crate::commands::config_key::{KeyArg, key_arg};
use crate::commands::config_known::{KnownKey, Multi, TABLE, ValueKind};
use crate::commands::config_read;
use crate::commands::error::{CONFIG_KEY_CODES, CONFIG_LIST_CODES, error_codes_line};
use crate::commands::{
    CATEGORY, CommandError, acquire_handle, flag_arg, handle_from, optional_arg,
};

use nu_plugin::{EngineInterface, EvaluatedCall, SimplePluginCommand};
use nu_plugin_gix_core::{self as git, ConfigType, Registry};
use nu_protocol::{
    Category, Example, LabeledError, Signature, Span, Spanned, SyntaxShape, Type, Value,
};

use std::sync::LazyLock;

pub(super) const CONFIG_COMMAND: &str = "gix config";
pub(super) const LIST_COMMAND: &str = "gix config list";
const KEY_ARG: &str = "key";
const KEY_POSITION: usize = 0;
const TYPE_FLAG: &str = "type";
const TYPE_SHORT: char = 't';
const BOOL_TYPE: &str = "bool";
const INT_TYPE: &str = "int";
const PATH_TYPE: &str = "path";
const STRING_TYPE: &str = "string";

/// Help text before the list of bool keys.
const HELP_BOOL_INTRO: &str = "Without a key, the whole configuration is returned as one nested record: each section is a column, a subsection is a record inside its section (`remote.origin.url` is `remote`, then `origin`, then `url`), and section and variable names are lowercase. A subsection keeps its case, except in a legacy `[section.subsection]` header, where it is lowercase too. Columns are alphabetical. Navigate the record with Nushell, for example `gix config | get remote.origin.url`; `gix config list` shows every setting with its file and scope.\n\
        Values come from the system, global, local and worktree configuration and their includes. \
        A key set in several layers has the value of the highest-priority layer.\n\
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
const HELP_REST_RESET: &str = "A key whose values were all reset away is left out of every record; asked for directly it is an error, `gix::config-key-reset`, where git prints an empty string; a section or subsection holding only such keys counts as not set.\n";
/// Help text on keys without a dot, which name sections, and on subsections.
const HELP_REST_GROUPS: &str = "The key is a cell path or a string. A cell path is already split: one part is a section (`core`), two parts are `section.name` (`user.name`, or the subsection `name` of the section: `remote.origin` returns the record of `[remote \"origin\"]`), three parts are `section.subsection.name` (`remote.origin.url`); more is `gix::invalid-config-key`. Quote a part that contains a dot: `url.\"https://a.b/\".insteadof`. A string (a variable holding text) is split at its first and last dot: the section is before the first dot, the name after the last, and the subsection between them may contain dots (`'url.https://a.b/.insteadof'`); without a dot it is a section. Section and variable names ignore case; a subsection is matched exactly. A part marked `?` makes a key that is not set null instead of an error; `!` is accepted and changes nothing, since names are already case-insensitive. A key that is not set is an error, `gix::config-key-not-found`, as with Nushell's `get`. A variable and a subsection of one name in one section show only the subsection in a record; `gix config list` shows both.\n";
/// Help text on the `--type` conversions and the errors they raise.
const HELP_REST_TYPED: &str = "With --type, the key must name a single value (a section or subsection is `gix::config-type-needs-value`, and --type without a key is `gix::config-type-needs-key`), and the value is converted as git does for that type: bool (yes/on/true, no/off/false, integers; a valueless key is true), int (k, m and g suffixes are powers of 1024), path (~ and %(prefix)/ expanded; values set in untrusted repository files are ignored; an `:(optional)` path that does not exist is null) or string (the raw value; git has no string type, so this is an extension of this plugin). --type overrides the conversion of the known-key table for that key. With --type only the last value of a key is read, never a list, even for a key on the list above. A valueless key is an error, `gix::config-value-missing`, for int and path, where git fails, and for string, which git does not have. A value that does not fit the type is `gix::invalid-config-value`. The types color, size and expiry-date of git are not supported yet.\n";
/// Help text on the errors of values that are not UTF-8 and of source names.
const HELP_REST_ERRORS: &str = "\
        A value that is not UTF-8 is an error only for a read that reaches it.\n\
        An error names the file its value came from, or else its source kind, such as `environment`.";
/// The `gix config list` help before its `Error codes:` line.
const LIST_HELP_TEXT: &str = "Every setting of the layered git configuration, one row per occurrence, as `git config --list --show-origin --show-scope` shows them: system, global, local and worktree files and their includes, then the command line and environment.\n\
        Columns: `key` (lowercase section and variable name, subsection case kept: `remote.Origin.url`), `value` (always text; null for a name written without `=`), `scope` (`system`, `global`, `local`, `worktree` or `command`) and `file` (the path of the file, or null when the setting does not come from a file). Rows are sorted by key, and the occurrences of one key are in load order, so the last row of a key is the value git uses. Unlike `gix config`, nothing is merged: a variable and a subsection of one name both appear, and a list key shows every value.\n\
        A value or subsection that is not UTF-8 is an error, `gix::config-value-not-utf8` or `gix::config-subsection-not-utf8`.\n\
        An error names the file its value came from, or else its source kind, such as `environment`.";
/// Separator of the key names in the help lists.
const HELP_KEY_SEPARATOR: &str = ", ";

/// The `gix config` help, with the key lists generated from the known-key table.
static CONFIG_HELP: LazyLock<String> = LazyLock::new(config_help);

/// The `gix config list` help, with the generated `Error codes:` line.
static LIST_HELP: LazyLock<String> = LazyLock::new(|| {
    [
        LIST_HELP_TEXT,
        "\n",
        &error_codes_line(&[CONFIG_LIST_CODES]),
    ]
    .concat()
});

/// Reads the whole layered git configuration of the repository a handle names as one nested record: each section is a
/// column, and a subsection is a record inside its section.
#[derive(Clone, Copy, Debug)]
pub struct Config;

impl SimplePluginCommand for Config {
    type Plugin = GixPlugin;

    fn name(&self) -> &'static str {
        CONFIG_COMMAND
    }

    fn description(&self) -> &'static str {
        "Read the layered git configuration as a nested record, or one key of it."
    }

    fn extra_description(&self) -> &'static str {
        CONFIG_HELP.as_str()
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "gix open | gix config",
                description: "Read the whole configuration as a nested record",
                result: None,
            },
            Example {
                example: "gix open | gix config user.name",
                description: "Read the user name",
                result: None,
            },
            Example {
                example: "gix open | gix config core.bare --type bool",
                description: "Read a value converted as a bool",
                result: None,
            },
            Example {
                example: "gix open | gix config remote.origin.fetch",
                description: "Read a key that holds a list of values",
                result: None,
            },
            Example {
                example: "gix open | gix config core",
                description: "Read a section as a record",
                result: None,
            },
            Example {
                example: "gix open | gix config | get remote.origin.url",
                description: "Navigate the nested record with get",
                result: None,
            },
            Example {
                example: "let k = 'url.https://a.b/.insteadof'; gix open | gix config $k",
                description: "Read a key given as a string, split at its first and last dot",
                result: None,
            },
            Example {
                example: "gix open | gix config user.nickname?",
                description: "Read a key that may be unset: null when it is not set",
                result: None,
            },
        ]
    }

    fn signature(&self) -> Signature {
        Signature::build(CONFIG_COMMAND)
            .optional(
                KEY_ARG,
                SyntaxShape::OneOf(vec![SyntaxShape::CellPath, SyntaxShape::String]),
                "configuration key: a cell path (`user.name`, `remote.origin`, `core`, `url.\"https://a.b/\".insteadof`) or a string split at its first and last dot (`'url.https://a.b/.insteadof'`)",
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
        config_call(plugin.registry(), call, input).map_err(|e| e.into_labeled(call.head))
    }
}

/// Lists the whole layered git configuration as a table with one row per occurrence: key, value, scope and file.
#[derive(Clone, Copy, Debug)]
pub struct ConfigList;

impl SimplePluginCommand for ConfigList {
    type Plugin = GixPlugin;

    fn name(&self) -> &'static str {
        LIST_COMMAND
    }

    fn description(&self) -> &'static str {
        "List every setting of the layered git configuration as a table: key, value, scope and file."
    }

    fn extra_description(&self) -> &'static str {
        LIST_HELP.as_str()
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "gix open | gix config list",
                description: "List every setting of the configuration as a table",
                result: None,
            },
            Example {
                example: "gix open | gix config list | where scope == local",
                description: "List the settings of the repository's own configuration file",
                result: None,
            },
        ]
    }

    fn signature(&self) -> Signature {
        Signature::build(LIST_COMMAND)
            .input_output_type(
                Type::Custom(<Repository as Handle>::TYPE_NAME.into()),
                Type::List(Box::new(Type::record())),
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

/// What `gix config` reads, settled from the key argument before the repository is opened.
enum Query {
    /// The whole configuration.
    Everything,
    /// A section, a subsection or a variable, read as the type `--type` names when it is given.
    Key {
        /// The key and the span of its argument.
        key: Spanned<KeyArg>,
        /// The type `--type` names; only for a key that can be a variable.
        ty: Option<ConfigType>,
    },
}

/// Reads the key argument and the `--type` flag, if any, into a [`Query`].
///
/// # Errors
///
/// Returns the errors of [`config_type`], [`key_arg`] and [`optional_arg`], [`CommandError::ConfigTypeNeedsKey`]
/// for `--type` without a key, and [`CommandError::ConfigTypeNeedsValue`] for `--type` with a key that names a
/// section.
fn parse_query(call: &EvaluatedCall) -> Result<Query, CommandError> {
    let ty = config_type(call)?;

    match (ty, optional_arg::<Value>(call, KEY_POSITION)?) {
        (None, None) => Ok(Query::Everything),
        (Some(_), None) => Err(CommandError::ConfigTypeNeedsKey {
            span: call.get_flag_span(TYPE_FLAG).unwrap_or(call.head),
        }),
        (ty, Some(key)) => {
            let item = key_arg(&key)?;

            if ty.is_some() && matches!(item.path, git::ConfigPath::Section(_)) {
                return Err(CommandError::ConfigTypeNeedsValue {
                    key: item.text,
                    span: key.span,
                });
            }

            Ok(Query::Key {
                key: Spanned {
                    item,
                    span: key.span,
                },
                ty: ty.map(|typed| typed.item),
            })
        }
    }
}

/// Reads what a key names in the tree: the untyped value of a section, subsection or variable, or with a type the
/// converted value of a variable. `None` when the key names nothing.
///
/// # Errors
///
/// Returns [`CommandError::ConfigTypeNeedsValue`] when a type is given and the key names a subsection, and
/// [`CommandError::Located`], at the key, for a value that cannot be read.
fn key_value(
    node: Option<git::Node<'_>>,
    typed: Option<&(ConfigType, git::PathContext)>,
    key: &KeyArg,
    key_span: Span,
    span: Span,
) -> Result<Option<Value>, CommandError> {
    let located = |source| CommandError::Located {
        span: key_span,
        source,
    };

    match (node, typed) {
        (None, _) => Ok(None),
        (Some(git::Node::Setting(setting)), Some((ty, paths))) => {
            config_read::typed_setting(setting, *ty, paths, span)
                .map(Some)
                .map_err(located)
        }
        (Some(git::Node::Section(_) | git::Node::Subsection(_)), Some(_)) => {
            Err(CommandError::ConfigTypeNeedsValue {
                key: key.text.clone(),
                span: key_span,
            })
        }
        (Some(node), None) => config_read::lookup_value(node, span).map_err(located),
    }
}

/// Settles the key argument, then reads the configuration of the repository the pipeline input names: all of it as
/// a nested record, or the section, subsection or variable the key names. A key that names nothing is an error,
/// or null when it ends in `?`.
fn config_call(
    registry: &Registry,
    call: &EvaluatedCall,
    input: &Value,
) -> Result<Value, CommandError> {
    let query = parse_query(call)?;
    let repo = handle_from::<Repository>(input)?;
    let handle_span = input.span();
    let opened = acquire_handle(registry, repo, handle_span)?;

    let config = git::Config::read(&opened).map_err(|source| CommandError::Located {
        span: handle_span,
        source,
    })?;

    match query {
        Query::Everything => {
            config_read::everything(&config, call.head).map_err(|source| CommandError::Located {
                span: handle_span,
                source,
            })
        }
        Query::Key {
            key: Spanned { item: key, span },
            ty,
        } => {
            let tree = config.tree();
            let typed = ty.map(|ty| (ty, git::PathContext::of(&opened)));

            let found = key_value(tree.get(&key.path), typed.as_ref(), &key, span, call.head)?;

            match found {
                Some(value) => Ok(value),
                None if key.optional => Ok(Value::nothing(call.head)),
                None => Err(CommandError::ConfigKeyNotFound {
                    key: key.text,
                    span,
                }),
            }
        }
    }
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

/// Builds the layered configuration of `repository` as a table with one row per occurrence. The rows span `span`.
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

    config_read::rows(&config.tree(), span)
        .map(|rows| Value::list(rows, span))
        .map_err(|source| CommandError::Located {
            span: handle_span,
            source,
        })
}

/// Reads the `--type` flag: `None` when it is absent, otherwise the type it names, with the span of its value.
///
/// # Errors
///
/// Returns [`CommandError::UnknownConfigType`] when the flag names none of the four types.
fn config_type(call: &EvaluatedCall) -> Result<Option<Spanned<ConfigType>>, CommandError> {
    let Some(given) = flag_arg::<String>(call, TYPE_FLAG)? else {
        return Ok(None);
    };

    let Spanned { item, span } = given;

    let ty = match item.as_str() {
        BOOL_TYPE => ConfigType::Scalar(git::ScalarType::Bool),
        INT_TYPE => ConfigType::Scalar(git::ScalarType::Int),
        PATH_TYPE => ConfigType::Path,
        STRING_TYPE => ConfigType::Scalar(git::ScalarType::String),
        _ => return Err(CommandError::UnknownConfigType { given: item, span }),
    };

    Ok(Some(Spanned { item: ty, span }))
}

/// The long help of `gix config`; its key lists are the rows of the known-key table.
fn config_help() -> String {
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
        &error_codes_line(&[CONFIG_LIST_CODES, CONFIG_KEY_CODES]),
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
