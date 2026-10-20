//! Reading single values from the layered git configuration.

use crate::error::GixError;
use crate::repository::OpenRepository;

use gix::config::AsKey as _;

use std::fmt;

const BOOL_NAME: &str = "bool";
const INT_NAME: &str = "int";
const PATH_NAME: &str = "path";
const STRING_NAME: &str = "string";
const TRUE_WORDS: [&str; 3] = ["true", "yes", "on"];
const FALSE_WORDS: [&str; 3] = ["false", "no", "off"];

/// The type a configuration value is read as by [`config_get_typed`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigType {
    /// A boolean, converted as git does (`yes`/`on`/`true`, `no`/`off`/`false`, integers, valueless keys).
    Bool,
    /// An integer, with git's `k`, `m` and `g` suffixes (powers of 1024) and `0x`/`0` prefixes.
    Int,
    /// A path, with `~` and `%(prefix)/` expanded, read only from trusted configuration files.
    Path,
    /// The raw string value.
    String,
}

/// A configuration value converted to the [`ConfigType`] it was read as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigValue {
    /// A boolean value.
    Bool(bool),
    /// An integer value.
    Int(i64),
    /// A path with its expansions applied.
    Path(String),
    /// A raw string value.
    String(String),
}

impl fmt::Display for ConfigType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Bool => BOOL_NAME,
            Self::Int => INT_NAME,
            Self::Path => PATH_NAME,
            Self::String => STRING_NAME,
        };

        f.write_str(name)
    }
}

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

/// Reads `key` as `ty`, converting the value the way git does for that type.
///
/// Same layers and last-value-wins rule as [`config_get`]. `Ok(None)` when no layer sets the key,
/// which includes a key that [`validate_config_key`] rejects.
///
/// # Errors
///
/// Returns [`GixError::ConfigValueConversion`] when the value cannot be read as `ty`,
/// [`GixError::ConfigValueNotUtf8`] when a string value is not valid UTF-8, and
/// [`GixError::ConfigPathNotUtf8`] when an expanded path is not valid UTF-8.
pub fn config_get_typed(
    opened: &OpenRepository,
    key: &str,
    ty: ConfigType,
) -> Result<Option<ConfigValue>, GixError> {
    let repo = opened.repo();
    let snapshot = repo.config_snapshot();

    let convert = |source| GixError::ConfigValueConversion {
        key: key.to_owned(),
        kind: ty,
        source,
    };

    match ty {
        ConfigType::Bool => Ok(snapshot
            .try_boolean(key)
            .map_err(convert)?
            .map(ConfigValue::Bool)),
        ConfigType::Int => Ok(snapshot
            .try_integer(key)
            .map_err(convert)?
            .map(ConfigValue::Int)),
        ConfigType::Path => snapshot
            .trusted_path(key)
            .map_err(convert)?
            .map(|path| {
                path.into_os_string().into_string().map_or_else(
                    |_| {
                        Err(GixError::ConfigPathNotUtf8 {
                            key: key.to_owned(),
                        })
                    },
                    |text| Ok(ConfigValue::Path(text)),
                )
            })
            .transpose(),
        ConfigType::String => Ok(config_get(opened, key)?.map(ConfigValue::String)),
    }
}

