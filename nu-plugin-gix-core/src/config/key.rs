//! Names of configuration keys: sections, variables and subsections, and the parsed `section.name` key.

use crate::error::GixError;

use gix::{
    bstr::{BStr, BString},
    config::AsKey as _,
};

use std::{
    fmt,
    str::{self, Utf8Error},
};

const KEY_SEPARATOR: char = '.';

/// A section name, ASCII, lowercased at construction (git compares section names without case).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SectionName(String);

impl SectionName {
    /// Lowercases `text`; an invalid name is not rejected here, it simply matches nothing.
    #[must_use = "the name is only useful when stored or compared"]
    pub fn new(text: &str) -> Self {
        Self(text.to_ascii_lowercase())
    }

    /// The lowercased name.
    #[must_use = "the name is only useful when read"]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Lowercases the bytes of a name read from gix; names are ASCII (D4 25), so each byte maps to the char of the same value.
    #[must_use = "the name is the only result"]
    pub fn from_name(name: &BStr) -> Self {
        Self(
            name.iter()
                .map(|byte| char::from(byte.to_ascii_lowercase()))
                .collect(),
        )
    }
}

impl fmt::Display for SectionName {
    /// Writes the lowercased name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A variable name, ASCII, lowercased at construction (git compares variable names without case).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VariableName(String);

impl VariableName {
    /// Lowercases `text`; an invalid name is not rejected here, it simply matches nothing.
    #[must_use = "the name is only useful when stored or compared"]
    pub fn new(text: &str) -> Self {
        Self(text.to_ascii_lowercase())
    }

    /// The lowercased name.
    #[must_use = "the name is only useful when read"]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Lowercases the bytes of a name read from gix; names are ASCII (D4 25), so each byte maps to the char of the same value.
    #[must_use = "the name is the only result"]
    pub fn from_name(name: &BStr) -> Self {
        Self(
            name.iter()
                .map(|byte| char::from(byte.to_ascii_lowercase()))
                .collect(),
        )
    }
}

impl fmt::Display for VariableName {
    /// Writes the lowercased name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A subsection name: exact bytes (case-sensitive, maybe not UTF-8). A legacy `[section.Sub]` header is lowercased
/// by `Subsection::legacy`, as git does; every lookup is then an exact comparison.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Subsection(BString);

impl Subsection {
    /// The subsection spelled exactly as `bytes`.
    #[must_use = "the subsection is only useful when stored or compared"]
    pub fn new<B>(bytes: B) -> Self
    where
        B: AsRef<[u8]>,
    {
        Self(BString::from(bytes.as_ref()))
    }

    /// The subsection of a legacy `[section.Sub]` header: ASCII-lowercased, as git does.
    #[must_use = "the subsection is only useful when stored or compared"]
    pub fn legacy<B>(bytes: B) -> Self
    where
        B: AsRef<[u8]>,
    {
        Self(BString::from(bytes.as_ref().to_ascii_lowercase()))
    }

    /// The subsection as text; the only place a subsection is decoded.
    ///
    /// # Errors
    ///
    /// Returns the UTF-8 error when the subsection is not valid UTF-8.
    pub fn to_text(&self) -> Result<&str, Utf8Error> {
        str::from_utf8(&self.0)
    }
}

impl fmt::Display for Subsection {
    /// Writes the subsection; invalid bytes are shown as `\xNN`, for messages only.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for chunk in self.0.utf8_chunks() {
            f.write_str(chunk.valid())?;
            for byte in chunk.invalid() {
                write!(f, "\\x{byte:02X}")?;
            }
        }
        Ok(())
    }
}

/// A parsed `section.name` or `section.subsection.name`. Field order is the derived `Ord`: sections, then variables
/// without a subsection before those with one.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConfigKey {
    /// The section, lowercased.
    pub section: SectionName,
    /// The subsection, if the key has one.
    pub subsection: Option<Subsection>,
    /// The variable name, lowercased.
    pub name: VariableName,
}

