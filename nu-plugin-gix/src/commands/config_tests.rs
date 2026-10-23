//! Tests of `gix config` and `gix config list`: signatures and command smoke tests.

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
const LOCAL_SECTION: &[u8] = b"[gixtest]\n\tanswer = 42\n";
const LOCAL_KEY: &str = "gixtest.answer";
const LOCAL_VALUE: &str = "42";
const IMPLICIT_SECTION: &[u8] = b"[gixtest]\n\timplicit\n";
const IMPLICIT_KEY: &str = "gixtest.implicit";
const VALUE_MISSING_TYPE: &str = "int";
const VALUE_MISSING_CODE: &str = "gix::config-value-missing";
const RESET_SECTION: &[u8] = b"[credential]\n\thelper = a\n\thelper =\n";
const RESET_KEY: &str = "credential.helper";
const KEY_RESET_CODE: &str = "gix::config-key-reset";
const NON_UTF8_SUBSECTION_SECTION: &[u8] = b"[user]\n\tname = Ada\n[remote \"\xff\"]\n\turl = x\n";
const SUBSECTION_NOT_UTF8_CODE: &str = "gix::config-subsection-not-utf8";
const NON_UTF8_VALUE_SECTION: &[u8] = b"[gixtest]\n\tbad = \xff\n";
const NON_UTF8_VALUE_KEY: &str = "gixtest.bad";
const NON_UTF8_VALUE_CODE: &str = "gix::config-value-not-utf8";
const KEY_NOT_FOUND_CODE: &str = "gix::config-key-not-found";
const UNKNOWN_TYPE: &str = "bogus";
const UNKNOWN_TYPE_CODE: &str = "gix::unknown-config-type";
const INVALID_VALUE_CODE: &str = "gix::invalid-config-value";
const TILDE_SECTION: &[u8] = b"[gixtest]\n\thome = ~/gixtest-probe\n";
const TILDE_KEY: &str = "gixtest.home";
const TILDE_SUFFIX: &str = "gixtest-probe";
const URL_VARIABLE: &str = "url";
const ORIGIN_SECTION: &[u8] = b"[remote \"origin\"]\n\turl = u1\n";
const ORIGIN_URL: &str = "u1";
const INSTEAD_OF_SECTION: &[u8] = b"[url \"https://a.b/\"]\n\tinsteadOf = gh:\n";
const VARIABLE_SCRIPT: &str = "gix config user.name";
const STRING_KEY_SCRIPT: &str = "let h = $in; let k = \"user.name\"; $h | gix config $k";
const DOTTED_STRING_SCRIPT: &str =
    "let h = $in; let k = \"url.https://a.b/.insteadof\"; $h | gix config $k";
const SIZE_SECTION: &[u8] = b"[gixtest]\n\tsize = 1k\n";
const SIZE_KEY: &str = "gixtest.size";
const SIZE_BYTES: i64 = 1024;
const ABSENT_SECTION: &str = "gixabsent";
const ORIGIN_KEY: &str = "remote.origin";
const NEEDS_KEY_CODE: &str = "gix::config-type-needs-key";
const NEEDS_VALUE_CODE: &str = "gix::config-type-needs-value";

/// The strings of a list value, in order.
fn string_list(value: &Value) -> Result<Vec<&str>, TestError> {
    let strings = value
        .as_list()?
        .iter()
        .map(Value::as_str)
        .collect::<Result<Vec<&str>, ShellError>>()?;

    Ok(strings)
}

/// The script `gix config <key>` evaluated against the fixture input.
fn key_script(key: &str) -> String {
    format!("{CONFIG_COMMAND} {key}")
}

/// The script `gix config --type <ty> <key>` evaluated against the fixture input.
fn type_script(key: &str, ty: &str) -> String {
    format!("{CONFIG_COMMAND} --{TYPE_FLAG} {ty} {key}")
}

mod signature {
    use super::*;

