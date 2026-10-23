//! Tests of the configuration read policy: resolution, untyped and typed reads, and the section, subsection and whole-configuration records.

use super::*;

use crate::commands::config_known::SubsectionRule;

use nu_plugin_gix_testing_support::{FixtureError, TestRepo, test_repo};
use nu_protocol::record;
use rstest::{fixture, rstest};

use gix::config::Source;

use std::{assert_matches, iter};

#[derive(thiserror::Error, Debug)]
enum TestError {
    #[error(transparent)]
    Gix(#[from] git::GixError),

    #[error(transparent)]
    Fixture(#[from] FixtureError),

    #[error("Test failure: {0}")]
    Failure(String),
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
const OTHER: &str = "other";
const OTHER_VALUE: &str = "x";
const LIST_SECTION: &str = "gixtest";
const LIST_NAME: &str = "list";
const SECTION: &str = "gixtest";
const ZETA_MEMBER: &str = "zeta.member";
const ALPHA: &str = "alpha";
const MID_MEMBER: &str = "mid.member";
const SECTION_ABSENT: &str = "gixabsent";
const CREDENTIAL: &str = "credential";
const USER: &str = "user";
const ADA: &str = "Ada";
const REMOTE: &str = "remote";
const ORIGIN: &str = "origin";
const URL: &str = "url";
const URL_VALUE: &str = "x";
const ZETA: &str = "zeta";
const BETA: &str = "beta";
const SUB: &str = "sub";
const GAMMA: &str = "gamma";
const ONE: &str = "1";
const TWO: &str = "2";
const THREE: &str = "3";
const SHARED: &str = "shared";
const MEMBER: &str = "member";
const UPPER_SHARED: &str = "Shared";
const INCLUDE: &str = "include";
const PATH: &str = "path";
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
    key: &str,
    occurrence: git::Occurrence,
) -> Result<(git::ConfigKey, git::Occurrence), git::GixError> {
    Ok((git::ConfigKey::parse(key)?, occurrence))
}

fn untyped(key: &str, values: &Values<'_>) -> Result<Value, git::GixError> {
    untyped_value(&git::ConfigKey::parse(key)?, values, SPAN)
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

fn record_of(group: Group) -> Result<Record, Box<TestError>> {
    match group {
        Group::Record(record) => Ok(record),
        other => Err(Box::new(TestError::Failure(format!(
            "expected a record, found {other:?}"
        )))),
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

mod nest {
    use super::*;

    #[rstest]
    fn variable_then_subsection_clashes() -> TestResult {
        let mut nest = Nest::default();
        nest.insert_key(
            &git::ConfigKey::parse(&format!("{SECTION}.{SHARED}"))?,
            &[occurrence(text(ONE), &origin_at(FIRST_FILE))],
            SPAN,
        )?;

        let result = nest.insert_key(
            &git::ConfigKey::parse(&format!("{SECTION}.{SHARED}.{MEMBER}"))?,
            &[occurrence(text(TWO), &origin_at(SECOND_FILE))],
            SPAN,
        );

        assert_matches!(
            result,
            Err(git::GixError::ConfigNameClash { section, name, variable_origin, subsection_origin })
                if section == git::SectionName::from(SECTION)
                    && name == git::VariableName::from(SHARED)
                    && variable_origin.path == Some(PathBuf::from(FIRST_FILE))
                    && subsection_origin.path == Some(PathBuf::from(SECOND_FILE))
        );

        Ok(())
    }

    #[rstest]
    fn subsection_then_variable_clashes() -> TestResult {
        let mut nest = Nest::default();
        nest.insert_key(
            &git::ConfigKey::parse(&format!("{SECTION}.{SHARED}.{MEMBER}"))?,
            &[occurrence(text(TWO), &origin_at(FIRST_FILE))],
            SPAN,
        )?;

        let result = nest.insert_key(
            &git::ConfigKey::parse(&format!("{SECTION}.{SHARED}"))?,
            &[occurrence(text(ONE), &origin_at(SECOND_FILE))],
            SPAN,
        );

        assert_matches!(
            result,
            Err(git::GixError::ConfigNameClash { section, name, variable_origin, subsection_origin })
                if section == git::SectionName::from(SECTION)
                    && name == git::VariableName::from(SHARED)
                    && variable_origin.path == Some(PathBuf::from(SECOND_FILE))
                    && subsection_origin.path == Some(PathBuf::from(FIRST_FILE))
        );

        Ok(())
    }

    #[rstest]
    fn columns_are_alphabetical_whatever_the_insertion_order() -> TestResult {
        let mut nest = Nest::default();

        for tail in [ZETA_MEMBER, ALPHA, MID_MEMBER] {
            nest.insert_key(
                &git::ConfigKey::parse(&format!("{SECTION}.{tail}"))?,
                &[occurrence(text(ONE), &origin_at(FIRST_FILE))],
                SPAN,
            )?;
        }

        let record = nest.into_record(SPAN);

        assert_eq!(
            record.columns().map(String::as_str).collect::<Vec<_>>(),
            vec!["alpha", "mid", "zeta"]
        );

        Ok(())
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
            Err(git::GixError::ConfigValueConversion { origin, .. })
                if origin.path == Some(PathBuf::from(FIRST_FILE))
        );
    }

    #[rstest]
    fn text_that_is_not_utf8_is_an_error() {
        let name = occurrence(
            git::RawValue::Bytes(NON_UTF8_BYTES.to_owned()),
            &origin_at(FIRST_FILE),
        );

        let result = untyped(NAME_KEY, &Values::Single(&name));

        assert_matches!(result, Err(git::GixError::ConfigValueNotUtf8 { .. }));
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

mod read_typed {
    use super::*;

    #[rstest]
    fn valueless_bool_is_true(
        path_context: Result<(TestRepo, git::PathContext), Box<TestError>>,
    ) -> TestResult {
        let (_repo, paths) = path_context?;
        let config = git::Config::from_occurrences([keyed(
            BARE_KEY,
            occurrence(git::RawValue::Valueless, &origin_at(FIRST_FILE)),
        )?]);

        let value = read_typed(
            &config,
            &git::ConfigKey::parse(BARE_KEY)?,
            git::ConfigType::Scalar(git::ScalarType::Bool),
            &paths,
            SPAN,
        )?;

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

        let result = read_typed(&config, &git::ConfigKey::parse(key)?, ty, &paths, SPAN);

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

        let result = read_typed(
            &config,
            &git::ConfigKey::parse(HELPER_KEY)?,
            git::ConfigType::Scalar(git::ScalarType::String),
            &paths,
            SPAN,
        );

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

        let value = read_typed(
            &config,
            &git::ConfigKey::parse(HELPER_KEY)?,
            git::ConfigType::Scalar(git::ScalarType::String),
            &paths,
            SPAN,
        )?;

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

        let value = read_typed(
            &config,
            &git::ConfigKey::parse(INCLUDE_KEY)?,
            git::ConfigType::Path,
            &paths,
            SPAN,
        )?;

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

        let value = read_typed(
            &config,
            &git::ConfigKey::parse(HELPER_KEY)?,
            git::ConfigType::Path,
            &paths,
            SPAN,
        )?;

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

        let result = read_typed(
            &config,
            &git::ConfigKey::parse(INCLUDE_KEY)?,
            git::ConfigType::Path,
            &paths,
            SPAN,
        );

        assert_matches!(result, Err(git::GixError::ConfigPathNotUtf8 { .. }));

        Ok(())
    }

    #[rstest]
    fn unset_key_is_null(
        path_context: Result<(TestRepo, git::PathContext), Box<TestError>>,
    ) -> TestResult {
        let (_repo, paths) = path_context?;
        let config = git::Config::default();

        let value = read_typed(
            &config,
            &git::ConfigKey::parse(BARE_KEY)?,
            git::ConfigType::Scalar(git::ScalarType::Bool),
            &paths,
            SPAN,
        )?;

        assert_eq!(value, Value::nothing(SPAN));

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

        let value = read_typed(
            &config,
            &git::ConfigKey::parse(BARE_KEY)?,
            git::ConfigType::Scalar(git::ScalarType::Bool),
            &paths,
            SPAN,
        )?;

        assert_eq!(value, Value::test_bool(true));

        Ok(())
    }
}

mod read_variable {
    use super::*;

    #[rstest]
    fn unset_key_is_none() -> TestResult {
        let config = git::Config::default();

        let value = read_variable(&config, &git::ConfigKey::parse(NAME_KEY)?, SPAN)?;

        assert_eq!(value, None);

        Ok(())
    }

    #[rstest]
    fn reset_is_an_error() -> TestResult {
        let config = git::Config::from_occurrences([keyed(
            HELPER_KEY,
            occurrence(text(""), &origin_at(FIRST_FILE)),
        )?]);

        let result = read_variable(&config, &git::ConfigKey::parse(HELPER_KEY)?, SPAN);

        assert_matches!(result, Err(git::GixError::ConfigKeyReset { .. }));

        Ok(())
    }
}

mod read_section {
    use super::*;

    #[rstest]
    fn nested_shape_has_alphabetical_columns() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(
                &format!("{SECTION}.{ZETA}"),
                occurrence(text(ONE), &origin_at(FIRST_FILE)),
            )?,
            keyed(
                &format!("{SECTION}.{BETA}"),
                occurrence(text(TWO), &origin_at(FIRST_FILE)),
            )?,
            keyed(
                &format!("{SECTION}.{SUB}.{GAMMA}"),
                occurrence(text(THREE), &origin_at(FIRST_FILE)),
            )?,
        ]);

        let group = read_section(&config, &git::SectionName::from(SECTION), SPAN)?;
        let record = record_of(group)?;

        assert_eq!(
            record.columns().map(String::as_str).collect::<Vec<_>>(),
            vec![BETA, SUB, ZETA]
        );
        assert_eq!(
            record.get(SUB),
            Some(&Value::record(
                record! { GAMMA => Value::test_string(THREE) },
                SPAN
            ))
        );

        Ok(())
    }

    #[rstest]
    fn reset_key_is_left_out() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(HELPER_KEY, occurrence(text(""), &origin_at(FIRST_FILE)))?,
            keyed(
                &format!("{CREDENTIAL}.{OTHER}"),
                occurrence(text(OTHER_VALUE), &origin_at(FIRST_FILE)),
            )?,
        ]);

        let group = read_section(&config, &git::SectionName::from(CREDENTIAL), SPAN)?;

        assert_eq!(
            record_of(group)?,
            record! { OTHER => Value::test_string(OTHER_VALUE) }
        );

        Ok(())
    }

