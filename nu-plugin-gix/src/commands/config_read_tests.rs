//! Tests of the configuration read policy: resolution, untyped and typed reads, and the section, subsection and whole-configuration records.

use super::*;

use crate::commands::config_key::key_for_test;
use crate::commands::config_known::SubsectionRule;

use nu_plugin_gix_testing_support::{
    AssertionError, FixtureError, TestRepo, record_column, test_repo,
};
use nu_protocol::{ShellError, record};
use rstest::{fixture, rstest};

use gix::config::Source;

use std::{assert_matches, iter};

#[derive(thiserror::Error, Debug)]
enum TestError {
    #[error(transparent)]
    Gix(Box<git::GixError>),

    #[error(transparent)]
    Fixture(Box<FixtureError>),

    #[error(transparent)]
    Assertion(Box<AssertionError>),

    #[error(transparent)]
    Shell(Box<ShellError>),

    #[error("Test failure: {0}")]
    Failure(String),
}

impl From<git::GixError> for TestError {
    fn from(error: git::GixError) -> Self {
        Self::Gix(Box::new(error))
    }
}

impl From<FixtureError> for TestError {
    fn from(error: FixtureError) -> Self {
        Self::Fixture(Box::new(error))
    }
}

impl From<AssertionError> for TestError {
    fn from(error: AssertionError) -> Self {
        Self::Assertion(Box::new(error))
    }
}

impl From<ShellError> for TestError {
    fn from(error: ShellError) -> Self {
        Self::Shell(Box::new(error))
    }
}

impl From<git::GixError> for Box<TestError> {
    fn from(error: git::GixError) -> Self {
        Self::from(TestError::from(error))
    }
}

impl From<FixtureError> for Box<TestError> {
    fn from(error: FixtureError) -> Self {
        Self::from(TestError::from(error))
    }
}

impl From<AssertionError> for Box<TestError> {
    fn from(error: AssertionError) -> Self {
        Self::from(TestError::from(error))
    }
}

impl From<ShellError> for Box<TestError> {
    fn from(error: ShellError) -> Self {
        Self::from(TestError::from(error))
    }
}

type TestResult = Result<(), Box<TestError>>;

const SPAN: Span = Span::test_data();
const FIRST_FILE: &str = "/gixtest/first";
const SECOND_FILE: &str = "/gixtest/second";
const FIRST_VALUE: &str = "first";
const SECOND_VALUE: &str = "second";
const TRUE_TEXT: &str = "true";
const BANANA_TEXT: &str = "banana";
const KILOBYTE_TEXT: &str = "1k";
const KILOBYTES: i64 = 1024;
const TILDE_TEXT: &str = "~/x";
const ABSOLUTE_PATH: &str = "/gixtest/path";
const NON_UTF8_BYTES: &[u8] = b"\xff";
const NON_UTF8_PATH: &[u8] = b"/gixtest/\xff";
const BARE_KEY: &str = "core.bare";
const AUTO_KEY: &str = "gc.auto";
const NAME_KEY: &str = "user.name";
const INCLUDE_KEY: &str = "include.path";
const HELPER_KEY: &str = "credential.helper";
const HELPER: &str = "helper";
const OTHER_VALUE: &str = "x";
const LIST_SECTION: &str = "gixtest";
const LIST_NAME: &str = "list";
const CREDENTIAL: &str = "credential";
const USER: &str = "user";
const ADA: &str = "Ada";
const REMOTE: &str = "remote";
const ORIGIN: &str = "origin";
const URL: &str = "url";
const URL_VALUE: &str = "x";
const ONE: &str = "1";
const TWO: &str = "2";
const EMPTY_LIST: &str = "a list needs at least one occurrence";

fn origin_at(path: &str) -> Arc<git::Origin> {
    Arc::new(git::Origin {
        path: Some(PathBuf::from(path)),
        source: Source::Local,
    })
}

fn occurrence(value: git::RawValue, origin: &Arc<git::Origin>) -> git::Occurrence {
    git::Occurrence {
        value,
        origin: Arc::clone(origin),
        trusted: true,
    }
}

fn untrusted(value: git::RawValue, origin: &Arc<git::Origin>) -> git::Occurrence {
    git::Occurrence {
        trusted: false,
        ..occurrence(value, origin)
    }
}

fn text(value: &str) -> git::RawValue {
    git::RawValue::Bytes(value.as_bytes().to_owned())
}

fn keyed(
    text: &str,
    occurrence: git::Occurrence,
) -> Result<(git::ConfigKey, git::Occurrence), TestError> {
    Ok((key_for_test(text)?, occurrence))
}

fn untyped(text: &str, values: &Values<'_>) -> Result<Value, TestError> {
    Ok(untyped_value(&key_for_test(text)?, values, SPAN)?)
}