    #[rstest]
    #[case::parent(Config, CONFIG_COMMAND)]
    #[case::list(ConfigList, LIST_COMMAND)]
    fn names_the_command<C>(#[case] command: C, #[case] expected: &str)
    where
        C: SimplePluginCommand<Plugin = GixPlugin>,
    {
        assert_eq!(command.signature().name, expected);
    }

    #[rstest]
    #[case::parent(Config)]
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
    fn config_help_names_every_typed_or_listed_key() {
        for row in TABLE
            .iter()
            .filter(|row| row.kind != ValueKind::Text || row.multi != Multi::Single)
        {
            assert!(
                CONFIG_HELP.contains(&row.to_string()),
                "{row} is missing from the help"
            );
        }
    }

    #[rstest]
    fn config_help_has_no_stale_key() {
        let help = CONFIG_HELP.as_str();

        assert!(!help.contains("fetch.negotiationtip"));
        assert!(help.contains("remote.<name>.url"));
        assert!(help.contains("credential.[<url>.]helper"));
        assert!(help.contains("core.bare"));
    }

    #[rstest]
    fn list_transforms_a_repository_handle_into_a_table() {
        let signature = SimplePluginCommand::signature(&ConfigList);

        assert_eq!(
            signature.input_output_types,
            vec![(
                Type::Custom(<Repository as Handle>::TYPE_NAME.into()),
                Type::List(Box::new(Type::record())),
            )]
        );
    }

    #[rstest]
    fn config_takes_a_type_flag_with_a_short_form() -> Result<(), TestError> {
        let signature = SimplePluginCommand::signature(&Config);

        let Some(flag) = signature.get_long_flag(TYPE_FLAG) else {
            return Err(TestError::Failure(format!("no --{TYPE_FLAG} flag")));
        };

        assert_eq!(flag.short, Some(TYPE_SHORT));

        Ok(())
    }

    #[rstest]
    fn config_transforms_a_repository_handle_into_any_value() {
        let signature = SimplePluginCommand::signature(&Config);

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
}

mod help {
    use super::*;

    #[rstest]
    fn config_help_lists_every_path_key() {
        let help = CONFIG_HELP.as_str();

        for row in TABLE.iter().filter(|row| row.kind == ValueKind::Path) {
            assert!(
                help.contains(&row.to_string()),
                "{row} is missing from the help"
            );
        }
    }

    #[rstest]
    fn config_help_uses_git_placeholders() {
        let help = CONFIG_HELP.as_str();

        assert!(help.contains("remote.<name>.url"));
        assert!(help.contains("url.<base>.insteadof"));
        assert!(help.contains("credential.[<url>.]helper"));
        assert!(!help.contains("<subsection>"));
    }

    #[rstest]
    fn config_help_documents_the_key_forms() {
        let help = CONFIG_HELP.as_str();

        for text in [
            "cell path",
            "first and last dot",
            "--type",
            "gix::config-key-not-found",
            "gix::config-type-needs-key",
            "gix::config-type-needs-value",
        ] {
            assert!(help.contains(text), "{text} is missing from the help");
        }

        assert!(!help.contains("gix config get"));
        assert!(!help.contains("name-clash"));
    }
}

/// End-to-end smoke tests: the commands driven through the plugin protocol against the fixtures.
mod eval {
    use super::*;

    mod config_command {
        use super::*;

        #[rstest]
        fn reads_everything_as_a_nested_record(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), USER_SECTION)?;

            let value = eval_value(&mut plugin, CONFIG_COMMAND, repo_input(repo.path())?)?;

            assert_eq!(
                record_column(record_column(&value, "user")?, "name")?.as_str()?,
                USER_NAME
            );

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

            let value = eval_value(&mut plugin, CONFIG_COMMAND, repo_input(repo.path())?)?;

            assert!(record_column(record_column(&value, "core")?, "bare")?.as_bool()?);

            Ok(())
        }

        #[rstest]
        #[case::variable(VARIABLE_SCRIPT)]
        #[case::string_variable(STRING_KEY_SCRIPT)]
        fn reads_a_variable(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
            #[case] script: &str,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), USER_SECTION)?;

            let value = eval_value(&mut plugin, script, repo_input(repo.path())?)?;

            assert_eq!(value.as_str()?, USER_NAME);

            Ok(())
        }

        #[rstest]
        fn reads_a_section_as_a_record(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), BARE_SECTION)?;

