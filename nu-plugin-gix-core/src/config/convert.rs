//! Conversion of one configuration occurrence to the type it is read as; a valueless occurrence has no value except as a bool.

use super::{
    Origin,
    key::ConfigKey,
    store::{Occurrence, RawValue},
};
use crate::error::GixError;
use crate::repository::OpenRepository;

use gix::{
    bstr::BStr,
    config::{
        Boolean, Integer, Path as ConfigPath,
        path::interpolate::{Context, home_for_user},
    },
    path::env::home_dir,
};

use std::{fmt, path::PathBuf, str::from_utf8, sync::Arc};

const BOOL_NAME: &str = "bool";
const INT_NAME: &str = "int";
const PATH_NAME: &str = "path";
/// gix's own marker is private and its failure is an untyped message, so the check is ours; `convert_path::prefix_relative` and `names_the_cause_for_a_prefix_path` fail if the two disagree.
const PREFIX_MARKER: &[u8] = b"%(prefix)/";
const STRING_NAME: &str = "string";

/// The type an occurrence is read as by [`convert`] or [`convert_path`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigType {
    /// A type [`convert`] reads from the raw value; see [`ScalarType`] for the variants.
    Scalar(ScalarType),
    /// A path, with `~` and `%(prefix)/` expanded. A `:(optional)` path that does not exist is no value. Trust is
    /// not checked here: the caller decides whether an untrusted occurrence counts.
    Path,
}

/// The types [`convert`] reads from an occurrence. A path is not one of them: it needs an expansion context and
/// the optional-path rule of [`convert_path`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScalarType {
    /// A boolean, converted as git does (`yes`/`on`/`true`, `no`/`off`/`false`, integers); a valueless key is `true`.
    Bool,
    /// An integer, with git's `k`, `m` and `g` suffixes (powers of 1024) and `0x`/`0` prefixes.
    Int,
    /// The raw string value.
    String,
}

/// A configuration value converted to the scalar type it was read as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigValue {
    /// A boolean value.
    Bool(bool),
    /// An integer value.
    Int(i64),
    /// A string value.
    String(String),
}

/// The directories that `~` and `%(prefix)/` expand to when a path is read.
///
/// The install directory is read once when the context is built; a failure is kept and raised by [`convert_path`]
/// only for a path that uses `%(prefix)/`.
#[derive(Clone, Debug)]
pub struct PathContext {
    /// The install directory of the repository's git, for `%(prefix)/`, or the error that failed to find it.
    install_dir: Result<PathBuf, Arc<gix::Error>>,
    /// The home directory, for `~`.
    home_dir: Option<PathBuf>,
}

impl PathContext {
    /// The context of this process: the install directory of `opened`'s git and the home directory from the
    /// environment of the plugin process.
    #[must_use = "the context is the only result of this constructor"]
    pub fn of(opened: &OpenRepository) -> Self {
        Self {
            install_dir: opened.repo().install_dir().map_err(Arc::new),
            home_dir: home_dir(),
        }
    }

    /// The gix context for `path`, read by [`convert_path`] for the occurrence of `key` that has `origin`.
    ///
    /// A path that uses `%(prefix)/` needs the install directory: its failure is returned. Any other path ignores
    /// the install directory, so its failure is not an error here.
    ///
    /// # Errors
    ///
    /// Returns [`GixError::InstallDirUnavailable`] for a path that uses `%(prefix)/` while the install directory of
    /// this context is unknown.
    fn interpolation(
        &self,
        path: &ConfigPath,
        key: &ConfigKey,
        origin: &Arc<Origin>,
    ) -> Result<Context<'_>, GixError> {
        let git_install_dir = match (path.starts_with(PREFIX_MARKER), &self.install_dir) {
            (true, Err(source)) => {
                return Err(GixError::InstallDirUnavailable {
                    key: key.clone(),
                    source: Arc::clone(source),
                    origin: Arc::clone(origin),
                });
            }
            (_, install_dir) => install_dir.as_deref().ok(),
        };

        Ok(Context {
            git_install_dir,
            home_dir: self.home_dir.as_deref(),
            home_for_user: Some(home_for_user),
        })
    }
}

impl ScalarType {
    /// The name of the type, as the `--type` flag and the error messages spell it.
    const fn name(self) -> &'static str {
        match self {
            Self::Bool => BOOL_NAME,
            Self::Int => INT_NAME,
            Self::String => STRING_NAME,
        }
    }
}

impl fmt::Display for ConfigType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Scalar(ty) => ty.name(),
            Self::Path => PATH_NAME,
        };

        f.write_str(name)
    }
}