fn list_of(values: Vec<&git::Occurrence>) -> Result<Values<'_>, Box<TestError>> {
    Listed::new(values)
        .map(Values::List)
        .ok_or_else(|| Box::new(TestError::Failure(EMPTY_LIST.to_owned())))
}

fn row(multi: Multi) -> KnownKey {
    KnownKey {
        section: LIST_SECTION,
        subsection: SubsectionRule::Absent,
        name: LIST_NAME,
        kind: ValueKind::Text,
        multi,
    }
}

#[fixture]
fn path_context() -> Result<(TestRepo, git::PathContext), Box<TestError>> {
    let repo = test_repo()?;
    let location = git::RepositoryLocation::discover(repo.dir.path())?;
    let opened = git::OpenRepository::try_from(&location)?;
    let paths = git::PathContext::of(&opened);

    Ok((repo, paths))
}

mod resolve {
    use super::*;

    #[rstest]
    fn scalar_repeated_takes_last() {
        let first = occurrence(text(FIRST_VALUE), &origin_at(FIRST_FILE));
        let last = occurrence(text(SECOND_VALUE), &origin_at(SECOND_FILE));

        let resolved = resolve(None, [&first, &last].into_iter());

        assert_matches!(resolved, Some(Resolved::Values(Values::Single(found))) if *found == last);
    }

    #[rstest]
    fn list_with_reset_keeps_entries_after_the_reset() {
        let origin = origin_at(FIRST_FILE);
        let first = occurrence(text(FIRST_VALUE), &origin);
        let reset = occurrence(text(""), &origin);
        let last = occurrence(text(SECOND_VALUE), &origin);
        let list = row(Multi::ListWithReset);

        let resolved = resolve(Some(&list), [&first, &reset, &last].into_iter());

        assert_matches!(
            resolved,
            Some(Resolved::Values(Values::List(found)))
                if found.iter().collect::<Vec<_>>() == vec![&last]
        );
    }

    #[rstest]
    fn list_with_reset_ends_in_reset_when_last_value_is_empty() {
        let origin = origin_at(FIRST_FILE);
        let first = occurrence(text(FIRST_VALUE), &origin);
        let reset = occurrence(text(""), &origin);
        let list = row(Multi::ListWithReset);

        let resolved = resolve(Some(&list), [&first, &reset].into_iter());

        assert_matches!(resolved, Some(Resolved::Reset(found)) if *found == reset);
    }

    #[rstest]
    fn list_with_reset_does_not_reset_on_valueless() {
        let origin = origin_at(FIRST_FILE);
        let first = occurrence(text(FIRST_VALUE), &origin);
        let valueless = occurrence(git::RawValue::Valueless, &origin);
        let list = row(Multi::ListWithReset);

        let resolved = resolve(Some(&list), [&first, &valueless].into_iter());

        assert_matches!(
            resolved,
            Some(Resolved::Values(Values::List(found)))
                if found.iter().collect::<Vec<_>>() == vec![&first, &valueless]
        );
    }

    #[rstest]
    fn list_keeps_every_value_in_order() {
        let origin = origin_at(FIRST_FILE);
        let first = occurrence(text(FIRST_VALUE), &origin);
        let empty = occurrence(text(""), &origin);
        let last = occurrence(text(SECOND_VALUE), &origin);
        let list = row(Multi::List);

        let resolved = resolve(Some(&list), [&first, &empty, &last].into_iter());

        assert_matches!(
            resolved,
            Some(Resolved::Values(Values::List(found)))
                if found.iter().collect::<Vec<_>>() == vec![&first, &empty, &last]
        );
    }

    #[rstest]
    fn list_keeps_a_lone_empty_value() {
        let empty = occurrence(text(""), &origin_at(FIRST_FILE));
        let list = row(Multi::List);

        let resolved = resolve(Some(&list), [&empty].into_iter());

        assert_matches!(
            resolved,
            Some(Resolved::Values(Values::List(found)))
                if found.iter().collect::<Vec<_>>() == vec![&empty]
        );
    }

    #[rstest]
    fn no_occurrence_is_none() {
        let list = row(Multi::ListWithReset);

        let resolved = resolve(Some(&list), iter::empty::<&git::Occurrence>());

        assert_matches!(resolved, None);
    }
}

mod listed {
    use super::*;

    #[rstest]
    fn new_is_none_for_an_empty_vec() {
        assert_matches!(Listed::new(Vec::new()), None);
    }

    #[rstest]
    fn iter_yields_init_then_last() -> TestResult {
        let origin = origin_at(FIRST_FILE);
        let first = occurrence(text(FIRST_VALUE), &origin);
        let second = occurrence(text(OTHER_VALUE), &origin);
        let last = occurrence(text(SECOND_VALUE), &origin);
        let listed = Listed::new(vec![&first, &second, &last])
            .ok_or_else(|| Box::new(TestError::Failure(EMPTY_LIST.to_owned())))?;

        assert_eq!(
            listed.iter().collect::<Vec<_>>(),
            vec![&first, &second, &last]
        );

        Ok(())
    }
}