            let value = eval_value(&mut plugin, &key_script("core"), repo_input(repo.path())?)?;

            assert!(record_column(&value, "bare")?.as_bool()?);

            Ok(())
        }

        #[rstest]
        fn reads_a_subsection_as_a_record(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), ORIGIN_SECTION)?;

            let value = eval_value(
                &mut plugin,
                &key_script("remote.origin"),
                repo_input(repo.path())?,
            )?;

            assert_eq!(
                string_list(record_column(&value, URL_VARIABLE)?)?,
                [ORIGIN_URL]
            );

            Ok(())
        }

        #[rstest]
        fn reads_a_three_member_variable(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), ORIGIN_SECTION)?;

            let value = eval_value(
                &mut plugin,
                &key_script("remote.origin.url"),
                repo_input(repo.path())?,
            )?;

            assert_eq!(string_list(&value)?, [ORIGIN_URL]);

            Ok(())
        }

        #[rstest]
        fn reads_a_dotted_subsection_from_a_string(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), INSTEAD_OF_SECTION)?;

            let value = eval_value(&mut plugin, DOTTED_STRING_SCRIPT, repo_input(repo.path())?)?;

            assert_eq!(string_list(&value)?, ["gh:"]);

            Ok(())
        }

        #[rstest]
        fn missing_key_is_an_error(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let script = key_script("gixabsent.x");

            let labeled = eval_labeled_error(&mut plugin, &script, repo_input(repo.path())?)?;

            assert_eq!(labeled.code.as_deref(), Some(KEY_NOT_FOUND_CODE));
            assert_label_in_script!(&labeled, &script);

            Ok(())
        }

        #[rstest]
        fn optional_member_gives_null(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            let value = eval_value(
                &mut plugin,
                &key_script("gixabsent.x?"),
                repo_input(repo.path())?,
            )?;

            assert!(value.is_nothing(), "got {value:?}");

            Ok(())
        }

        #[rstest]
        fn case_insensitive_marker_is_accepted(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), USER_SECTION)?;

            let value = eval_value(
                &mut plugin,
                &key_script("user!.name"),
                repo_input(repo.path())?,
            )?;

            assert_eq!(value.as_str()?, USER_NAME);

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
                &key_script(USER_NAME_KEY),
                repo_input(repo.path())?,
            )?;

            assert_eq!(value.as_str()?, NUMERIC_TEXT);

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
                &key_script(IMPLICIT_KEY),
                repo_input(repo.path())?,
            )?;

            assert!(value.is_nothing(), "got {value:?}");

            Ok(())
        }

        #[rstest]
        #[case::reset_key(RESET_KEY, RESET_SECTION, KEY_RESET_CODE)]
        #[case::non_utf8_value(NON_UTF8_VALUE_KEY, NON_UTF8_VALUE_SECTION, NON_UTF8_VALUE_CODE)]
        fn rejects_an_unreadable_key_with_its_code_and_label(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
            #[case] key: &str,
            #[case] section: &[u8],
            #[case] code: &str,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let script = key_script(key);

            append_config(repo.path(), section)?;

            let labeled = eval_labeled_error(&mut plugin, &script, repo_input(repo.path())?)?;

            assert_eq!(labeled.code.as_deref(), Some(code));
            assert_label_in_script!(&labeled, &script);

            Ok(())
        }
    }

    mod type_flag {
        use super::*;

        #[rstest]
        fn converts_with_git_units(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), SIZE_SECTION)?;

            let value = eval_value(
                &mut plugin,
                &type_script(SIZE_KEY, INT_TYPE),
                repo_input(repo.path())?,
            )?;

            assert_eq!(value.as_int()?, SIZE_BYTES);

            Ok(())
        }

        #[rstest]
        fn bool_type_converts_yes(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), BARE_SECTION)?;

            let value = eval_value(
                &mut plugin,
                &type_script(CORE_BARE_KEY, BOOL_TYPE),
                repo_input(repo.path())?,
            )?;

            assert!(value.as_bool()?, "got {value:?}");

            Ok(())
        }

        #[rstest]
        fn type_overrides_the_known_kind(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), BARE_SECTION)?;

            let value = eval_value(
                &mut plugin,
                &type_script(CORE_BARE_KEY, STRING_TYPE),
                repo_input(repo.path())?,
            )?;

            assert_eq!(value.as_str()?, "yes");

            Ok(())
        }

        #[rstest]
        fn type_path_expands_tilde(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), TILDE_SECTION)?;

            let value = eval_value(
                &mut plugin,
                &type_script(TILDE_KEY, PATH_TYPE),
                repo_input(repo.path())?,
            )?;

            let expanded = Path::new(value.as_str()?);

            assert!(expanded.is_absolute(), "got {value:?}");
            assert!(expanded.ends_with(TILDE_SUFFIX), "got {value:?}");

            Ok(())
        }

        #[rstest]
        fn type_on_a_bad_value_is_an_error(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), USER_SECTION)?;

            let script = type_script(USER_NAME_KEY, INT_TYPE);
            let labeled = eval_labeled_error(&mut plugin, &script, repo_input(repo.path())?)?;

            assert_eq!(labeled.code.as_deref(), Some(INVALID_VALUE_CODE));
            assert_label_in_script!(&labeled, &script);

            Ok(())
        }

        #[rstest]
        fn type_without_a_key_is_an_error(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let script = format!("{CONFIG_COMMAND} --{TYPE_FLAG} {INT_TYPE}");

            let labeled = eval_labeled_error(&mut plugin, &script, repo_input(repo.path())?)?;

            assert_eq!(labeled.code.as_deref(), Some(NEEDS_KEY_CODE));
            assert_label_in_script!(&labeled, &script);

            Ok(())
        }

        #[rstest]
        #[case::section(ABSENT_SECTION)]
        #[case::subsection(ORIGIN_KEY)]
        fn type_on_a_group_is_an_error(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
            #[case] key: &str,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), ORIGIN_SECTION)?;

            let script = type_script(key, INT_TYPE);
            let labeled = eval_labeled_error(&mut plugin, &script, repo_input(repo.path())?)?;

            assert_eq!(labeled.code.as_deref(), Some(NEEDS_VALUE_CODE));
            assert_label_in_script!(&labeled, &script);

            Ok(())
        }

        #[rstest]
        fn type_on_a_missing_key_is_an_error(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let script = type_script("gixabsent.x", INT_TYPE);

            let labeled = eval_labeled_error(&mut plugin, &script, repo_input(repo.path())?)?;

            assert_eq!(labeled.code.as_deref(), Some(KEY_NOT_FOUND_CODE));
            assert_label_in_script!(&labeled, &script);

            Ok(())
        }

        #[rstest]
        #[case::valueless_int(IMPLICIT_KEY, VALUE_MISSING_TYPE, VALUE_MISSING_CODE)]
        #[case::unknown_type(LOCAL_KEY, UNKNOWN_TYPE, UNKNOWN_TYPE_CODE)]
        fn rejects_a_bad_read_with_its_code_and_label(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
            #[case] key: &str,
            #[case] ty: &str,
            #[case] code: &str,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;
            let script = type_script(key, ty);

            append_config(repo.path(), IMPLICIT_SECTION)?;

            let labeled = eval_labeled_error(&mut plugin, &script, repo_input(repo.path())?)?;

            assert_eq!(labeled.code.as_deref(), Some(code));
            assert_label_in_script!(&labeled, &script);

            Ok(())
        }
    }

    mod list {
        use super::*;

        #[rstest]
        fn list_has_one_row_per_value_with_the_columns(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let repo = test_repo?;

            append_config(repo.path(), LOCAL_SECTION)?;

            let value = eval_value(&mut plugin, LIST_COMMAND, repo_input(repo.path())?)?;

            let mut found = false;

            for row in value.as_list()? {
                if record_column(row, "key")?.as_str()? == LOCAL_KEY {
                    found = true;

                    assert_eq!(record_column(row, "value")?.as_str()?, LOCAL_VALUE);
                    assert_eq!(record_column(row, "scope")?.as_str()?, "local");
                    record_column(row, "file")?;
                }
            }

            assert!(found, "no row for {LOCAL_KEY}");

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
