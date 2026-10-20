//! `gix config`: read the layered git configuration.

use crate::GixPlugin;
use crate::cdt::{Handle, Repository};
use crate::commands::{
    CATEGORY, CommandError, acquire_handle, flag_arg, handle_from, help_call, positional_arg,
};

use nu_plugin::{EngineInterface, EvaluatedCall, SimplePluginCommand};
use nu_plugin_gix_core::{self as git, ConfigType, ConfigValue, Registry};
use nu_protocol::{
    Category, Example, LabeledError, Signature, Span, Spanned, SyntaxShape, Type, Value,
};

const CONFIG_COMMAND: &str = "gix config";
const GET_COMMAND: &str = "gix config get";
const KEY_ARG: &str = "key";
const KEY_POSITION: usize = 0;
const TYPE_FLAG: &str = "type";
const TYPE_SHORT: char = 't';
const BOOL_TYPE: &str = "bool";
const INT_TYPE: &str = "int";
const PATH_TYPE: &str = "path";
const STRING_TYPE: &str = "string";

/// Sample value shown as the output of the command example.
const EXAMPLE_USER_NAME: &str = "Ada";

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

/// Reads one key from the layered git configuration of the repository a handle names; null when no
/// layer sets it.
#[derive(Clone, Copy, Debug)]
pub struct ConfigGet;

impl SimplePluginCommand for ConfigGet {
    type Plugin = GixPlugin;