mod values {
    use super::*;

    mod attributed {
        use super::*;

        #[rstest]
        fn single_is_its_occurrence() {
            let origin = origin_at(FIRST_FILE);
            let only = occurrence(text(FIRST_VALUE), &origin);
            let values = Values::Single(&only);

            assert_eq!(values.attributed(), &only);
        }

        #[rstest]
        fn list_is_its_last_element() -> TestResult {
            let origin = origin_at(FIRST_FILE);
            let first = occurrence(text(FIRST_VALUE), &origin);
            let last = occurrence(text(SECOND_VALUE), &origin);
            let values = list_of(vec![&first, &last])?;

            assert_eq!(values.attributed(), &last);

            Ok(())
        }
    }
}

mod untyped_value {
    use super::*;

    #[rstest]
    fn valueless_known_bool_is_null() -> TestResult {
        let bare = occurrence(git::RawValue::Valueless, &origin_at(FIRST_FILE));

        let value = untyped(BARE_KEY, &Values::Single(&bare))?;

        assert_eq!(value, Value::nothing(SPAN));

        Ok(())
    }

    #[rstest]
    fn known_bool_is_bool() -> TestResult {
        let bare = occurrence(text(TRUE_TEXT), &origin_at(FIRST_FILE));

        let value = untyped(BARE_KEY, &Values::Single(&bare))?;

        assert_eq!(value, Value::test_bool(true));

        Ok(())
    }

    #[rstest]
    fn known_int_is_int() -> TestResult {
        let auto = occurrence(text(KILOBYTE_TEXT), &origin_at(FIRST_FILE));

        let value = untyped(AUTO_KEY, &Values::Single(&auto))?;

        assert_eq!(value, Value::test_int(KILOBYTES));

        Ok(())
    }

    #[rstest]
    fn unknown_key_stays_text() -> TestResult {
        let name = occurrence(text(KILOBYTE_TEXT), &origin_at(FIRST_FILE));

        let value = untyped(NAME_KEY, &Values::Single(&name))?;

        assert_eq!(value, Value::test_string(KILOBYTE_TEXT));

        Ok(())
    }

    #[rstest]
    fn path_kind_key_stays_raw_text() -> TestResult {
        let include = occurrence(text(TILDE_TEXT), &origin_at(FIRST_FILE));

        let value = untyped(INCLUDE_KEY, &Values::Single(&include))?;

        assert_eq!(value, Value::test_string(TILDE_TEXT));

        Ok(())
    }

    #[rstest]
    fn known_bool_that_does_not_convert_is_an_error() {
        let bare = occurrence(text(BANANA_TEXT), &origin_at(FIRST_FILE));

        let result = untyped(BARE_KEY, &Values::Single(&bare));

        assert_matches!(
            result,
            Err(TestError::Gix(error))
                if matches!(
                    error.as_ref(),
                    git::GixError::ConfigValueConversion { origin, .. }
                        if origin.path == Some(PathBuf::from(FIRST_FILE))
                )
        );
    }

    #[rstest]
    fn text_that_is_not_utf8_is_an_error() {
        let name = occurrence(
            git::RawValue::Bytes(NON_UTF8_BYTES.to_owned()),
            &origin_at(FIRST_FILE),
        );

        let result = untyped(NAME_KEY, &Values::Single(&name));

        assert_matches!(
            result,
            Err(TestError::Gix(error))
                if matches!(error.as_ref(), git::GixError::ConfigValueNotUtf8 { .. })
        );
    }

    #[rstest]
    fn valueless_element_of_a_list_is_null_in_place() -> TestResult {
        let origin = origin_at(FIRST_FILE);
        let valueless = occurrence(git::RawValue::Valueless, &origin);
        let value = occurrence(text(OTHER_VALUE), &origin);

        let list = untyped(NAME_KEY, &list_of(vec![&valueless, &value])?)?;

        assert_eq!(
            list,
            Value::list(
                vec![Value::nothing(SPAN), Value::test_string(OTHER_VALUE)],
                SPAN
            )
        );

        Ok(())
    }
}

mod typed_setting {
    use super::*;

    fn typed(
        config: &git::Config,
        ty: git::ConfigType,
        paths: &git::PathContext,
    ) -> Result<Result<Value, git::GixError>, Box<TestError>> {
        let tree = config.tree();
        let setting = tree
            .settings()
            .into_iter()
            .next()
            .ok_or_else(|| TestError::Failure("the config has no setting".to_owned()))?;

        Ok(typed_setting(setting, ty, paths, SPAN))
    }