/// Converts `occurrence` of `key` to `ty`, the way git reads that type.
///
/// A valueless occurrence (`[s]\n\tkey`, no `=`) is `true` for [`ScalarType::Bool`], as `git config --type=bool`
/// reads it. It has no value for [`ScalarType::Int`] and [`ScalarType::String`], which is an error.
///
/// # Errors
///
/// Returns [`GixError::ConfigValueMissing`] for a valueless occurrence read as an integer or a string,
/// [`GixError::ConfigValueConversion`] when the value cannot be read as `ty`, and
/// [`GixError::ConfigValueNotUtf8`] when a string value is not valid UTF-8. Each error carries the origin of
/// `occurrence`.
pub fn convert(
    key: &ConfigKey,
    occurrence: &Occurrence,
    ty: ScalarType,
) -> Result<ConfigValue, GixError> {
    match ty {
        ScalarType::Bool => match &occurrence.value {
            RawValue::Valueless => Ok(ConfigValue::Bool(true)),
            RawValue::Bytes(bytes) => Boolean::try_from(BStr::new(bytes))
                .map(|boolean| ConfigValue::Bool(boolean.0))
                .map_err(|source| {
                    conversion_error(key, occurrence, ConfigType::Scalar(ty), source)
                }),
        },
        ScalarType::Int => match &occurrence.value {
            RawValue::Valueless => Err(missing_value(key, occurrence, ConfigType::Scalar(ty))),
            RawValue::Bytes(bytes) => Integer::from_bytes::<i64>(bytes)
                .map(ConfigValue::Int)
                .map_err(|source| {
                    conversion_error(key, occurrence, ConfigType::Scalar(ty), source)
                }),
        },
        ScalarType::String => match &occurrence.value {
            RawValue::Valueless => Err(missing_value(key, occurrence, ConfigType::Scalar(ty))),
            RawValue::Bytes(bytes) => from_utf8(bytes)
                .map(|text| ConfigValue::String(text.to_owned()))
                .map_err(|source| GixError::ConfigValueNotUtf8 {
                    key: key.clone(),
                    source,
                    origin: Arc::clone(&occurrence.origin),
                }),
        },
    }
}

/// Expands the path of `occurrence` of `key` as git does for a path value, with `context` for `~`, `~user` and
/// `%(prefix)/`.
///
/// A `:(optional)` prefix marks the path as optional. The prefix is dropped and the path is expanded; an
/// optional path that then cannot be stat'ed is `None`, as in gix's `trusted_path`, which ignores everything
/// that cannot be stat'ed instead of checking for a missing file only. A path without the prefix is returned
/// even when it does not exist. The expanded path is not checked for UTF-8.
///
/// # Errors
///
/// Returns [`GixError::ConfigValueMissing`] with [`ConfigType::Path`] for a valueless occurrence,
/// [`GixError::InstallDirUnavailable`] for a path that uses `%(prefix)/` while the install directory of `context`
/// is unknown, and [`GixError::ConfigValueConversion`] with [`ConfigType::Path`] when the path cannot be expanded
/// with `context`, which includes an empty value. Each error carries the origin of `occurrence`.
pub fn convert_path(
    key: &ConfigKey,
    occurrence: &Occurrence,
    context: &PathContext,
) -> Result<Option<PathBuf>, GixError> {
    let bytes = match &occurrence.value {
        RawValue::Valueless => {
            return Err(missing_value(key, occurrence, ConfigType::Path));
        }
        RawValue::Bytes(bytes) => bytes,
    };
    let path = ConfigPath::from(BStr::new(bytes));
    let is_optional = path.is_optional;
    let interpolation = context.interpolation(&path, key, &occurrence.origin)?;
    let expanded = path
        .interpolate(interpolation)
        .map_err(|source| conversion_error(key, occurrence, ConfigType::Path, source))?;

    if is_optional && expanded.metadata().is_err() {
        return Ok(None);
    }

    Ok(Some(expanded))
}

/// The error for a value of `key` in `occurrence` that cannot be read as `kind`.
fn conversion_error(
    key: &ConfigKey,
    occurrence: &Occurrence,
    kind: ConfigType,
    source: gix::Error,
) -> GixError {
    GixError::ConfigValueConversion {
        key: key.clone(),
        kind,
        source,
        origin: Arc::clone(&occurrence.origin),
    }
}