/// Reads `key` like [`config_get_typed`], guessing the type from the value instead of taking one.
///
/// A valueless key is `true`. `true`/`yes`/`on` and `false`/`no`/`off` (any case) are booleans, and a
/// value that git reads as an integer (decimal, `0x`, octal, or with a `k`/`m`/`g` suffix) is an
/// integer. Anything else, including the empty string, stays a string. Never fails on a value that
/// does not convert.
///
/// # Errors
///
/// Returns [`GixError::ConfigValueNotUtf8`] when a string value is not valid UTF-8.
pub fn config_get_auto(
    opened: &OpenRepository,
    key: &str,
) -> Result<Option<ConfigValue>, GixError> {
    let Some(raw) = config_get(opened, key)? else {
        return Ok(opened
            .repo()
            .config_snapshot()
            .boolean(key)
            .map(ConfigValue::Bool));
    };

    if TRUE_WORDS
        .iter()
        .any(|word| word.eq_ignore_ascii_case(&raw))
    {
        return Ok(Some(ConfigValue::Bool(true)));
    }
    if FALSE_WORDS
        .iter()
        .any(|word| word.eq_ignore_ascii_case(&raw))
    {
        return Ok(Some(ConfigValue::Bool(false)));
    }

    Ok(Some(
        opened
            .repo()
            .config_snapshot()
            .integer(key)
            .map_or_else(|| ConfigValue::String(raw), ConfigValue::Int),
    ))
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
    const TYPED_CONFIG: &[u8] = b"[gixtest]\n\tflag = yes\n\tno = no\n\toff = off\n\tzero = 0\n\timplicit\n\tempty =\n\tnum = 2k\n\thex = 0x10\n\tdec = 42\n\tbad = maybe\n\thome = ~/x\n\tword = hi\n";
    const AUTO_CONFIG: &[u8] = b"[gixtest]\n\tyes = yes\n\tupper = TRUE\n\ton = On\n\tno = No\n\toff = off\n\tfalse = false\n\tone = 1\n\tzero = 0\n\thex = 0x10\n\tkilo = 2k\n\tempty =\n\timplicit\n\tword = input\n";

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

    mod config_get_typed {
        use super::*;

        mod bool_values {
            use super::*;

            #[rstest]
            #[case::yes("gixtest.flag", true)]
            #[case::valueless("gixtest.implicit", true)]
            #[case::no("gixtest.no", false)]
            #[case::off("gixtest.off", false)]
            #[case::zero("gixtest.zero", false)]
            #[case::empty("gixtest.empty", false)]
            fn reads_a_boolean(
                test_repo: Result<TestRepo, FixtureError>,
                #[case] key: &str,
                #[case] expected: bool,
            ) -> Result<(), TestError> {
                let repo = test_repo?;

                append_config(repo.path(), TYPED_CONFIG)?;

                let opened = open_path(repo.path())?;

                assert_eq!(
                    config_get_typed(&opened, key, ConfigType::Bool)?,
                    Some(ConfigValue::Bool(expected))
                );

                Ok(())
            }
        }

        mod int_values {
            use super::*;

            #[rstest]
            #[case::kilobytes("gixtest.num", 2048)]
            #[case::hexadecimal("gixtest.hex", 16)]
            #[case::decimal("gixtest.dec", 42)]
            fn reads_an_integer(
                test_repo: Result<TestRepo, FixtureError>,
                #[case] key: &str,
                #[case] expected: i64,
            ) -> Result<(), TestError> {
                let repo = test_repo?;

                append_config(repo.path(), TYPED_CONFIG)?;

                let opened = open_path(repo.path())?;

                assert_eq!(
                    config_get_typed(&opened, key, ConfigType::Int)?,
                    Some(ConfigValue::Int(expected))
                );

                Ok(())
            }
        }

        mod path_values {
            use super::*;

            #[rstest]
            fn expands_a_home_relative_path(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;

                append_config(repo.path(), TYPED_CONFIG)?;

                // Discovery opens with the default permissions that production uses, which let gix read
                // HOME; the isolated fixture open cannot expand `~`.
                let (opened, _) = OpenRepository::discover(repo.path())?;

                assert_matches!(
                    config_get_typed(&opened, "gixtest.home", ConfigType::Path)?,
                    Some(ConfigValue::Path(path)) if !path.starts_with('~') && path.ends_with("/x")
                );

                Ok(())
            }
        }

        mod string_values {
            use super::*;

            #[rstest]
            fn reads_the_raw_string(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;

                append_config(repo.path(), TYPED_CONFIG)?;

                let opened = open_path(repo.path())?;

                assert_eq!(
                    config_get_typed(&opened, "gixtest.word", ConfigType::String)?,
                    Some(ConfigValue::String("hi".to_owned()))
                );

                Ok(())
            }
        }

        mod missing {
            use super::*;

            #[rstest]
            #[case::bool(ConfigType::Bool)]
            #[case::int(ConfigType::Int)]
            #[case::path(ConfigType::Path)]
            #[case::string(ConfigType::String)]
            fn returns_none_for_an_unset_key(
                test_repo: Result<TestRepo, FixtureError>,
                #[case] ty: ConfigType,
            ) -> Result<(), TestError> {
                let repo = test_repo?;
                let opened = open_path(repo.path())?;

                assert_eq!(config_get_typed(&opened, "gixtest.absent", ty)?, None);

                Ok(())
            }
        }

        mod errors {
            use super::*;

            #[rstest]
            #[case::bool(ConfigType::Bool)]
            #[case::int(ConfigType::Int)]
            #[case::path(ConfigType::Path)]
            #[case::string(ConfigType::String)]
            fn reads_a_malformed_key_as_unset(
                test_repo: Result<TestRepo, FixtureError>,
                #[case] ty: ConfigType,
            ) -> Result<(), TestError> {
                let repo = test_repo?;

                append_config(repo.path(), TYPED_CONFIG)?;

                let opened = open_path(repo.path())?;

                assert_eq!(config_get_typed(&opened, "nodot", ty)?, None);

                Ok(())
            }

            #[rstest]
            fn rejects_a_value_that_is_not_a_boolean(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;

                append_config(repo.path(), TYPED_CONFIG)?;

                let opened = open_path(repo.path())?;

                let err = config_get_typed(&opened, "gixtest.bad", ConfigType::Bool)
                    .err()
                    .ok_or_else(|| TestError::Failure(NO_ERROR_FAILURE.to_owned()))?;

                assert_matches!(
                    &err,
                    GixError::ConfigValueConversion {
                        kind: ConfigType::Bool,
                        ..
                    },
                    "got {err:?}"
                );

                Ok(())
            }

            #[rstest]
            fn rejects_a_value_that_is_not_an_integer(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;

                append_config(repo.path(), TYPED_CONFIG)?;

                let opened = open_path(repo.path())?;

                let err = config_get_typed(&opened, "gixtest.word", ConfigType::Int)
                    .err()
                    .ok_or_else(|| TestError::Failure(NO_ERROR_FAILURE.to_owned()))?;

                assert_matches!(
                    &err,
                    GixError::ConfigValueConversion {
                        kind: ConfigType::Int,
                        ..
                    },
                    "got {err:?}"
                );

                Ok(())
            }

            #[rstest]
            fn rejects_a_non_utf8_string(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;

                append_config(repo.path(), NON_UTF8_CONFIG)?;

                let opened = open_path(repo.path())?;

                let err = config_get_typed(&opened, "gixtest.bad", ConfigType::String)
                    .err()
                    .ok_or_else(|| TestError::Failure(NO_ERROR_FAILURE.to_owned()))?;

                assert_matches!(&err, GixError::ConfigValueNotUtf8 { .. }, "got {err:?}");

                Ok(())
            }
        }
    }

    mod config_get_auto {
        use super::*;

        mod values {
            use super::*;

            #[rstest]
            #[case::valueless("gixtest.implicit", ConfigValue::Bool(true))]
            #[case::yes("gixtest.yes", ConfigValue::Bool(true))]
            #[case::upper_case_true("gixtest.upper", ConfigValue::Bool(true))]
            #[case::on("gixtest.on", ConfigValue::Bool(true))]
            #[case::no("gixtest.no", ConfigValue::Bool(false))]
            #[case::off("gixtest.off", ConfigValue::Bool(false))]
            #[case::false_word("gixtest.false", ConfigValue::Bool(false))]
            #[case::one("gixtest.one", ConfigValue::Int(1))]
            #[case::zero("gixtest.zero", ConfigValue::Int(0))]
            #[case::hexadecimal("gixtest.hex", ConfigValue::Int(16))]
            #[case::kilobytes("gixtest.kilo", ConfigValue::Int(2048))]
            #[case::empty("gixtest.empty", ConfigValue::String(String::new()))]
            #[case::word("gixtest.word", ConfigValue::String("input".to_owned()))]
            fn converts_like_git(
                test_repo: Result<TestRepo, FixtureError>,
                #[case] key: &str,
                #[case] expected: ConfigValue,
            ) -> Result<(), TestError> {
                let repo = test_repo?;

                append_config(repo.path(), AUTO_CONFIG)?;

                let opened = open_path(repo.path())?;

                assert_eq!(config_get_auto(&opened, key)?, Some(expected));

                Ok(())
            }
        }

        mod missing {
            use super::*;

            #[rstest]
            #[case::unset("gixtest.absent")]
            #[case::no_dot("nodot")]
            #[case::empty("")]
            fn returns_none_without_a_value(
                test_repo: Result<TestRepo, FixtureError>,
                #[case] key: &str,
            ) -> Result<(), TestError> {
                let repo = test_repo?;

                append_config(repo.path(), AUTO_CONFIG)?;

                let opened = open_path(repo.path())?;

                assert_eq!(config_get_auto(&opened, key)?, None);

                Ok(())
            }
        }
    }
}