    #[rstest]
    fn valueless_bool_is_true(
        path_context: Result<(TestRepo, git::PathContext), Box<TestError>>,
    ) -> TestResult {
        let (_repo, paths) = path_context?;
        let config = git::Config::from_occurrences([keyed(
            BARE_KEY,
            occurrence(git::RawValue::Valueless, &origin_at(FIRST_FILE)),
        )?]);

        let value = typed(
            &config,
            git::ConfigType::Scalar(git::ScalarType::Bool),
            &paths,
        )??;

        assert_eq!(value, Value::test_bool(true));

        Ok(())
    }

    #[rstest]
    #[case::int(AUTO_KEY, git::ConfigType::Scalar(git::ScalarType::Int))]
    #[case::string(NAME_KEY, git::ConfigType::Scalar(git::ScalarType::String))]
    #[case::path(INCLUDE_KEY, git::ConfigType::Path)]
    fn valueless_is_missing(
        path_context: Result<(TestRepo, git::PathContext), Box<TestError>>,
        #[case] key: &str,
        #[case] ty: git::ConfigType,
    ) -> TestResult {
        let (_repo, paths) = path_context?;
        let config = git::Config::from_occurrences([keyed(
            key,
            occurrence(git::RawValue::Valueless, &origin_at(FIRST_FILE)),
        )?]);

        let result = typed(&config, ty, &paths)?;

        assert_matches!(result, Err(git::GixError::ConfigValueMissing { .. }));

        Ok(())
    }

    #[rstest]
    fn reset_is_an_error_naming_the_emptying_value(
        path_context: Result<(TestRepo, git::PathContext), Box<TestError>>,
    ) -> TestResult {
        let (_repo, paths) = path_context?;
        let config = git::Config::from_occurrences([
            keyed(
                HELPER_KEY,
                occurrence(text(FIRST_VALUE), &origin_at(FIRST_FILE)),
            )?,
            keyed(HELPER_KEY, occurrence(text(""), &origin_at(SECOND_FILE)))?,
        ]);

        let result = typed(
            &config,
            git::ConfigType::Scalar(git::ScalarType::String),
            &paths,
        )?;

        assert_matches!(
            result,
            Err(git::GixError::ConfigKeyReset { origin, .. })
                if origin.path == Some(PathBuf::from(SECOND_FILE))
        );

        Ok(())
    }

    #[rstest]
    fn list_reads_its_last_element(
        path_context: Result<(TestRepo, git::PathContext), Box<TestError>>,
    ) -> TestResult {
        let (_repo, paths) = path_context?;
        let config = git::Config::from_occurrences([
            keyed(
                HELPER_KEY,
                occurrence(text(FIRST_VALUE), &origin_at(FIRST_FILE)),
            )?,
            keyed(
                HELPER_KEY,
                occurrence(text(SECOND_VALUE), &origin_at(SECOND_FILE)),
            )?,
        ]);

        let value = typed(
            &config,
            git::ConfigType::Scalar(git::ScalarType::String),
            &paths,
        )??;

        assert_eq!(value, Value::test_string(SECOND_VALUE));

        Ok(())
    }

    #[rstest]
    fn path_ignores_untrusted_value(
        path_context: Result<(TestRepo, git::PathContext), Box<TestError>>,
    ) -> TestResult {
        let (_repo, paths) = path_context?;
        let config = git::Config::from_occurrences([keyed(
            INCLUDE_KEY,
            untrusted(text(ABSOLUTE_PATH), &origin_at(FIRST_FILE)),
        )?]);

        let value = typed(&config, git::ConfigType::Path, &paths)??;

        assert_eq!(value, Value::nothing(SPAN));

        Ok(())
    }

    #[rstest]
    fn untrusted_reset_keeps_trusted_path(
        path_context: Result<(TestRepo, git::PathContext), Box<TestError>>,
    ) -> TestResult {
        let (_repo, paths) = path_context?;
        let config = git::Config::from_occurrences([
            keyed(
                HELPER_KEY,
                occurrence(text(ABSOLUTE_PATH), &origin_at(FIRST_FILE)),
            )?,
            keyed(HELPER_KEY, untrusted(text(""), &origin_at(SECOND_FILE)))?,
        ]);

        let value = typed(&config, git::ConfigType::Path, &paths)??;

        assert_eq!(value, Value::test_string(ABSOLUTE_PATH));

        Ok(())
    }

    #[rstest]
    fn path_that_is_not_utf8_is_an_error(
        path_context: Result<(TestRepo, git::PathContext), Box<TestError>>,
    ) -> TestResult {
        let (_repo, paths) = path_context?;
        let config = git::Config::from_occurrences([keyed(
            INCLUDE_KEY,
            occurrence(
                git::RawValue::Bytes(NON_UTF8_PATH.to_owned()),
                &origin_at(FIRST_FILE),
            ),
        )?]);

        let result = typed(&config, git::ConfigType::Path, &paths)?;

        assert_matches!(result, Err(git::GixError::ConfigPathNotUtf8 { .. }));

        Ok(())
    }