    #[rstest]
    fn section_of_only_resets_is_all_reset() -> TestResult {
        let config = git::Config::from_occurrences([keyed(
            HELPER_KEY,
            occurrence(text(""), &origin_at(FIRST_FILE)),
        )?]);

        let group = read_section(&config, &git::SectionName::from(CREDENTIAL), SPAN)?;

        assert_matches!(group, Group::AllReset);

        Ok(())
    }

    #[rstest]
    fn unknown_section_is_missing() -> TestResult {
        let config = git::Config::default();

        let group = read_section(&config, &git::SectionName::from(SECTION_ABSENT), SPAN)?;

        assert_matches!(group, Group::Missing);

        Ok(())
    }

    #[rstest]
    fn variable_and_subsection_of_one_name_clash() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(
                &format!("{SECTION}.{SHARED}"),
                occurrence(text(ONE), &origin_at(FIRST_FILE)),
            )?,
            keyed(
                &format!("{SECTION}.{SHARED}.{MEMBER}"),
                occurrence(text(TWO), &origin_at(SECOND_FILE)),
            )?,
        ]);

        let result = read_section(&config, &git::SectionName::from(SECTION), SPAN);

        assert_matches!(
            result,
            Err(git::GixError::ConfigNameClash { section, name, variable_origin, subsection_origin })
                if section == git::SectionName::from(SECTION)
                    && name == git::VariableName::from(SHARED)
                    && variable_origin.path == Some(PathBuf::from(FIRST_FILE))
                    && subsection_origin.path == Some(PathBuf::from(SECOND_FILE))
        );

        Ok(())
    }

    #[rstest]
    fn fully_reset_clashing_variable_is_no_error() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(HELPER_KEY, occurrence(text(""), &origin_at(FIRST_FILE)))?,
            keyed(
                &format!("{CREDENTIAL}.{HELPER}.{MEMBER}"),
                occurrence(text(TWO), &origin_at(SECOND_FILE)),
            )?,
        ]);

        let group = read_section(&config, &git::SectionName::from(CREDENTIAL), SPAN)?;

        assert_eq!(
            record_of(group)?,
            record! {
                HELPER => Value::record(record! { MEMBER => Value::test_string(TWO) }, SPAN),
            }
        );

        Ok(())
    }

    #[rstest]
    fn subsection_differing_in_case_does_not_clash() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(
                &format!("{SECTION}.{SHARED}"),
                occurrence(text(ONE), &origin_at(FIRST_FILE)),
            )?,
            keyed(
                &format!("{SECTION}.{UPPER_SHARED}.{MEMBER}"),
                occurrence(text(TWO), &origin_at(SECOND_FILE)),
            )?,
        ]);

        let group = read_section(&config, &git::SectionName::from(SECTION), SPAN)?;

        assert_eq!(
            record_of(group)?,
            record! {
                UPPER_SHARED => Value::record(record! { MEMBER => Value::test_string(TWO) }, SPAN),
                SHARED => Value::test_string(ONE),
            }
        );

        Ok(())
    }

    #[rstest]
    fn non_utf8_subsection_of_the_section_is_an_error() {
        let config = git::Config::from_occurrences([(
            git::ConfigKey::new(
                git::SectionName::from(REMOTE),
                Some(git::Subsection::from(NON_UTF8_BYTES)),
                git::VariableName::from(URL),
            ),
            occurrence(text(URL_VALUE), &origin_at(FIRST_FILE)),
        )]);

        let result = read_section(&config, &git::SectionName::from(REMOTE), SPAN);

        assert_matches!(result, Err(git::GixError::ConfigSubsectionNotUtf8 { .. }));
    }

    #[rstest]
    fn non_utf8_subsection_does_not_break_another_section() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(
                &format!("{USER}.{HELPER}"),
                occurrence(text(ADA), &origin_at(FIRST_FILE)),
            )?,
            (
                git::ConfigKey::new(
                    git::SectionName::from(REMOTE),
                    Some(git::Subsection::from(NON_UTF8_BYTES)),
                    git::VariableName::from(URL),
                ),
                occurrence(text(URL_VALUE), &origin_at(SECOND_FILE)),
            ),
        ]);

        let group = read_section(&config, &git::SectionName::from(USER), SPAN)?;

        assert_eq!(
            record_of(group)?,
            record! { HELPER => Value::test_string(ADA) }
        );

        Ok(())
    }

    #[rstest]
    fn clash_names_the_first_emitted_variable_of_the_subsection() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(
                &format!("{REMOTE}.{ORIGIN}.{URL}"),
                occurrence(text(""), &origin_at(FIRST_FILE)),
            )?,
            keyed(
                &format!("{REMOTE}.{ORIGIN}.{BETA}"),
                occurrence(text(TWO), &origin_at(SECOND_FILE)),
            )?,
            keyed(
                &format!("{REMOTE}.{ORIGIN}"),
                occurrence(text(ONE), &origin_at(FIRST_FILE)),
            )?,
        ]);

        let result = read_section(&config, &git::SectionName::from(REMOTE), SPAN);

        assert_matches!(
            result,
            Err(git::GixError::ConfigNameClash { subsection_origin, .. })
                if subsection_origin.path == Some(PathBuf::from(SECOND_FILE))
        );

        Ok(())
    }

    #[rstest]
    fn clash_origin_of_a_list_variable_is_its_last_element() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(
                INCLUDE_KEY,
                occurrence(text(FIRST_VALUE), &origin_at(FIRST_FILE)),
            )?,
            keyed(
                INCLUDE_KEY,
                occurrence(text(SECOND_VALUE), &origin_at(SECOND_FILE)),
            )?,
            keyed(
                &format!("{INCLUDE}.{PATH}.{MEMBER}"),
                occurrence(text(TWO), &origin_at(FIRST_FILE)),
            )?,
        ]);

        let result = read_section(&config, &git::SectionName::from(INCLUDE), SPAN);

        assert_matches!(
            result,
            Err(git::GixError::ConfigNameClash { variable_origin, .. })
                if variable_origin.path == Some(PathBuf::from(SECOND_FILE))
        );

        Ok(())
    }
}