impl ConfigKey {
    /// Parses `text` with gix's `AsKey::try_as_key`: the first `.` ends the section and the last `.` starts the name.
    /// An empty section or variable name is rejected as git does; an empty subsection (`a..b`) is valid.
    ///
    /// # Errors
    ///
    /// Returns [`GixError::InvalidConfigKey`] when `text` has no `.`, an empty section or an empty variable name.
    pub fn parse(text: &str) -> Result<Self, GixError> {
        let invalid = || invalid_key(text);
        let parts = text.try_as_key().ok_or_else(invalid)?;
        let section = non_empty(parts.section_name).ok_or_else(invalid)?;
        let name = non_empty(parts.value_name).ok_or_else(invalid)?;

        Ok(Self {
            section: SectionName::new(section),
            subsection: parts.subsection_name.map(Subsection::new),
            name: VariableName::new(name),
        })
    }

    /// The section, lowercased.
    #[must_use = "the component is the only result"]
    pub const fn section(&self) -> &SectionName {
        &self.section
    }

    /// The subsection, if the key has one.
    #[must_use = "the component is the only result"]
    pub const fn subsection(&self) -> Option<&Subsection> {
        self.subsection.as_ref()
    }

    /// The variable name, lowercased.
    #[must_use = "the component is the only result"]
    pub const fn name(&self) -> &VariableName {
        &self.name
    }
}

impl fmt::Display for ConfigKey {
    /// Writes `section[.subsection].name`; invalid subsection bytes are shown as `\xNN`, for messages only.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            section,
            subsection,
            name,
        } = self;
        match subsection {
            Some(subsection) => {
                write!(
                    f,
                    "{section}{KEY_SEPARATOR}{subsection}{KEY_SEPARATOR}{name}"
                )
            }
            None => write!(f, "{section}{KEY_SEPARATOR}{name}"),
        }
    }
}

/// What a `get` argument without a variable names: a section, or `section.subsection`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupPath {
    /// A section, such as `core`.
    Section(SectionName),
    /// A section and its subsection, such as `remote.origin`.
    Subsection(SectionName, Subsection),
}

impl GroupPath {
    /// A `text` without a `.` names a section. Otherwise everything after the first `.` is the subsection, and the
    /// acceptance of `text` is that of a key (`ConfigKey::parse`). Case is kept.
    ///
    /// # Errors
    ///
    /// Returns [`GixError::InvalidConfigKey`] for empty text, and for the same invalid keys as `ConfigKey::parse`.
    #[must_use = "the path is only useful when looked up"]
    pub fn parse(text: &str) -> Result<Self, GixError> {
        let Some((_, rest)) = text.split_once(KEY_SEPARATOR) else {
            let section = non_empty(text).ok_or_else(|| invalid_key(text))?;
            return Ok(Self::Section(SectionName::new(section)));
        };
        let key = ConfigKey::parse(text)?;
        Ok(Self::Subsection(key.section, Subsection::new(rest)))
    }
}

impl fmt::Display for GroupPath {
    /// Writes `section` or `section.subsection`; a subsection that is not UTF-8 is shown lossily, for messages only.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Section(section) => write!(f, "{section}"),
            Self::Subsection(section, subsection) => {
                write!(f, "{section}{KEY_SEPARATOR}{subsection}")
            }
        }
    }
}

/// The one rule for a key component: an empty section or variable name is invalid, as in git.
#[must_use = "the component is the only result"]
fn non_empty(part: &str) -> Option<&str> {
    (!part.is_empty()).then_some(part)
}