    #[rstest]
    fn untrusted_scalar_still_counts(
        path_context: Result<(TestRepo, git::PathContext), Box<TestError>>,
    ) -> TestResult {
        let (_repo, paths) = path_context?;
        let config = git::Config::from_occurrences([keyed(
            BARE_KEY,
            untrusted(text(TRUE_TEXT), &origin_at(FIRST_FILE)),
        )?]);

        let value = typed(
            &config,
            git::ConfigType::Scalar(git::ScalarType::Bool),
            &paths,
        )??;

        assert_eq!(value, Value::test_bool(true));

        Ok(())
    }
}

mod rows {
    use super::*;

    const REMOTE_ORIGIN_URL_KEY: &str = "remote.Origin.url";
    const REMOTE_ORIGIN_URL_ROW: &str = "remote.Origin.url";
    const BOB: &str = "Bob";
    const KEY_COLUMN: &str = "key";
    const VALUE_COLUMN: &str = "value";
    const SCOPE_COLUMN: &str = "scope";
    const FILE_COLUMN: &str = "file";
    const HIDDEN_KEY: &str = "a.x";
    const HIDDEN_SUBSECTION_KEY: &str = "a.x.y";
    const UNKNOWN_KEY: &str = "gixtest.unknown";
    const EMPTY_TEXT: &str = "";
    const FIVE: &str = "5";
    const LOCAL_SCOPE: &str = "local";
    const COMMAND_SCOPE: &str = "command";
    fn texts(rows: &[Value], name: &str) -> Result<Vec<String>, Box<TestError>> {
        rows.iter()
            .map(|row| -> Result<String, Box<TestError>> {
                Ok(record_column(row, name)?.as_str()?.to_owned())
            })
            .collect()
    }

