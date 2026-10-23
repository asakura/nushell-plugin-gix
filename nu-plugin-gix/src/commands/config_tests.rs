//! Tests of `gix config get` and `gix config list`: signatures and command smoke tests.

use super::*;

use crate::commands::config_known::{Multi, TABLE, ValueKind};
use crate::fixtures::{RepoInputError, plugin_test, repo_input};

use nu_plugin_gix_testing_support::{
    AssertionError, FixtureError, FixtureRepo as _, TestRepo, append_config,
    assert_label_in_script, eval_labeled_error, eval_value, record_column, test_repo,
};
use nu_plugin_test_support::PluginTest;
use nu_protocol::ShellError;
use rstest::rstest;

use std::{assert_matches, path::Path};

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

    #[error("Test failure: {0}")]
    Failure(String),
}

impl From<ShellError> for TestError {
    fn from(error: ShellError) -> Self {
        Self::Shell(Box::new(error))
    }
}

const USER_NAME_KEY: &str = "user.name";
const USER_NAME: &str = "Ada";
const USER_SECTION: &[u8] = b"[user]\n\tname = Ada\n";
const NUMERIC_TEXT_SECTION: &[u8] = b"[user]\n\tname = 1k\n";
const NUMERIC_TEXT: &str = "1k";
const CORE_BARE_KEY: &str = "core.bare";
const BARE_SECTION: &[u8] = b"[core]\n\tbare = yes\n";
const FALSE_BARE_SECTION: &[u8] = b"[core]\n\tbare = false\n";
const FALSE_TEXT: &str = "false";
const LOCAL_SECTION: &[u8] = b"[gixtest]\n\tanswer = 42\n";
const LOCAL_SECTION_NAME: &str = "gixtest";
const LOCAL_KEY: &str = "gixtest.answer";
const LOCAL_NAME: &str = "answer";
const LOCAL_VALUE: &str = "42";
const IMPLICIT_SECTION: &[u8] = b"[gixtest]\n\timplicit\n";
const IMPLICIT_KEY: &str = "gixtest.implicit";
const VALUE_MISSING_TYPE: &str = "int";
const VALUE_MISSING_CODE: &str = "gix::config-value-missing";
const REFSPEC_SECTION: &[u8] =
    b"[remote \"gixtest\"]\n\tfetch = +refs/heads/*:refs/remotes/gixtest/*\n";
const REMOTE_FETCH_KEY: &str = "remote.gixtest.fetch";
const REFSPEC: &str = "+refs/heads/*:refs/remotes/gixtest/*";
const RESET_SECTION: &[u8] = b"[credential]\n\thelper = a\n\thelper =\n";
const RESET_KEY: &str = "credential.helper";
const RESET_SECTION_NAME: &str = "credential";
const KEY_RESET_CODE: &str = "gix::config-key-reset";
const CLASH_SECTION: &[u8] = b"[a]\n\tx = 1\n[a \"x\"]\n\ty = 2\n";
const SUBSECTION_SECTION: &[u8] = b"[gixtest \"Sub\"]\n\tname = x\n";
const SUBSECTION_KEY: &str = "gixtest.Sub";
const SUBSECTION_VARIABLE: &str = "name";
const SUBSECTION_VALUE: &str = "x";
const CLASH_SECTION_NAME: &str = "a";
const CLASH_KEY: &str = "a.x";
const CLASH_VALUE: &str = "1";
const NAME_CLASH_CODE: &str = "gix::config-name-clash";
const NON_UTF8_SUBSECTION_SECTION: &[u8] = b"[user]\n\tname = Ada\n[remote \"\xff\"]\n\turl = x\n";
const SUBSECTION_NOT_UTF8_CODE: &str = "gix::config-subsection-not-utf8";
const NON_UTF8_VALUE_SECTION: &[u8] = b"[gixtest]\n\tbad = \xff\n";
const NON_UTF8_VALUE_KEY: &str = "gixtest.bad";
const NON_UTF8_VALUE_CODE: &str = "gix::config-value-not-utf8";
const MALFORMED_KEY: &str = "nodot";
const INVALID_KEY_CODE: &str = "gix::invalid-config-key";
const UNKNOWN_TYPE: &str = "bogus";
const UNKNOWN_TYPE_CODE: &str = "gix::unknown-config-type";
const CONVERSION_SECTION: &[u8] = b"[gixtest]\n\ttext = maybe\n";
const CONVERSION_KEY: &str = "gixtest.text";
const KNOWN_INT_BAD_SECTION: &[u8] = b"[gc]\n\tauto = banana\n";
const KNOWN_INT_BAD_KEY: &str = "gc.auto";
const INVALID_VALUE_CODE: &str = "gix::invalid-config-value";
const TILDE_SECTION: &[u8] = b"[gixtest]\n\thome = ~/gixtest-probe\n";
const TILDE_KEY: &str = "gixtest.home";
const TILDE_SUFFIX: &str = "gixtest-probe";
const SUBSECTION_DOT_SECTION: &[u8] = b"[remote \"a.b\"]\n\turl = dotted\n";
const SUBSECTION_DOT_GROUP: &str = "remote.\"a.b\"";
const SUBSECTION_DOT_VALUE: &str = "dotted";
const URL_VARIABLE: &str = "url";
const STRING_VARIABLE_SCRIPT: &str = "let h = $in; let k = \"user.name\"; $h | gix config get $k";
const QUOTED_WHOLE_SCRIPT: &str = "gix config get \"user.name\"";
const NUMERIC_MEMBER_KEY: &str = "remote.0";
const OPTIONAL_MEMBER_KEY: &str = "remote.a?";
const UNSUPPORTED_MEMBER_CODE: &str = "gix::config-unsupported-key-member";

