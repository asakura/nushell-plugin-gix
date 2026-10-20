//! Reading single values from the layered git configuration.

use crate::error::GixError;
use crate::repository::OpenRepository;

use gix::config::AsKey as _;

/// Checks that `key` is of the form `section.name` or `section.subsection.name`, without touching a
/// repository, so callers can reject a bad key before acquiring one.
///
/// # Errors
///
/// Returns [`GixError::InvalidConfigKey`] when `key` does not parse as a configuration key.
pub fn validate_config_key(key: &str) -> Result<(), GixError> {
    if key.try_as_key().is_none() {
        return Err(GixError::InvalidConfigKey {
            key: key.to_owned(),
        });
    }

    Ok(())
}

/// Reads `key` (`section.name` or `section.subsection.name`) from the configuration layers.
///
/// Layers are those resolved when `opened` was opened, and the most recent value wins. `Ok(None)` when
/// no layer sets the key, which includes a key that [`validate_config_key`] rejects.
///
/// # Errors
///
/// Returns [`GixError::ConfigValueNotUtf8`] when the value is not valid UTF-8.
pub fn config_get(opened: &OpenRepository, key: &str) -> Result<Option<String>, GixError> {
    let repo = opened.repo();
    let snapshot = repo.config_snapshot();

    snapshot
        .string(key)
        .map(|value| {
            String::from_utf8(value.into()).map_err(|source| GixError::ConfigValueNotUtf8 {
                key: key.to_owned(),
                source,
            })
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fixtures::open_path;
    use nu_plugin_gix_testing_support::{
        FixtureError, FixtureRepo as _, TestRepo, append_config, test_repo,
    };
    use rstest::rstest;

    use std::{assert_matches, io};

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Gix(#[from] GixError),

        #[error(transparent)]
        Fixture(#[from] FixtureError),

        #[error(transparent)]
        Io(#[from] io::Error),

        #[error(transparent)]
        OpenFixture(#[from] gix::Error),

        #[error("Test failure: {0}")]
        Failure(String),
    }

    const NO_ERROR_FAILURE: &str = "expected an error";
    const LOCAL_CONFIG: &[u8] = b"[gixtest]\n\tanswer = 42\n";
    const SUBSECTION_CONFIG: &[u8] = b"[gixtest \"sub\"]\n\tname = x\n";
    const REPEATED_CONFIG: &[u8] = b"[gixtest]\n\tanswer = 1\n\tanswer = 2\n";
    const NON_UTF8_CONFIG: &[u8] = b"[gixtest]\n\tbad = \xff\xfe\n";

    mod validate_config_key {
        use super::*;

        #[rstest]
        #[case::section_and_name("core.editor")]
        #[case::with_subsection("remote.origin.url")]
        fn accepts_a_dotted_key(#[case] key: &str) {
            assert_matches!(validate_config_key(key), Ok(()));
        }

        #[rstest]
        #[case::no_dot("nodot")]
        #[case::empty("")]
        fn rejects_a_malformed_key(#[case] key: &str) {
            assert_matches!(
                validate_config_key(key),
                Err(GixError::InvalidConfigKey { key: reported }) if reported == key
            );
        }
    }

    mod config_get {
        use super::*;

        mod found {
            use super::*;

            #[rstest]
            fn reads_a_local_value(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;
                append_config(repo.path(), LOCAL_CONFIG)?;
                let opened = open_path(repo.path())?;

                assert_eq!(
                    config_get(&opened, "gixtest.answer")?,
                    Some("42".to_owned())
                );

                Ok(())
            }

            #[rstest]
            fn reads_a_subsection_key(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;
                append_config(repo.path(), SUBSECTION_CONFIG)?;
                let opened = open_path(repo.path())?;

                assert_eq!(
                    config_get(&opened, "gixtest.sub.name")?,
                    Some("x".to_owned())
                );

                Ok(())
            }

            #[rstest]
            fn matches_section_and_name_case_insensitively(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;
                append_config(repo.path(), LOCAL_CONFIG)?;
                let opened = open_path(repo.path())?;

                assert_eq!(
                    config_get(&opened, "GixTest.ANSWER")?,
                    Some("42".to_owned())
                );

                Ok(())
            }

            #[rstest]
            fn takes_the_last_value_when_a_key_repeats(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;
                append_config(repo.path(), REPEATED_CONFIG)?;
                let opened = open_path(repo.path())?;

                assert_eq!(config_get(&opened, "gixtest.answer")?, Some("2".to_owned()));

                Ok(())
            }
        }

        mod missing {
            use super::*;

            #[rstest]
            fn returns_none_for_an_unset_key(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;
                let opened = open_path(repo.path())?;

                assert_eq!(config_get(&opened, "gixtest.absent")?, None);

                Ok(())
            }
        }

        mod errors {
            use super::*;

            #[rstest]
            #[case::no_dot("nodot")]
            #[case::empty("")]
            fn reads_a_malformed_key_as_unset(
                test_repo: Result<TestRepo, FixtureError>,
                #[case] key: &str,
            ) -> Result<(), TestError> {
                let repo = test_repo?;
                let opened = open_path(repo.path())?;

                assert_eq!(config_get(&opened, key)?, None);

                Ok(())
            }

            #[rstest]
            #[case::trailing_dot("a.")]
            #[case::leading_dot(".b")]
            #[case::empty_subsection("a..b")]
            #[case::space_in_name("a.b c")]
            #[case::newline_in_section("a\nb.c")]
            fn reads_an_odd_key_without_failing_the_process(
                test_repo: Result<TestRepo, FixtureError>,
                #[case] key: &str,
            ) -> Result<(), TestError> {
                let repo = test_repo?;
                let opened = open_path(repo.path())?;

                assert_matches!(config_get(&opened, key), Ok(None) | Err(_));

                Ok(())
            }

            #[rstest]
            fn rejects_a_non_utf8_value(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                const KEY: &str = "gixtest.bad";

                let repo = test_repo?;
                append_config(repo.path(), NON_UTF8_CONFIG)?;
                let opened = open_path(repo.path())?;

                let err = config_get(&opened, KEY)
                    .err()
                    .ok_or_else(|| TestError::Failure(NO_ERROR_FAILURE.to_owned()))?;

                assert_matches!(&err, GixError::ConfigValueNotUtf8 { .. }, "got {err:?}");

                Ok(())
            }
        }
    }
}