    #[rstest]
    fn key_form() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(
                BARE_KEY,
                occurrence(text(TRUE_TEXT), &origin_at(FIRST_FILE)),
            )?,
            keyed(
                REMOTE_ORIGIN_URL_KEY,
                occurrence(text(URL_VALUE), &origin_at(FIRST_FILE)),
            )?,
        ]);

        let rows = rows(&config.tree(), SPAN)?;

        assert_eq!(texts(&rows, KEY_COLUMN)?, [BARE_KEY, REMOTE_ORIGIN_URL_ROW]);

        Ok(())
    }

    #[rstest]
    fn sorted_by_key_then_load_order() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(NAME_KEY, occurrence(text(ADA), &origin_at(FIRST_FILE)))?,
            keyed(NAME_KEY, occurrence(text(BOB), &origin_at(SECOND_FILE)))?,
            keyed(
                BARE_KEY,
                occurrence(text(TRUE_TEXT), &origin_at(FIRST_FILE)),
            )?,
        ]);

        let rows = rows(&config.tree(), SPAN)?;

        assert_eq!(texts(&rows, KEY_COLUMN)?, [BARE_KEY, NAME_KEY, NAME_KEY]);
        assert_eq!(texts(&rows, VALUE_COLUMN)?, [TRUE_TEXT, ADA, BOB]);

        Ok(())
    }

    #[rstest]
    fn list_values_are_separate_rows() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(
                HELPER_KEY,
                occurrence(text(FIRST_VALUE), &origin_at(FIRST_FILE)),
            )?,
            keyed(
                HELPER_KEY,
                occurrence(text(SECOND_VALUE), &origin_at(SECOND_FILE)),
            )?,
        ]);

        let rows = rows(&config.tree(), SPAN)?;

        assert_eq!(texts(&rows, VALUE_COLUMN)?, [FIRST_VALUE, SECOND_VALUE]);

        Ok(())
    }

    #[rstest]
    fn keeps_a_hidden_variable() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(HIDDEN_KEY, occurrence(text(ONE), &origin_at(FIRST_FILE)))?,
            keyed(
                HIDDEN_SUBSECTION_KEY,
                occurrence(text(TWO), &origin_at(SECOND_FILE)),
            )?,
        ]);

        let rows = rows(&config.tree(), SPAN)?;

        assert_eq!(
            texts(&rows, KEY_COLUMN)?,
            [HIDDEN_KEY, HIDDEN_SUBSECTION_KEY]
        );

        Ok(())
    }

    #[rstest]
    fn valueless_is_null_and_empty_is_empty_string() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(
                BARE_KEY,
                occurrence(git::RawValue::Valueless, &origin_at(FIRST_FILE)),
            )?,
            keyed(
                NAME_KEY,
                occurrence(text(EMPTY_TEXT), &origin_at(FIRST_FILE)),
            )?,
        ]);

        let rows = rows(&config.tree(), SPAN)?;

        let values: Vec<&Value> = rows
            .iter()
            .map(|row| record_column(row, VALUE_COLUMN))
            .collect::<Result<_, _>>()?;

        assert_eq!(
            values,
            [&Value::nothing(SPAN), &Value::string(EMPTY_TEXT, SPAN)]
        );

        Ok(())
    }

    #[rstest]
    #[case::known_list_key(HELPER_KEY)]
    #[case::unknown_key(UNKNOWN_KEY)]
    fn reset_marker_is_empty_string(#[case] key: &str) -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(key, occurrence(text(ADA), &origin_at(FIRST_FILE)))?,
            keyed(key, occurrence(text(EMPTY_TEXT), &origin_at(SECOND_FILE)))?,
        ]);

        let rows = rows(&config.tree(), SPAN)?;

        assert_eq!(texts(&rows, VALUE_COLUMN)?, [ADA, EMPTY_TEXT]);
        assert_eq!(
            rows.iter()
                .map(|row| row.as_record().ok().map(Record::len))
                .collect::<Vec<_>>(),
            [Some(4), Some(4)]
        );

        Ok(())
    }

    #[rstest]
    fn value_is_always_text() -> TestResult {
        let config = git::Config::from_occurrences([keyed(
            AUTO_KEY,
            occurrence(text(FIVE), &origin_at(FIRST_FILE)),
        )?]);

        let rows = rows(&config.tree(), SPAN)?;

        assert_eq!(texts(&rows, VALUE_COLUMN)?, [FIVE]);

        Ok(())
    }

    #[rstest]
    fn scope_and_file() -> TestResult {
        let cli = Arc::new(git::Origin {
            path: None,
            source: Source::Cli,
        });
        let config = git::Config::from_occurrences([
            keyed(NAME_KEY, occurrence(text(ADA), &cli))?,
            keyed(
                BARE_KEY,
                occurrence(text(TRUE_TEXT), &origin_at("/p/config")),
            )?,
        ]);

        let rows = rows(&config.tree(), SPAN)?;

        assert_eq!(texts(&rows, SCOPE_COLUMN)?, [LOCAL_SCOPE, COMMAND_SCOPE]);

        let files: Vec<&Value> = rows
            .iter()
            .map(|row| record_column(row, FILE_COLUMN))
            .collect::<Result<_, _>>()?;

        assert_eq!(
            files,
            [&Value::string("/p/config", SPAN), &Value::nothing(SPAN)]
        );

        Ok(())
    }

    #[rstest]
    fn non_utf8_value_is_an_error() -> TestResult {
        let config = git::Config::from_occurrences([keyed(
            NAME_KEY,
            occurrence(
                git::RawValue::Bytes(NON_UTF8_BYTES.to_vec()),
                &origin_at(FIRST_FILE),
            ),
        )?]);

        let result = rows(&config.tree(), SPAN);

        assert_matches!(result, Err(git::GixError::ConfigValueNotUtf8 { .. }));

        Ok(())
    }

    #[rstest]
    fn non_utf8_subsection_is_an_error() {
        let config = git::Config::from_occurrences([(
            git::ConfigKey::new(
                git::SectionName::from(REMOTE),
                Some(git::Subsection::from(NON_UTF8_BYTES)),
                git::VariableName::from(URL),
            ),
            occurrence(text(URL_VALUE), &origin_at(SECOND_FILE)),
        )]);

        let result = rows(&config.tree(), SPAN);

        assert_matches!(result, Err(git::GixError::ConfigSubsectionNotUtf8 { .. }));
    }
}

mod everything {
    use super::*;

    const FETCH_KEY: &str = "remote.o.fetch";
    const FIRST_REFSPEC: &str = "+refs/heads/a:refs/remotes/o/a";
    const SECOND_REFSPEC: &str = "+refs/heads/b:refs/remotes/o/b";
    const MIXED_CASE_BARE_KEY: &str = "Core.Bare";
    const MIXED_CASE_SUBSECTION_KEY: &str = "remote.Origin.url";
    const MIXED_CASE_SUBSECTION: &str = "Origin";
    const BARE: &str = "bare";
    const CORE: &str = "core";
    const NAME: &str = "name";
    const AUTO: &str = "auto";
    const GC: &str = "gc";
    const FIVE: &str = "5";
    const FETCH: &str = "fetch";
    const A: &str = "a";
    const X: &str = "x";
    const Y: &str = "y";
    const YES: &str = "yes";

    fn root(value: &Value) -> Result<&Record, Box<TestError>> {
        value
            .as_record()
            .map_err(|error| Box::new(TestError::Failure(error.to_string())))
    }