/// The strings of a list value, in order.
fn string_list(value: &Value) -> Result<Vec<&str>, TestError> {
    let strings = value
        .as_list()?
        .iter()
        .map(Value::as_str)
        .collect::<Result<Vec<&str>, ShellError>>()?;

    Ok(strings)
}

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
    #[case::list(ConfigList, LIST_COMMAND)]
    fn names_the_command<C>(#[case] command: C, #[case] expected: &str)
    where
        C: SimplePluginCommand<Plugin = GixPlugin>,
    {
        assert_eq!(command.signature().name, expected);
    }

    #[rstest]
    #[case::parent(Config)]
    #[case::get(ConfigGet)]
    #[case::list(ConfigList)]
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
    fn get_help_names_every_typed_or_listed_key() {
        for row in TABLE
            .iter()
            .filter(|row| row.kind != ValueKind::Text || row.multi != Multi::Single)
        {
            assert!(
                ConfigGet.extra_description().contains(&row.to_string()),
                "{row} is missing from the help"
            );
        }
    }

    #[rstest]
    fn get_help_has_no_stale_key() {
        let help = ConfigGet.extra_description();

        assert!(!help.contains("fetch.negotiationtip"));
        assert!(help.contains("remote.<name>.url"));
        assert!(help.contains("credential.[<url>.]helper"));
        assert!(help.contains("core.bare"));
    }

    #[rstest]
    fn get_transforms_a_repository_handle_into_a_typed_value_or_null() {
        let signature = SimplePluginCommand::signature(&ConfigGet);

        assert_eq!(
            signature.input_output_types,
            vec![(
                Type::Custom(<Repository as Handle>::TYPE_NAME.into()),
                Type::OneOf(Box::new([
                    Type::String,
                    Type::Int,
                    Type::Bool,
                    Type::Nothing,
                    Type::record(),
                    Type::List(Box::new(Type::Any)),
                ])),
            )]
        );
    }

    #[rstest]
    fn list_transforms_a_repository_handle_into_a_record() {
        let signature = SimplePluginCommand::signature(&ConfigList);

        assert_eq!(
            signature.input_output_types,
            vec![(
                Type::Custom(<Repository as Handle>::TYPE_NAME.into()),
                Type::record(),
            )]
        );
    }

    #[rstest]
    fn get_takes_a_type_flag_with_a_short_form() -> Result<(), TestError> {
        let signature = SimplePluginCommand::signature(&ConfigGet);

        let Some(flag) = signature.get_long_flag(TYPE_FLAG) else {
            return Err(TestError::Failure(format!("no --{TYPE_FLAG} flag")));
        };

        assert_eq!(flag.short, Some(TYPE_SHORT));

        Ok(())
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

mod help {
    use super::*;

    #[rstest]
    fn get_help_lists_every_path_key() {
        let help = SimplePluginCommand::extra_description(&ConfigGet);

        for row in TABLE.iter().filter(|row| row.kind == ValueKind::Path) {
            assert!(
                help.contains(&row.to_string()),
                "{row} is missing from the help"
            );
        }
    }

    #[rstest]
    fn get_help_uses_git_placeholders() {
        let help = SimplePluginCommand::extra_description(&ConfigGet);

        assert!(help.contains("remote.<name>.url"));
        assert!(help.contains("url.<base>.insteadof"));
        assert!(help.contains("credential.[<url>.]helper"));
        assert!(!help.contains("<subsection>"));
    }
}

/// End-to-end smoke tests: the commands driven through the plugin protocol against the fixtures.
mod eval {
    use super::*;

    mod get {
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

            assert_eq!(value.as_str()?, LOCAL_VALUE);

            Ok(())
        }

        #[rstest]
        fn reads_a_user_name(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), USER_SECTION)?;

            let value = eval_value(
                &mut plugin,
                &get_script(USER_NAME_KEY),
                repo_input(repo.path())?,
            )?;

            assert_eq!(value.as_str()?, USER_NAME);

            Ok(())
        }

        #[rstest]
        fn reads_a_known_bool_as_a_bool(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), BARE_SECTION)?;

            let value = eval_value(
                &mut plugin,
                &get_script(CORE_BARE_KEY),
                repo_input(repo.path())?,
            )?;

            assert!(value.as_bool()?);

            Ok(())
        }

        #[rstest]
        fn keeps_an_unknown_key_as_text(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), NUMERIC_TEXT_SECTION)?;

            let value = eval_value(
                &mut plugin,
                &get_script(USER_NAME_KEY),
                repo_input(repo.path())?,
            )?;

            assert_eq!(value.as_str()?, NUMERIC_TEXT);

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
                &get_script(LOCAL_KEY),
                repo_input(repo.path())?,
            )?;

            assert!(value.is_nothing(), "got {value:?}");

            Ok(())
        }

        #[rstest]
        fn reads_a_valueless_key_as_null(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), IMPLICIT_SECTION)?;

            let value = eval_value(
                &mut plugin,
                &get_script(IMPLICIT_KEY),
                repo_input(repo.path())?,
            )?;

            assert!(value.is_nothing(), "got {value:?}");

            Ok(())
        }

        #[rstest]
        fn reads_a_multi_value_key_as_a_list(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), REFSPEC_SECTION)?;

            let value = eval_value(
                &mut plugin,
                &get_script(REMOTE_FETCH_KEY),
                repo_input(repo.path())?,
            )?;

            assert_eq!(string_list(&value)?, vec![REFSPEC]);

            Ok(())
        }

        #[rstest]
        fn reads_a_section_as_a_record(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), LOCAL_SECTION)?;

            let value = eval_value(
                &mut plugin,
                &get_script(LOCAL_SECTION_NAME),
                repo_input(repo.path())?,
            )?;

            assert_eq!(record_column(&value, LOCAL_NAME)?.as_str()?, LOCAL_VALUE);

            Ok(())
        }

        #[rstest]
        fn reads_a_section_of_only_reset_keys_as_an_empty_list(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), RESET_SECTION)?;

            let value = eval_value(
                &mut plugin,
                &get_script(RESET_SECTION_NAME),
                repo_input(repo.path())?,
            )?;

            assert_eq!(string_list(&value)?, Vec::<&str>::new());

            Ok(())
        }

        #[rstest]
        fn reads_a_subsection_as_a_record(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), SUBSECTION_SECTION)?;

            let value = eval_value(
                &mut plugin,
                &get_script(SUBSECTION_KEY),
                repo_input(repo.path())?,
            )?;

            assert_eq!(
                record_column(&value, SUBSECTION_VARIABLE)?.as_str()?,
                SUBSECTION_VALUE
            );

            Ok(())
        }

        #[rstest]
        fn reads_a_key_as_a_string_with_type_string(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), FALSE_BARE_SECTION)?;

            let value = eval_value(
                &mut plugin,
                &typed_script(CORE_BARE_KEY, STRING_TYPE),
                repo_input(repo.path())?,
            )?;

            assert_eq!(value.as_str()?, FALSE_TEXT);

            Ok(())
        }

        #[rstest]
        fn reads_a_key_past_a_non_utf8_subsection(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), NON_UTF8_SUBSECTION_SECTION)?;

            let value = eval_value(
                &mut plugin,
                &get_script(USER_NAME_KEY),
                repo_input(repo.path())?,
            )?;

            assert_eq!(value.as_str()?, USER_NAME);

            Ok(())
        }

        #[rstest]
        fn reads_a_variable_of_a_clashing_section(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), CLASH_SECTION)?;

            let value = eval_value(
                &mut plugin,
                &get_script(CLASH_KEY),
                repo_input(repo.path())?,
            )?;

            assert_eq!(value.as_str()?, CLASH_VALUE);

            Ok(())
        }

        #[rstest]
        fn rejects_a_reset_key_with_its_code_and_label(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let script = get_script(RESET_KEY);

            append_config(repo.path(), RESET_SECTION)?;

            let labeled = eval_labeled_error(&mut plugin, &script, repo_input(repo.path())?)?;

            assert_eq!(labeled.code.as_deref(), Some(KEY_RESET_CODE));
            assert_label_in_script!(&labeled, &script);

            Ok(())
        }

        #[rstest]
        fn rejects_a_valueless_key_read_as_an_int_with_its_code_and_label(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let script = typed_script(IMPLICIT_KEY, VALUE_MISSING_TYPE);

            append_config(repo.path(), IMPLICIT_SECTION)?;

            let labeled = eval_labeled_error(&mut plugin, &script, repo_input(repo.path())?)?;

            assert_eq!(labeled.code.as_deref(), Some(VALUE_MISSING_CODE));
            assert_label_in_script!(&labeled, &script);

            Ok(())
        }

        #[rstest]
        fn rejects_a_section_with_a_name_clash_with_its_code_and_label(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let script = get_script(CLASH_SECTION_NAME);

            append_config(repo.path(), CLASH_SECTION)?;

            let labeled = eval_labeled_error(&mut plugin, &script, repo_input(repo.path())?)?;

            assert_eq!(labeled.code.as_deref(), Some(NAME_CLASH_CODE));
            assert_label_in_script!(&labeled, &script);

            Ok(())
        }

        #[rstest]
        fn rejects_a_non_utf8_value_with_its_code_and_label(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let script = get_script(NON_UTF8_VALUE_KEY);

            append_config(repo.path(), NON_UTF8_VALUE_SECTION)?;

            let labeled = eval_labeled_error(&mut plugin, &script, repo_input(repo.path())?)?;

            assert_eq!(labeled.code.as_deref(), Some(NON_UTF8_VALUE_CODE));
            assert_label_in_script!(&labeled, &script);

            Ok(())
        }

        #[rstest]
        fn rejects_an_invalid_key_with_its_code_and_label(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let script = typed_script(MALFORMED_KEY, VALUE_MISSING_TYPE);
            let labeled = eval_labeled_error(&mut plugin, &script, repo_input(repo.path())?)?;

            assert_eq!(labeled.code.as_deref(), Some(INVALID_KEY_CODE));
            assert_label_in_script!(&labeled, &script);

            Ok(())
        }

        #[rstest]
        fn rejects_an_unknown_type_with_its_code_and_label(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let script = typed_script(LOCAL_KEY, UNKNOWN_TYPE);
            let labeled = eval_labeled_error(&mut plugin, &script, repo_input(repo.path())?)?;

            assert_eq!(labeled.code.as_deref(), Some(UNKNOWN_TYPE_CODE));
            assert_label_in_script!(&labeled, &script);

            Ok(())
        }

        #[rstest]
        fn rejects_a_value_that_does_not_convert_to_the_type_with_its_code_and_label(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let script = typed_script(CONVERSION_KEY, INT_TYPE);

            append_config(repo.path(), CONVERSION_SECTION)?;

            let labeled = eval_labeled_error(&mut plugin, &script, repo_input(repo.path())?)?;

            assert_eq!(labeled.code.as_deref(), Some(INVALID_VALUE_CODE));
            assert!(labeled.msg.contains(CONVERSION_KEY), "got {labeled:?}");
            assert_label_in_script!(&labeled, &script);

            Ok(())
        }

        #[rstest]
        fn rejects_an_unknown_type_before_an_invalid_key(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let script = typed_script(MALFORMED_KEY, UNKNOWN_TYPE);
            let labeled = eval_labeled_error(&mut plugin, &script, repo_input(repo.path())?)?;

            assert_eq!(labeled.code.as_deref(), Some(UNKNOWN_TYPE_CODE));
            assert_label_in_script!(&labeled, &script);

            Ok(())
        }

        #[rstest]
        fn expands_a_tilde_path_with_the_home_context(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), TILDE_SECTION)?;

            let value = eval_value(
                &mut plugin,
                &typed_script(TILDE_KEY, PATH_TYPE),
                repo_input(repo.path())?,
            )?;

            let expanded = Path::new(value.as_str()?);

            assert!(expanded.is_absolute(), "got {value:?}");
            assert!(expanded.ends_with(TILDE_SUFFIX), "got {value:?}");

            Ok(())
        }

        mod cell_path {
            use super::*;

            #[rstest]
            fn reads_two_members_with_a_dotted_second_as_a_group(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;

                append_config(repo.path(), SUBSECTION_DOT_SECTION)?;

                let value = eval_value(
                    &mut plugin,
                    &get_script(SUBSECTION_DOT_GROUP),
                    repo_input(repo.path())?,
                )?;

                assert_eq!(
                    string_list(record_column(&value, URL_VARIABLE)?)?,
                    [SUBSECTION_DOT_VALUE]
                );

                Ok(())
            }

            #[rstest]
            fn reads_a_key_from_a_string_variable(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;

                append_config(repo.path(), USER_SECTION)?;

                let value = eval_value(
                    &mut plugin,
                    STRING_VARIABLE_SCRIPT,
                    repo_input(repo.path())?,
                )?;

                assert_eq!(value.as_str()?, USER_NAME);

                Ok(())
            }

            #[rstest]
            fn reads_a_quoted_whole_key_as_git_text(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;

                append_config(repo.path(), USER_SECTION)?;

                let value = eval_value(&mut plugin, QUOTED_WHOLE_SCRIPT, repo_input(repo.path())?)?;

                assert_eq!(value.as_str()?, USER_NAME);

                Ok(())
            }

            #[rstest]
            #[case::numeric(NUMERIC_MEMBER_KEY)]
            #[case::optional(OPTIONAL_MEMBER_KEY)]
            fn rejects_a_non_string_member_with_its_code_and_label(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
                #[case] key: &str,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;
                let script = get_script(key);
                let labeled = eval_labeled_error(&mut plugin, &script, repo_input(repo.path())?)?;

                assert_eq!(labeled.code.as_deref(), Some(UNSUPPORTED_MEMBER_CODE));
                assert_label_in_script!(&labeled, &script);

                Ok(())
            }
        }
    }

    mod list {
        use super::*;

        #[rstest]
        fn nests_a_local_entry_under_its_section(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), LOCAL_SECTION)?;

            let value = eval_value(&mut plugin, LIST_COMMAND, repo_input(repo.path())?)?;

            assert_eq!(
                record_column(record_column(&value, LOCAL_SECTION_NAME)?, LOCAL_NAME)?.as_str()?,
                LOCAL_VALUE
            );

            Ok(())
        }

        #[rstest]
        fn rejects_a_name_clash_with_its_code_and_label_at_the_handle(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let input = repo_input(repo.path())?;
            let handle_span = input.span();

            append_config(repo.path(), CLASH_SECTION)?;

            let labeled = eval_labeled_error(&mut plugin, LIST_COMMAND, input)?;

            assert_eq!(labeled.code.as_deref(), Some(NAME_CLASH_CODE));
            assert_eq!(labeled.labels.first().map(|label| label.span), handle_span);

            Ok(())
        }

        #[rstest]
        fn rejects_a_known_key_value_that_does_not_convert_with_its_code(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), KNOWN_INT_BAD_SECTION)?;

            let labeled = eval_labeled_error(&mut plugin, LIST_COMMAND, repo_input(repo.path())?)?;

            assert_eq!(labeled.code.as_deref(), Some(INVALID_VALUE_CODE));
            assert!(labeled.msg.contains(KNOWN_INT_BAD_KEY), "got {labeled:?}");

            Ok(())
        }

        #[rstest]
        fn rejects_a_non_utf8_subsection_with_its_code_and_label_at_the_handle(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let input = repo_input(repo.path())?;
            let handle_span = input.span();

            append_config(repo.path(), NON_UTF8_SUBSECTION_SECTION)?;

            let labeled = eval_labeled_error(&mut plugin, LIST_COMMAND, input)?;

            assert_eq!(labeled.code.as_deref(), Some(SUBSECTION_NOT_UTF8_CODE));
            assert_eq!(labeled.labels.first().map(|label| label.span), handle_span);

            Ok(())
        }
    }
}