/// The error for a valueless `key` in `occurrence` that was needed as `kind`.
fn missing_value(key: &ConfigKey, occurrence: &Occurrence, kind: ConfigType) -> GixError {
    GixError::ConfigValueMissing {
        key: key.clone(),
        kind,
        origin: Arc::clone(&occurrence.origin),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::{Origin, SectionName, VariableName};

    use gix::config::Source;
    use rstest::rstest;

    use std::{assert_matches, fs, io, path::Path};

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Gix(#[from] GixError),

        #[error(transparent)]
        Io(#[from] io::Error),

        #[error("Test failure: {0}")]
        Failure(String),
    }

    const HOME: &str = "/h";
    const INSTALL_DIR: &str = "/usr";
    const OPTIONAL_PREFIX: &str = ":(optional)";
    const ORIGIN_PATH: &str = "/p/config";
    const PRESENT_FILE: &str = "present";
    const MISSING_FILE: &str = "missing";
    const NON_UTF8_TEMP_DIR: &str = "the temporary directory is not valid UTF-8";

    fn key() -> ConfigKey {
        ConfigKey {
            section: SectionName::new("gixtest"),
            subsection: None,
            name: VariableName::new("k"),
        }
    }

    fn occurrence(value: RawValue) -> Occurrence {
        Occurrence {
            value,
            origin: Arc::new(Origin {
                path: Some(PathBuf::from(ORIGIN_PATH)),
                source: Source::Local,
            }),
            trusted: true,
        }
    }

    fn context() -> PathContext {
        PathContext {
            install_dir: Ok(PathBuf::from(INSTALL_DIR)),
            home_dir: Some(PathBuf::from(HOME)),
        }
    }

    fn raw(text: &str) -> RawValue {
        RawValue::Bytes(text.as_bytes().to_vec())
    }

    fn is_origin_path(origin: &Origin) -> bool {
        origin.path.as_deref() == Some(Path::new(ORIGIN_PATH))
    }

    mod convert {
        use super::*;

        mod bool_values {
            use super::*;

            #[rstest]
            #[case::valueless(RawValue::Valueless, true)]
            #[case::yes(raw("yes"), true)]
            #[case::on(raw("on"), true)]
            #[case::true_word(raw("true"), true)]
            #[case::one(raw("1"), true)]
            #[case::no(raw("no"), false)]
            #[case::off(raw("off"), false)]
            #[case::false_word(raw("false"), false)]
            #[case::zero(raw("0"), false)]
            #[case::empty(raw(""), false)]
            fn reads_a_boolean(
                #[case] value: RawValue,
                #[case] expected: bool,
            ) -> Result<(), TestError> {
                assert_eq!(
                    convert(&key(), &occurrence(value), ScalarType::Bool)?,
                    ConfigValue::Bool(expected)
                );

                Ok(())
            }
        }

        mod int_values {
            use super::*;

            #[rstest]
            #[case::kilobytes("1k", 1024)]
            #[case::hexadecimal("0x10", 16)]
            fn reads_an_integer(
                #[case] text: &str,
                #[case] expected: i64,
            ) -> Result<(), TestError> {
                assert_eq!(
                    convert(&key(), &occurrence(raw(text)), ScalarType::Int)?,
                    ConfigValue::Int(expected)
                );

                Ok(())
            }
        }

        mod string_values {
            use super::*;

            #[rstest]
            fn reads_the_raw_string() -> Result<(), TestError> {
                assert_eq!(
                    convert(&key(), &occurrence(raw("Ada")), ScalarType::String)?,
                    ConfigValue::String("Ada".to_owned())
                );

                Ok(())
            }
        }

        mod valueless_values {
            use super::*;

            #[rstest]
            #[case::int(ScalarType::Int)]
            #[case::string(ScalarType::String)]
            fn reports_a_missing_value(#[case] ty: ScalarType) {
                let result = convert(&key(), &occurrence(RawValue::Valueless), ty);

                assert_matches!(
                    &result,
                    Err(GixError::ConfigValueMissing { kind, origin, .. })
                        if *kind == ConfigType::Scalar(ty) && is_origin_path(origin),
                    "got {result:?}"
                );
            }
        }

        mod errors {
            use super::*;

            #[rstest]
            #[case::bool_banana(raw("banana"), ScalarType::Bool)]
            #[case::int_word(raw("x"), ScalarType::Int)]
            fn reports_a_value_that_does_not_convert(
                #[case] value: RawValue,
                #[case] ty: ScalarType,
            ) {
                let result = convert(&key(), &occurrence(value), ty);

                assert_matches!(
                    &result,
                    Err(GixError::ConfigValueConversion { kind, origin, .. })
                        if *kind == ConfigType::Scalar(ty) && is_origin_path(origin),
                    "got {result:?}"
                );
            }

            #[rstest]
            fn rejects_a_string_that_is_not_utf8() {
                let result = convert(
                    &key(),
                    &occurrence(RawValue::Bytes(b"\xff".to_vec())),
                    ScalarType::String,
                );

                assert_matches!(
                    &result,
                    Err(GixError::ConfigValueNotUtf8 { key: error_key, origin, .. })
                        if *error_key == key() && is_origin_path(origin),
                    "got {result:?}"
                );
            }
        }
    }

    mod convert_path {
        use super::*;

        fn optional_value(path: &Path) -> RawValue {
            RawValue::Bytes(format!("{OPTIONAL_PREFIX}{}", path.display()).into_bytes())
        }

        mod expansion {
            use super::*;

            #[rstest]
            #[case::home_relative("~/x", "/h/x")]
            #[case::prefix_relative("%(prefix)/share", "/usr/share")]
            #[case::absolute("/srv/x", "/srv/x")]
            fn expands_a_path(#[case] text: &str, #[case] expected: &str) -> Result<(), TestError> {
                assert_eq!(
                    convert_path(&key(), &occurrence(raw(text)), &context())?,
                    Some(PathBuf::from(expected))
                );

                Ok(())
            }
        }

        mod install_dir {
            use super::*;

            const CAUSE: &str = "current_exe is unreadable";

            fn unavailable() -> PathContext {
                PathContext {
                    install_dir: Err(Arc::new(gix::Error::from_error(io::Error::other(CAUSE)))),
                    home_dir: Some(PathBuf::from(HOME)),
                }
            }

            #[rstest]
            #[case::home_relative("~/x", "/h/x")]
            #[case::absolute("/srv/x", "/srv/x")]
            fn works_without_the_install_dir_for_other_paths(
                #[case] text: &str,
                #[case] expected: &str,
            ) -> Result<(), TestError> {
                assert_eq!(
                    convert_path(&key(), &occurrence(raw(text)), &unavailable())?,
                    Some(PathBuf::from(expected))
                );

                Ok(())
            }

            #[rstest]
            #[case::prefix("%(prefix)/x")]
            #[case::optional_prefix(":(optional)%(prefix)/x")]
            fn names_the_cause_for_a_prefix_path(#[case] text: &str) {
                let result = convert_path(&key(), &occurrence(raw(text)), &unavailable());

                assert_matches!(
                    &result,
                    Err(GixError::InstallDirUnavailable { source, .. })
                        if source.to_string().contains(CAUSE),
                    "got {result:?}"
                );
            }
        }

        mod required {
            use super::*;

            #[rstest]
            fn keeps_a_missing_path_that_is_not_optional() -> Result<(), TestError> {
                let dir = tempfile::tempdir()?;
                let missing = dir.path().join(MISSING_FILE);
                let expected = missing
                    .to_str()
                    .ok_or_else(|| TestError::Failure(NON_UTF8_TEMP_DIR.to_owned()))?
                    .to_owned();

                assert_eq!(
                    convert_path(&key(), &occurrence(raw(&expected)), &context())?,
                    Some(PathBuf::from(expected))
                );

                Ok(())
            }

            #[rstest]
            fn reports_a_valueless_occurrence_as_missing() {
                let result = convert_path(&key(), &occurrence(RawValue::Valueless), &context());

                assert_matches!(
                    &result,
                    Err(GixError::ConfigValueMissing { kind, .. }) if *kind == ConfigType::Path,
                    "got {result:?}"
                );
            }
        }

        mod optional {
            use super::*;

            #[rstest]
            fn reads_no_value_for_a_missing_path() -> Result<(), TestError> {
                let dir = tempfile::tempdir()?;
                let missing = dir.path().join(MISSING_FILE);

                assert_eq!(
                    convert_path(&key(), &occurrence(optional_value(&missing)), &context())?,
                    None
                );

                Ok(())
            }

            #[rstest]
            fn reads_an_existing_path_without_the_prefix() -> Result<(), TestError> {
                let dir = tempfile::tempdir()?;
                let present = dir.path().join(PRESENT_FILE);
                fs::write(&present, b"")?;

                assert_eq!(
                    convert_path(&key(), &occurrence(optional_value(&present)), &context())?,
                    Some(present)
                );

                Ok(())
            }
        }

        mod errors {
            use super::*;

            #[rstest]
            #[case::home_missing(raw("~/x"), PathContext { home_dir: None, ..context() })]
            #[case::user_without_lookup(raw("~gixtest-no-such-user/x"), context())]
            #[case::optional_user_without_lookup(
                raw(":(optional)~gixtest-no-such-user/x"),
                context()
            )]
            fn reports_a_path_that_does_not_expand(
                #[case] value: RawValue,
                #[case] ctx: PathContext,
            ) {
                let result = convert_path(&key(), &occurrence(value), &ctx);

                assert_matches!(
                    &result,
                    Err(GixError::ConfigValueConversion { kind, .. }) if *kind == ConfigType::Path,
                    "got {result:?}"
                );
            }
        }
    }
}