    #[rstest]
    fn nests_sections_and_subsections() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(BARE_KEY, occurrence(text(YES), &origin_at(FIRST_FILE)))?,
            keyed(NAME_KEY, occurrence(text(ADA), &origin_at(FIRST_FILE)))?,
            keyed(
                &format!("{REMOTE}.{ORIGIN}.{URL}"),
                occurrence(text(URL_VALUE), &origin_at(FIRST_FILE)),
            )?,
        ]);

        let value = everything(&config, SPAN)?;

        assert_eq!(
            value,
            Value::record(
                record! {
                    CORE => Value::record(record! { BARE => Value::test_bool(true) }, SPAN),
                    REMOTE => Value::record(
                        record! {
                            ORIGIN => Value::record(
                                record! { URL => Value::test_list(vec![Value::test_string(URL_VALUE)]) },
                                SPAN
                            ),
                        },
                        SPAN
                    ),
                    USER => Value::record(record! { NAME => Value::test_string(ADA) }, SPAN),
                },
                SPAN
            )
        );

        Ok(())
    }

    #[rstest]
    fn lowercase_names_subsection_case_kept() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(
                MIXED_CASE_BARE_KEY,
                occurrence(text(YES), &origin_at(FIRST_FILE)),
            )?,
            keyed(
                MIXED_CASE_SUBSECTION_KEY,
                occurrence(text(URL_VALUE), &origin_at(FIRST_FILE)),
            )?,
        ]);

        let value = everything(&config, SPAN)?;
        let sections = root(&value)?;
        let remote = sections
            .get(REMOTE)
            .ok_or_else(|| Box::new(TestError::Failure("no remote section".to_owned())))?;

        assert!(sections.contains(CORE));
        assert!(root(remote)?.contains(MIXED_CASE_SUBSECTION));

        Ok(())
    }

    #[rstest]
    fn known_list_key_is_a_list() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(
                FETCH_KEY,
                occurrence(text(FIRST_REFSPEC), &origin_at(FIRST_FILE)),
            )?,
            keyed(
                FETCH_KEY,
                occurrence(text(SECOND_REFSPEC), &origin_at(SECOND_FILE)),
            )?,
        ]);

        let value = everything(&config, SPAN)?;

        assert_eq!(
            value,
            Value::record(
                record! {
                    REMOTE => Value::record(
                        record! {
                            "o" => Value::record(
                                record! {
                                    FETCH => Value::test_list(vec![
                                        Value::test_string(FIRST_REFSPEC),
                                        Value::test_string(SECOND_REFSPEC),
                                    ]),
                                },
                                SPAN
                            ),
                        },
                        SPAN
                    ),
                },
                SPAN
            )
        );

        Ok(())
    }

    #[rstest]
    fn unknown_repeated_key_is_the_last_value() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(
                NAME_KEY,
                occurrence(text(FIRST_VALUE), &origin_at(FIRST_FILE)),
            )?,
            keyed(
                NAME_KEY,
                occurrence(text(SECOND_VALUE), &origin_at(SECOND_FILE)),
            )?,
        ]);

        let value = everything(&config, SPAN)?;

        assert_eq!(
            value,
            Value::record(
                record! {
                    USER => Value::record(record! { NAME => Value::test_string(SECOND_VALUE) }, SPAN),
                },
                SPAN
            )
        );

        Ok(())
    }

    #[rstest]
    fn known_int_is_int() -> TestResult {
        let config = git::Config::from_occurrences([keyed(
            AUTO_KEY,
            occurrence(text(FIVE), &origin_at(FIRST_FILE)),
        )?]);

        let value = everything(&config, SPAN)?;

        assert_eq!(
            value,
            Value::record(
                record! { GC => Value::record(record! { AUTO => Value::test_int(5) }, SPAN) },
                SPAN
            )
        );

        Ok(())
    }

    #[rstest]
    fn subsection_hides_variable_of_its_name() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(
                &format!("{A}.{X}"),
                occurrence(text(ONE), &origin_at(FIRST_FILE)),
            )?,
            keyed(
                &format!("{A}.{X}.{Y}"),
                occurrence(text(TWO), &origin_at(SECOND_FILE)),
            )?,
        ]);

        let value = everything(&config, SPAN)?;

        assert_eq!(
            value,
            Value::record(
                record! {
                    A => Value::record(
                        record! {
                            X => Value::record(record! { Y => Value::test_string(TWO) }, SPAN),
                        },
                        SPAN
                    ),
                },
                SPAN
            )
        );

        Ok(())
    }

    #[rstest]
    fn reset_key_is_left_out() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(HELPER_KEY, occurrence(text(""), &origin_at(FIRST_FILE)))?,
            keyed(
                &format!("{CREDENTIAL}.{NAME}"),
                occurrence(text(ADA), &origin_at(FIRST_FILE)),
            )?,
        ]);

        let value = everything(&config, SPAN)?;

        assert_eq!(
            value,
            Value::record(
                record! {
                    CREDENTIAL => Value::record(record! { NAME => Value::test_string(ADA) }, SPAN),
                },
                SPAN
            )
        );

        Ok(())
    }

    #[rstest]
    fn section_of_only_resets_is_absent() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(HELPER_KEY, occurrence(text(""), &origin_at(FIRST_FILE)))?,
            keyed(NAME_KEY, occurrence(text(ADA), &origin_at(FIRST_FILE)))?,
        ]);

        let value = everything(&config, SPAN)?;

        assert_eq!(
            value,
            Value::record(
                record! { USER => Value::record(record! { NAME => Value::test_string(ADA) }, SPAN) },
                SPAN
            )
        );

        Ok(())
    }

    #[rstest]
    fn non_utf8_subsection_is_an_error() {
        let config = git::Config::from_occurrences([(
            git::ConfigKey::new(
                git::SectionName::from(REMOTE),
                Some(git::Subsection::from(NON_UTF8_BYTES)),
                git::VariableName::from(URL),
            ),
            occurrence(text(URL_VALUE), &origin_at(SECOND_FILE)),
        )]);

        let result = everything(&config, SPAN);

        assert_matches!(result, Err(git::GixError::ConfigSubsectionNotUtf8 { .. }));
    }

    #[rstest]
    fn known_bool_that_does_not_convert_is_an_error() -> TestResult {
        let config = git::Config::from_occurrences([keyed(
            BARE_KEY,
            occurrence(text(BANANA_TEXT), &origin_at(FIRST_FILE)),
        )?]);

        let result = everything(&config, SPAN);

        assert_matches!(result, Err(git::GixError::ConfigValueConversion { .. }));

        Ok(())
    }
}