    fn name(&self) -> &'static str {
        GET_COMMAND
    }

    fn description(&self) -> &'static str {
        "Read one key from the layered git configuration; null when it is not set."
    }

    fn extra_description(&self) -> &'static str {
        "Values come from the system, global, local and worktree configuration and their includes; \
        the most recent value wins. Writes are not supported.\n\
        Without --type, a value is converted by its content: true, yes, on, a valueless key, false, \
        no and off give a bool; an integer as git reads it (1, 0x10, 2k) gives an int; anything else, \
        including the empty string, stays a string.\n\
        With --type, the value is converted as git does for that type: bool (yes/on/true, no/off/false, \
        integers, valueless keys), int (k, m and g suffixes are powers of 1024), path (~ and %(prefix)/ \
        expanded; only trusted files are read, so a value set only in an untrusted file is null) or \
        string (the raw value).\n\
        Error codes: gix::wrong-input, gix::invalid-argument, gix::invalid-config-key, \
        gix::unknown-config-type, gix::invalid-config-value, gix::config-value-not-utf8, \
        gix::config-path-not-utf8, gix::open-failed, gix::repository-gone"
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
        ]
    }

    fn signature(&self) -> Signature {
        Signature::build(GET_COMMAND)
            .required(
                KEY_ARG,
                SyntaxShape::String,
                "configuration key, e.g. `user.name` or `remote.origin.url`",
            )
            .named(
                TYPE_FLAG,
                SyntaxShape::String,
                "convert the value as git does: bool, int, path or string",
                Some(TYPE_SHORT),
            )
            .input_output_type(
                Type::Custom(<Repository as Handle>::TYPE_NAME.into()),
                Type::OneOf(Box::new([
                    Type::String,
                    Type::Int,
                    Type::Bool,
                    Type::Nothing,
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

/// Reads and checks the key argument, then reads the key from the repository the pipeline input names.
///
/// The argument is settled first, so a bad key is reported whatever the input or the repository is.
fn get_call(
    registry: &Registry,
    call: &EvaluatedCall,
    input: &Value,
) -> Result<Value, CommandError> {
    let key = positional_arg::<String>(call, KEY_POSITION)?;

    git::validate_config_key(&key.item).map_err(|source| CommandError::Located {
        span: key.span,
        source,
    })?;

    let ty = config_type(call)?;
    let repo = handle_from::<Repository>(input)?;

    run_get(registry, repo, &key, ty, call.head, input.span())
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
        BOOL_TYPE => Ok(Some(ConfigType::Bool)),
        INT_TYPE => Ok(Some(ConfigType::Int)),
        PATH_TYPE => Ok(Some(ConfigType::Path)),
        STRING_TYPE => Ok(Some(ConfigType::String)),
        _ => Err(CommandError::UnknownConfigType { given: item, span }),
    }
}

/// Reads `key` from the configuration of `repository`, as `ty` when given or converted by its content
/// otherwise; null when unset.
///
/// The registry supplies the opened repository, rebuilding it when it is not cached. A handle whose
/// repository is gone or cannot be opened is reported at `handle_span`, the value that carried it.
/// A value that cannot be read is reported at the span of the key argument.
/// The value spans `span`.
fn run_get(
    registry: &Registry,
    repository: &Repository,
    key: &Spanned<String>,
    ty: Option<ConfigType>,
    span: Span,
    handle_span: Span,
) -> Result<Value, CommandError> {
    let opened = acquire_handle(registry, repository, handle_span)?;

    let found = ty.map_or_else(
        || git::config_get_auto(&opened, &key.item),
        |ty| git::config_get_typed(&opened, &key.item, ty),
    );

    let value = found.map_err(|source| CommandError::Located {
        span: key.span,
        source,
    })?;

    Ok(config_value(value, span))
}

/// Maps a converted configuration value to the Nushell value it is returned as; null when unset.
fn config_value(value: Option<ConfigValue>, span: Span) -> Value {
    match value {
        None => Value::nothing(span),
        Some(ConfigValue::Bool(flag)) => Value::bool(flag, span),
        Some(ConfigValue::Int(number)) => Value::int(number, span),
        Some(ConfigValue::Path(text) | ConfigValue::String(text)) => Value::string(text, span),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fixtures::{RepoInputError, plugin_test, repo_input};

    use nu_plugin_gix_testing_support::{
        AssertionError, FixtureError, FixtureRepo as _, TestRepo, append_config,
        assert_label_in_script, eval_labeled_error, eval_value, test_repo,
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

        #[expect(
            dead_code,
            reason = "mandatory TestError variant; the smoke tests have no failure path that builds it"
        )]
        #[error("Test failure: {0}")]
        Failure(String),
    }

    impl From<ShellError> for TestError {
        fn from(error: ShellError) -> Self {
            Self::Shell(Box::new(error))
        }
    }

    const LOCAL_SECTION: &[u8] = b"[gixtest]\n\tanswer = 42\n";
    const LOCAL_KEY: &str = "gixtest.answer";
    const LOCAL_VALUE: i64 = 42;
    const TYPED_SECTION: &[u8] = b"[gixtest]\n\tflag = yes\n\tnum = 2k\n\ttext = maybe\n";
    const FLAG_KEY: &str = "gixtest.flag";
    const NUM_KEY: &str = "gixtest.num";
    const TEXT_KEY: &str = "gixtest.text";
    const ABSENT_KEY: &str = "gixtest.absent";
    const KILOBYTES_VALUE: i64 = 2048;
    const PLAIN_TEXT: &str = "maybe";
    const UNKNOWN_TYPE: &str = "bogus";
    const MALFORMED_KEY: &str = "nodot";
    const INVALID_KEY_CODE: &str = "gix::invalid-config-key";
    const UNKNOWN_TYPE_CODE: &str = "gix::unknown-config-type";
    const INVALID_VALUE_CODE: &str = "gix::invalid-config-value";

    /// The script `gix config get <key>` evaluated against the fixture input.
    fn get_script(key: &str) -> String {
        format!("{GET_COMMAND} {key}")
    }

    /// The script `gix config get <key> --type <ty>` evaluated against the fixture input.
    fn typed_script(key: &str, ty: &str) -> String {
        format!("{GET_COMMAND} {key} --{TYPE_FLAG} {ty}")
    }

    mod signature {
        use super::*;

        #[rstest]
        #[case::parent(Config, CONFIG_COMMAND)]
        #[case::get(ConfigGet, GET_COMMAND)]
        fn names_the_command<C>(#[case] command: C, #[case] expected: &str)
        where
            C: SimplePluginCommand<Plugin = GixPlugin>,
        {
            assert_eq!(command.signature().name, expected);
        }

        #[rstest]
        #[case::parent(Config)]
        #[case::get(ConfigGet)]
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
        fn transforms_a_repository_handle_into_a_typed_value_or_null() {
            let signature = SimplePluginCommand::signature(&ConfigGet);

            assert_eq!(
                signature.input_output_types,
                vec![(
                    Type::Custom(<Repository as Handle>::TYPE_NAME.into()),
                    Type::OneOf(Box::new([
                        Type::String,
                        Type::Int,
                        Type::Bool,
                        Type::Nothing
                    ])),
                )]
            );
        }

        #[rstest]
        fn takes_a_type_flag_with_a_short_form() {
            let signature = SimplePluginCommand::signature(&ConfigGet);

            assert_matches!(
                signature.get_long_flag(TYPE_FLAG),
                Some(flag) if flag.short == Some(TYPE_SHORT)
            );
        }

        #[rstest]
        fn prints_help_for_the_parent() {
            let signature = SimplePluginCommand::signature(&Config);

            assert_eq!(
                signature.input_output_types,
                vec![(Type::Nothing, Type::String)]
            );
        }
    }

    /// End-to-end tests: `gix config get` driven through the plugin protocol against the fixtures.
    mod eval {
        use super::*;

        mod read {
            use super::*;

            #[rstest]
            fn reads_a_local_value(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;

                append_config(repo.path(), LOCAL_SECTION)?;

                let value = eval_value(
                    &mut plugin,
                    &get_script(LOCAL_KEY),
                    repo_input(repo.path())?,
                )?;

                assert_eq!(value.as_int()?, LOCAL_VALUE);

                Ok(())
            }

            #[rstest]
            fn returns_null_for_an_unset_key(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;

                let value = eval_value(
                    &mut plugin,
                    &get_script(ABSENT_KEY),
                    repo_input(repo.path())?,
                )?;

                assert!(value.is_nothing(), "got {value:?}");

                Ok(())
            }
        }

        mod auto {
            use super::*;

            #[rstest]
            fn reads_a_yes_as_a_bool_without_type(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;

                append_config(repo.path(), TYPED_SECTION)?;

                let value =
                    eval_value(&mut plugin, &get_script(FLAG_KEY), repo_input(repo.path())?)?;

                assert!(value.as_bool()?);

                Ok(())
            }

            #[rstest]
            fn reads_a_word_as_a_string_without_type(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;

                append_config(repo.path(), TYPED_SECTION)?;

                let value =
                    eval_value(&mut plugin, &get_script(TEXT_KEY), repo_input(repo.path())?)?;

                assert_eq!(value.as_str()?, PLAIN_TEXT);

                Ok(())
            }
        }

        mod typed {
            use super::*;

            #[rstest]
            fn reads_a_yes_as_a_bool(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;

                append_config(repo.path(), TYPED_SECTION)?;

                let value = eval_value(
                    &mut plugin,
                    &typed_script(FLAG_KEY, BOOL_TYPE),
                    repo_input(repo.path())?,
                )?;

                assert!(value.as_bool()?);

                Ok(())
            }

            #[rstest]
            fn reads_a_kilobyte_suffix_as_an_int(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;

                append_config(repo.path(), TYPED_SECTION)?;

                let value = eval_value(
                    &mut plugin,
                    &typed_script(NUM_KEY, INT_TYPE),
                    repo_input(repo.path())?,
                )?;

                assert_eq!(value.as_int()?, KILOBYTES_VALUE);

                Ok(())
            }

            #[rstest]
            fn keeps_a_yes_as_the_string_with_type_string(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;

                append_config(repo.path(), TYPED_SECTION)?;

                let value = eval_value(
                    &mut plugin,
                    &typed_script(FLAG_KEY, STRING_TYPE),
                    repo_input(repo.path())?,
                )?;

                assert_eq!(value.as_str()?, "yes");

                Ok(())
            }

            #[rstest]
            fn reads_a_plain_value_as_a_path_string(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;

                append_config(repo.path(), TYPED_SECTION)?;

                let value = eval_value(
                    &mut plugin,
                    &typed_script(TEXT_KEY, PATH_TYPE),
                    repo_input(repo.path())?,
                )?;

                assert_eq!(value.as_str()?, PLAIN_TEXT);

                Ok(())
            }

            #[rstest]
            fn returns_null_for_an_unset_key(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;

                let value = eval_value(
                    &mut plugin,
                    &typed_script(ABSENT_KEY, INT_TYPE),
                    repo_input(repo.path())?,
                )?;

                assert!(value.is_nothing(), "got {value:?}");

                Ok(())
            }
        }

        mod errors {
            use super::*;

            #[rstest]
            fn rejects_an_invalid_key_before_looking_at_the_repository(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;
                let input = repo_input(repo.path())?;

                fs::remove_dir_all(repo.path())?;

                let labeled = eval_labeled_error(&mut plugin, &get_script(MALFORMED_KEY), input)?;

                assert_eq!(labeled.code.as_deref(), Some(INVALID_KEY_CODE));

                Ok(())
            }

            #[rstest]
            fn rejects_an_invalid_key_with_its_code_and_label(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;
                let script = get_script(MALFORMED_KEY);

                let labeled = eval_labeled_error(&mut plugin, &script, repo_input(repo.path())?)?;

                assert_eq!(labeled.code.as_deref(), Some(INVALID_KEY_CODE));
                assert_label_in_script!(&labeled, &script);

                assert!(
                    labeled.msg.contains(MALFORMED_KEY),
                    "message lacks the key: {}",
                    labeled.msg
                );

                Ok(())
            }

            #[rstest]
            fn rejects_an_unknown_type_before_looking_at_the_repository(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;
                let input = repo_input(repo.path())?;

                fs::remove_dir_all(repo.path())?;

                let script = typed_script(FLAG_KEY, UNKNOWN_TYPE);
                let labeled = eval_labeled_error(&mut plugin, &script, input)?;

                assert_eq!(labeled.code.as_deref(), Some(UNKNOWN_TYPE_CODE));

                Ok(())
            }

            #[rstest]
            fn rejects_an_unknown_type_with_its_code_and_label(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;
                let script = typed_script(FLAG_KEY, UNKNOWN_TYPE);

                let labeled = eval_labeled_error(&mut plugin, &script, repo_input(repo.path())?)?;

                assert_eq!(labeled.code.as_deref(), Some(UNKNOWN_TYPE_CODE));
                assert_label_in_script!(&labeled, &script);

                Ok(())
            }

            #[rstest]
            fn rejects_a_value_that_does_not_convert_to_the_type(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;

                append_config(repo.path(), TYPED_SECTION)?;

                let labeled = eval_labeled_error(
                    &mut plugin,
                    &typed_script(TEXT_KEY, INT_TYPE),
                    repo_input(repo.path())?,
                )?;

                assert_eq!(labeled.code.as_deref(), Some(INVALID_VALUE_CODE));

                Ok(())
            }
        }
    }
}