mod read_subsection {
    use super::*;

    #[rstest]
    fn reads_members_of_a_subsection_that_clashes_with_a_variable() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(
                &format!("{SECTION}.{SHARED}"),
                occurrence(text(ONE), &origin_at(FIRST_FILE)),
            )?,
            keyed(
                &format!("{SECTION}.{SHARED}.{MEMBER}"),
                occurrence(text(TWO), &origin_at(SECOND_FILE)),
            )?,
        ]);

        let group = read_subsection(
            &config,
            &git::SectionName::from(SECTION),
            &git::Subsection::from(SHARED),
            SPAN,
        )?;

        assert_eq!(
            record_of(group)?,
            record! { MEMBER => Value::test_string(TWO) }
        );

        Ok(())
    }

    #[rstest]
    fn unset_subsection_is_missing() -> TestResult {
        let config = git::Config::default();

        let group = read_subsection(
            &config,
            &git::SectionName::from(REMOTE),
            &git::Subsection::from(ORIGIN),
            SPAN,
        )?;

        assert_matches!(group, Group::Missing);

        Ok(())
    }
}

mod read_all {
    use super::*;

    #[rstest]
    fn nests_sections_and_subsections() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(
                &format!("{USER}.{HELPER}"),
                occurrence(text(ADA), &origin_at(FIRST_FILE)),
            )?,
            keyed(
                &format!("{REMOTE}.{ORIGIN}.{URL}"),
                occurrence(text(URL_VALUE), &origin_at(FIRST_FILE)),
            )?,
        ]);

        let value = read_all(&config, SPAN)?;

        assert_eq!(
            value,
            Value::record(
                record! {
                    REMOTE => Value::record(
                        record! {
                            ORIGIN => Value::record(
                                record! { URL => Value::test_list(vec![Value::test_string(URL_VALUE)]) },
                                SPAN
                            ),
                        },
                        SPAN
                    ),
                    USER => Value::record(
                        record! { HELPER => Value::test_string(ADA) },
                        SPAN
                    ),
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
            keyed(
                &format!("{USER}.{HELPER}"),
                occurrence(text(ADA), &origin_at(FIRST_FILE)),
            )?,
        ]);

        let value = read_all(&config, SPAN)?;

        assert_eq!(
            value,
            Value::record(
                record! { USER => Value::record(record! { HELPER => Value::test_string(ADA) }, SPAN) },
                SPAN
            )
        );

        Ok(())
    }

    #[rstest]
    fn variable_and_subsection_of_one_name_clash() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(
                &format!("{SECTION}.{SHARED}"),
                occurrence(text(ONE), &origin_at(FIRST_FILE)),
            )?,
            keyed(
                &format!("{SECTION}.{SHARED}.{MEMBER}"),
                occurrence(text(TWO), &origin_at(SECOND_FILE)),
            )?,
        ]);

        let result = read_all(&config, SPAN);

        assert_matches!(result, Err(git::GixError::ConfigNameClash { .. }));

        Ok(())
    }

    #[rstest]
    fn non_utf8_subsection_is_an_error() -> TestResult {
        let config = git::Config::from_occurrences([
            keyed(
                &format!("{USER}.{HELPER}"),
                occurrence(text(ADA), &origin_at(FIRST_FILE)),
            )?,
            (
                git::ConfigKey::new(
                    git::SectionName::from(REMOTE),
                    Some(git::Subsection::from(NON_UTF8_BYTES)),
                    git::VariableName::from(URL),
                ),
                occurrence(text(URL_VALUE), &origin_at(SECOND_FILE)),
            ),
        ]);

        let result = read_all(&config, SPAN);

        assert_matches!(result, Err(git::GixError::ConfigSubsectionNotUtf8 { .. }));

        Ok(())
    }
}