mod lookup_value {
    use super::*;

    const CORE: &str = "core";
    const BARE: &str = "bare";
    const FETCH_KEY: &str = "remote.o.fetch";
    const FETCH_VALUE: &str = "+refs/heads/a:refs/remotes/o/a";

    fn lookup(
        config: &git::Config,
        path: &git::ConfigPath,
    ) -> Result<Option<Value>, Box<TestError>> {
        let tree = config.tree();

        tree.get(path)
            .map(|node| lookup_value(node, SPAN))
            .transpose()
            .map(Option::flatten)
            .map_err(Into::into)
    }

    #[rstest]
    fn setting_is_its_untyped_value() -> TestResult {
        let config = git::Config::from_occurrences([keyed(
            BARE_KEY,
            occurrence(text(TRUE_TEXT), &origin_at(FIRST_FILE)),
        )?]);

        let value = lookup(&config, &git::ConfigPath::member(CORE, BARE)?)?;

        assert_eq!(value, Some(Value::test_bool(true)));

        Ok(())
    }

    #[rstest]
    fn known_list_setting_is_a_list() -> TestResult {
        let config = git::Config::from_occurrences([keyed(
            FETCH_KEY,
            occurrence(text(FETCH_VALUE), &origin_at(FIRST_FILE)),
        )?]);

        let value = lookup(&config, &git::ConfigPath::variable(REMOTE, "o", "fetch")?)?;

        assert_eq!(
            value,
            Some(Value::list(vec![Value::test_string(FETCH_VALUE)], SPAN))
        );

        Ok(())
    }

    #[rstest]
    fn reset_setting_is_an_error() -> TestResult {
        let origin = origin_at(FIRST_FILE);
        let config = git::Config::from_occurrences([
            keyed(HELPER_KEY, occurrence(text(HELPER), &origin))?,
            keyed(HELPER_KEY, occurrence(text(""), &origin))?,
        ]);

        let result = lookup(&config, &git::ConfigPath::member(CREDENTIAL, "helper")?);

        assert_matches!(
            result.map_err(|error| *error),
            Err(TestError::Gix(error))
                if matches!(error.as_ref(), git::GixError::ConfigKeyReset { .. })
        );

        Ok(())
    }

    #[rstest]
    fn all_reset_section_is_none() -> TestResult {
        let origin = origin_at(FIRST_FILE);
        let config = git::Config::from_occurrences([
            keyed(HELPER_KEY, occurrence(text(HELPER), &origin))?,
            keyed(HELPER_KEY, occurrence(text(""), &origin))?,
        ]);

        let value = lookup(&config, &git::ConfigPath::section(CREDENTIAL)?)?;

        assert_eq!(value, None);

        Ok(())
    }

    #[rstest]
    fn subsection_is_its_record() -> TestResult {
        let config = git::Config::from_occurrences([keyed(
            FETCH_KEY,
            occurrence(text(FETCH_VALUE), &origin_at(FIRST_FILE)),
        )?]);

        let value = lookup(&config, &git::ConfigPath::member(REMOTE, "o")?)?;

        assert_eq!(
            value,
            Some(Value::record(
                record! { "fetch" => Value::list(vec![Value::test_string(FETCH_VALUE)], SPAN) },
                SPAN
            ))
        );

        Ok(())
    }
}