fn invalid_key(text: &str) -> GixError {
    GixError::InvalidConfigKey {
        key: text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use rstest::rstest;

    use std::assert_matches;

    #[derive(thiserror::Error, Debug)]
    enum TestError {
        #[error(transparent)]
        Gix(#[from] GixError),
        #[error("Test failure: {0}")]
        Failure(String),
    }

    mod section_name {
        use super::*;

        #[rstest]
        fn section_names_ignore_case() {
            assert_eq!(SectionName::new("A"), SectionName::new("a"));
            assert_eq!(SectionName::new("A").as_str(), "a");
        }

        #[rstest]
        #[case::mixed_case("NaMe-1", "name-1")]
        #[case::lower("core", "core")]
        fn from_name_folds_ascii(#[case] text: &str, #[case] expected: &str) {
            assert_eq!(
                SectionName::from_name(BStr::new(text)),
                SectionName::new(expected)
            );
        }

        #[rstest]
        #[case::mixed_case("Core", "core")]
        fn display(#[case] text: &str, #[case] expected: &str) {
            assert_eq!(SectionName::new(text).to_string(), expected);
        }
    }

    mod variable_name {
        use super::*;

        #[rstest]
        fn variable_names_ignore_case() {
            assert_eq!(VariableName::new("NAME"), VariableName::new("name"));
            assert_eq!(VariableName::new("NAME").as_str(), "name");
        }

        #[rstest]
        #[case::mixed_case("NaMe-1", "name-1")]
        #[case::lower("core", "core")]
        fn from_name_folds_ascii(#[case] text: &str, #[case] expected: &str) {
            assert_eq!(
                VariableName::from_name(BStr::new(text)),
                VariableName::new(expected)
            );
        }

        #[rstest]
        #[case::mixed_case("Core", "core")]
        fn display(#[case] text: &str, #[case] expected: &str) {
            assert_eq!(VariableName::new(text).to_string(), expected);
        }
    }

    mod subsection {
        use super::*;

        #[rstest]
        fn exact_differs_from_legacy() {
            assert_ne!(Subsection::new(b"Sub"), Subsection::legacy(b"Sub"));
        }

        #[rstest]
        fn legacy_is_lowercased() {
            assert_eq!(Subsection::legacy(b"Sub"), Subsection::new(b"sub"));
        }

        #[rstest]
        fn constructors_take_gix_values() {
            let bstr: &BStr = "Sub".into();
            assert_eq!(Subsection::legacy(bstr), Subsection::new("sub"));
            let maybe: Option<&BStr> = Some(bstr);
            assert_eq!(maybe.map(Subsection::new), Some(Subsection::new(b"Sub")));
        }

        #[rstest]
        fn invalid_utf8_has_no_text() -> Result<(), TestError> {
            let Err(_) = Subsection::new([0xff]).to_text() else {
                return Err(TestError::Failure(
                    "bytes [0xff] decoded as text".to_owned(),
                ));
            };
            Ok(())
        }

        #[rstest]
        fn display_escapes_invalid_bytes() {
            assert_eq!(Subsection::new([b'x', 0xff, b'y']).to_string(), "x\\xFFy");
        }

        #[rstest]
        fn display_keeps_valid_text() {
            assert_eq!(Subsection::new("a b").to_string(), "a b");
        }
    }

    mod config_key {
        use super::*;

        #[rstest]
        #[case::section_and_name("user.name", "user", None, "name")]
        #[case::subsection("remote.origin.url", "remote", Some("origin"), "url")]
        #[case::subsection_with_dot("a.X.Y.z", "a", Some("X.Y"), "z")]
        fn parses_into_parts(
            #[case] text: &str,
            #[case] section: &str,
            #[case] subsection: Option<&str>,
            #[case] name: &str,
        ) -> Result<(), TestError> {
            let parsed = ConfigKey::parse(text)?;
            assert_eq!(parsed.section(), &SectionName::new(section));
            assert_eq!(
                parsed.subsection(),
                subsection
                    .map(|value| Subsection::new(value.as_bytes()))
                    .as_ref()
            );
            assert_eq!(parsed.name(), &VariableName::new(name));
            Ok(())
        }

        #[rstest]
        fn key_without_subsection_sorts_before_subsection() -> Result<(), TestError> {
            assert!(ConfigKey::parse("a.x")? < ConfigKey::parse("a.b.x")?);
            Ok(())
        }

        #[rstest]
        #[case::no_dot("core")]
        #[case::empty_text("")]
        #[case::empty_name("a.")]
        #[case::empty_section(".b")]
        #[case::only_dot(".")]
        #[case::two_dots("..")]
        #[case::empty_name_with_subsection("a.x.")]
        #[case::empty_name_empty_subsection("a..")]
        fn invalid_keys(#[case] text: &str) {
            assert_matches!(
                ConfigKey::parse(text),
                Err(GixError::InvalidConfigKey { .. })
            );
        }

        #[rstest]
        fn empty_subsection_is_a_subsection() -> Result<(), TestError> {
            let parsed = ConfigKey::parse("a..b")?;
            assert_eq!(
                parsed,
                ConfigKey {
                    section: SectionName::new("a"),
                    subsection: Some(Subsection::new(b"")),
                    name: VariableName::new("b"),
                }
            );
            assert_ne!(parsed, ConfigKey::parse("a.b")?);
            Ok(())
        }

        #[rstest]
        #[case::section_and_name("user.name")]
        #[case::subsection("remote.origin.url")]
        #[case::subsection_with_dot("a.X.Y.z")]
        #[case::empty_subsection("a..b")]
        fn display_round_trips(#[case] text: &str) -> Result<(), TestError> {
            assert_eq!(ConfigKey::parse(text)?.to_string(), text);
            Ok(())
        }

        #[rstest]
        fn display_escapes_invalid_subsection_bytes() {
            let key = ConfigKey {
                section: SectionName::new("a"),
                subsection: Some(Subsection::new([b'x', 0xff, b'y'])),
                name: VariableName::new("k"),
            };
            assert_eq!(key.to_string(), "a.x\\xFFy.k");
        }
    }

    mod group_path {
        use super::*;

        #[rstest]
        fn section_without_dot() -> Result<(), TestError> {
            assert_eq!(
                GroupPath::parse("core")?,
                GroupPath::Section(SectionName::new("core"))
            );
            Ok(())
        }

        #[rstest]
        fn subsection_after_first_dot() -> Result<(), TestError> {
            assert_eq!(
                GroupPath::parse("remote.origin")?,
                GroupPath::Subsection(SectionName::new("remote"), Subsection::new(b"origin"))
            );
            Ok(())
        }

        #[rstest]
        fn subsection_may_contain_dots() -> Result<(), TestError> {
            assert_eq!(
                GroupPath::parse("remote.a.b")?,
                GroupPath::Subsection(SectionName::new("remote"), Subsection::new(b"a.b"))
            );
            Ok(())
        }

        #[rstest]
        #[case::empty_first_component("a..b", "a", ".b")]
        #[case::dotted_rest("a.b.c.d", "a", "b.c.d")]
        fn subsection_is_everything_after_first_dot(
            #[case] text: &str,
            #[case] section: &str,
            #[case] subsection: &str,
        ) -> Result<(), TestError> {
            assert_eq!(
                GroupPath::parse(text)?,
                GroupPath::Subsection(
                    SectionName::new(section),
                    Subsection::new(subsection.as_bytes())
                )
            );
            Ok(())
        }

        #[rstest]
        fn keeps_the_case_of_the_subsection() -> Result<(), TestError> {
            assert_eq!(
                GroupPath::parse("remote.Origin")?,
                GroupPath::Subsection(SectionName::new("remote"), Subsection::new(b"Origin"))
            );
            Ok(())
        }

        #[rstest]
        #[case::section(GroupPath::Section(SectionName::new("core")), "core")]
        #[case::subsection(
            GroupPath::Subsection(SectionName::new("remote"), Subsection::new(b"Origin")),
            "remote.Origin"
        )]
        #[case::dotted_subsection(
            GroupPath::Subsection(SectionName::new("remote"), Subsection::new(b"a.b")),
            "remote.a.b"
        )]
        fn display(#[case] group: GroupPath, #[case] expected: &str) {
            assert_eq!(group.to_string(), expected);
        }

        #[rstest]
        #[case::empty_text("")]
        #[case::empty_name("a.")]
        #[case::empty_section(".b")]
        #[case::empty_name_empty_subsection("a..")]
        fn invalid_groups(#[case] text: &str) {
            assert_matches!(
                GroupPath::parse(text),
                Err(GixError::InvalidConfigKey { .. })
            );
        }
    }
}
