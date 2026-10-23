//! Names of configuration keys: sections, variables and subsections, and the parsed `section.name` key.

use crate::error::GixError;

use gix::{bstr::BStr, config::AsKey as _};

use std::{
    cmp::Ordering,
    fmt,
    hash::{Hash, Hasher},
    str::{self, Utf8Error},
};

const KEY_SEPARATOR: &str = ".";

/// A section name, ASCII, lowercased at construction (git compares section names without case).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SectionName(Box<str>);

impl SectionName {
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
                .collect::<String>()
                .into_boxed_str(),
        )
    }
}

/// Infallible: folds ASCII case only; emptiness and `.` are checked by the key constructors.
impl<S> From<S> for SectionName
where
    S: AsRef<str>,
{
    /// Lowercases `text`; an invalid name is not rejected here, it simply matches nothing.
    fn from(text: S) -> Self {
        Self(text.as_ref().to_ascii_lowercase().into_boxed_str())
    }
}

impl fmt::Display for SectionName {
    /// Writes the lowercased name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A variable name. It keeps the spelling it was read with; equality, hashing and ordering use the lowercased form,
/// as git compares variable names without case.
#[derive(Clone, Debug)]
pub struct VariableName {
    /// The name as read, case kept.
    spelling: Box<str>,
    /// The name lowercased: the form that is compared, hashed and displayed.
    folded: Box<str>,
}

impl VariableName {
    /// Builds the name from its spelling; the folded form is derived here, the only place it is computed.
    fn new(spelling: String) -> Self {
        let folded = spelling.to_ascii_lowercase();
        Self {
            spelling: spelling.into_boxed_str(),
            folded: folded.into_boxed_str(),
        }
    }

    /// The lowercased name.
    #[must_use = "the name is only useful when read"]
    pub fn as_str(&self) -> &str {
        &self.folded
    }

    /// The name as read, case kept.
    #[must_use = "the spelling is only useful when read"]
    pub fn spelling(&self) -> &str {
        &self.spelling
    }

    /// Keeps the bytes of a name read from gix as chars; names are ASCII (D4 25), so each byte maps to the char of the same value.
    #[must_use = "the name is the only result"]
    pub fn from_name(name: &BStr) -> Self {
        Self::new(name.iter().map(|byte| char::from(*byte)).collect())
    }
}

impl PartialEq for VariableName {
    fn eq(&self, other: &Self) -> bool {
        self.folded == other.folded
    }
}

impl Eq for VariableName {}

impl PartialOrd for VariableName {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for VariableName {
    fn cmp(&self, other: &Self) -> Ordering {
        self.folded.cmp(&other.folded)
    }
}

impl Hash for VariableName {
    fn hash<H>(&self, state: &mut H)
    where
        H: Hasher,
    {
        self.folded.hash(state);
    }
}

impl<S> From<S> for VariableName
where
    S: AsRef<str>,
{
    /// Keeps `text` as spelled; an invalid name is not rejected here, it simply matches nothing.
    fn from(text: S) -> Self {
        Self::new(text.as_ref().to_owned())
    }
}

impl fmt::Display for VariableName {
    /// Writes the lowercased name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.folded)
    }
}

/// A subsection name: exact bytes (case-sensitive, maybe not UTF-8). No constructor folds case; a caller that reads a
/// legacy `[section.Sub]` header lowercases the bytes first, as git does.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Subsection(Box<[u8]>);

impl Subsection {
    /// The subsection as text; the only place a subsection is decoded.
    ///
    /// # Errors
    ///
    /// Returns the UTF-8 error when the subsection is not valid UTF-8.
    pub fn to_text(&self) -> Result<&str, Utf8Error> {
        str::from_utf8(&self.0)
    }

    /// The exact bytes of the subsection.
    #[must_use = "the bytes are only useful when read"]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl<B> From<B> for Subsection
where
    B: AsRef<[u8]>,
{
    /// The subsection spelled exactly as `bytes`, case kept.
    fn from(bytes: B) -> Self {
        Self(Box::from(bytes.as_ref()))
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
    section: SectionName,
    /// The subsection, if the key has one.
    subsection: Option<Subsection>,
    /// The variable name; compared and displayed lowercased, spelling kept.
    name: VariableName,
}

impl ConfigKey {
    /// Builds a key from components that are already structured: gix's config parser output (`Config::read`) and tests, including keys whose subsection is not UTF-8, which no text constructor can express. It does not check that the section or name is non-empty or free of `.`; user input goes through [`ConfigKey::parse`] or [`ConfigKey::from_members`].
    #[must_use = "the constructed key is the only result"]
    pub const fn new(
        section: SectionName,
        subsection: Option<Subsection>,
        name: VariableName,
    ) -> Self {
        Self {
            section,
            subsection,
            name,
        }
    }

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
            section: SectionName::from(section),
            subsection: parts.subsection_name.map(Subsection::from),
            name: VariableName::from(name),
        })
    }

    /// Builds a key from cell path `members`, already unquoted and never split or joined: two members are
    /// `section`, `name`; three are `section`, `subsection`, `name`, the subsection verbatim (any bytes, dots, or empty).
    /// The section and the name must be non-empty and contain no `.`.
    ///
    /// # Errors
    ///
    /// Returns [`GixError::InvalidConfigKey`], with the members joined by `.` as its key, for any other member count
    /// and for an empty section or name, or one that contains a `.`.
    pub fn from_members<I, S>(members: I) -> Result<Self, GixError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let owned: Vec<S> = members.into_iter().collect();
        let parts: Vec<&str> = owned.iter().map(S::as_ref).collect();
        Self::from_parts(&parts)
    }

    /// 2 parts: section, name; 3 parts: section, subsection, name (verbatim). Section and name via `member`.
    /// Other counts and invalid section/name => `invalid_members(parts)`.
    fn from_parts(parts: &[&str]) -> Result<Self, GixError> {
        let (section, subsection, name) = match parts {
            [section, name] => (*section, None, *name),
            [section, subsection, name] => (*section, Some(*subsection), *name),
            _ => return Err(invalid_members(parts)),
        };

        let section = member(section).ok_or_else(|| invalid_members(parts))?;
        let name = member(name).ok_or_else(|| invalid_members(parts))?;

        Ok(Self {
            section: SectionName::from(section),
            subsection: subsection.map(Subsection::from),
            name: VariableName::from(name),
        })
    }

    /// The group reading of the key: its subsection (if any), a `.`, then the name as spelled.
    /// `remote.a.B` reads as `a.B`, `remote.Origin` as `Origin`, `a..b` as `.b`. The section is [`Self::section`].
    #[must_use = "the subsection is the only result"]
    pub fn name_as_subsection(&self) -> Subsection {
        let spelling = self.name.spelling().as_bytes();

        self.subsection.as_ref().map_or_else(
            || Subsection::from(spelling),
            |subsection| {
                Subsection::from(
                    [subsection.as_bytes(), KEY_SEPARATOR.as_bytes(), spelling].concat(),
                )
            },
        )
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

    /// The variable name; compared and displayed lowercased, spelling kept.
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

/// What a `get` argument without `--type` names: a group, or a key that may also read as a group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LookupPath {
    /// A group only: a text without a `.` (a section, such as `core`), or two members whose second contains a `.`
    /// (the group `section.<member>`: a variable name cannot contain one).
    Group(GroupPath),
    /// A key read as a variable first, else as the group [`ConfigKey::name_as_subsection`]: two members, or any text
    /// with a `.`, such as `remote.origin` or `remote.a.B`.
    Dotted(ConfigKey),
    /// Exactly the variable of three members, with no group reading.
    Variable(ConfigKey),
}

impl LookupPath {
    /// A `text` without a `.` is a section, case folded. Any other `text` is a key (`ConfigKey::parse`) that is also
    /// read as a group: everything after the first `.`, with its case kept.
    ///
    /// # Errors
    ///
    /// Returns [`GixError::InvalidConfigKey`] for empty text, and for the same invalid keys as `ConfigKey::parse`.
    pub fn parse(text: &str) -> Result<Self, GixError> {
        if text.contains(KEY_SEPARATOR) {
            return ConfigKey::parse(text).map(Self::Dotted);
        }

        let section = non_empty(text).ok_or_else(|| invalid_key(text))?;
        Ok(Self::Group(GroupPath::Section(SectionName::from(section))))
    }

    /// Reads cell path `members`, already unquoted: two are [`Self::Dotted`], or a [`Self::Group`] when the second
    /// contains a `.`; three are a [`Self::Variable`] (`ConfigKey::from_members`). A lone section comes only from text.
    ///
    /// # Errors
    ///
    /// Returns [`GixError::InvalidConfigKey`], with the members joined by `.` as its key, for 0, 1 and 4 or more
    /// members, and for an empty section or name, or one that contains a `.`.
    pub fn from_members<I, S>(members: I) -> Result<Self, GixError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let owned: Vec<S> = members.into_iter().collect();
        let parts: Vec<&str> = owned.iter().map(S::as_ref).collect();

        match parts.as_slice() {
            [section, group] if group.contains(KEY_SEPARATOR) => {
                let section = member(section).ok_or_else(|| invalid_members(&parts))?;
                Ok(Self::Group(GroupPath::Subsection(
                    SectionName::from(section),
                    Subsection::from(group),
                )))
            }
            [_, _] => ConfigKey::from_parts(&parts).map(Self::Dotted),
            [_, _, _] => ConfigKey::from_parts(&parts).map(Self::Variable),
            _ => Err(invalid_members(&parts)),
        }
    }
}

/// The one rule for a key component: an empty section or variable name is invalid, as in git.
#[must_use = "the component is the only result"]
fn non_empty(part: &str) -> Option<&str> {
    (!part.is_empty()).then_some(part)
}

/// A cell path member used as a section or variable name: non-empty and without a `.`.
#[must_use = "the component is the only result"]
fn member(part: &str) -> Option<&str> {
    non_empty(part).filter(|text| !text.contains(KEY_SEPARATOR))
}

fn invalid_key(text: &str) -> GixError {
    GixError::InvalidConfigKey {
        key: text.to_owned(),
    }
}

/// The members joined by `.` as an invalid key.
fn invalid_members(parts: &[&str]) -> GixError {
    invalid_key(&parts.join(KEY_SEPARATOR))
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
            assert_eq!(SectionName::from("A"), SectionName::from("a"));
            assert_eq!(SectionName::from("A").as_str(), "a");
        }

        #[rstest]
        #[case::owned(String::from("Core"), "core")]
        fn from_string_folds_ascii(#[case] text: String, #[case] expected: &str) {
            assert_eq!(SectionName::from(text), SectionName::from(expected));
        }

        #[rstest]
        #[case::mixed_case("NaMe-1", "name-1")]
        #[case::lower("core", "core")]
        fn from_name_folds_ascii(#[case] text: &str, #[case] expected: &str) {
            assert_eq!(
                SectionName::from_name(BStr::new(text)),
                SectionName::from(expected)
            );
        }

        #[rstest]
        #[case::mixed_case("Core", "core")]
        fn display(#[case] text: &str, #[case] expected: &str) {
            assert_eq!(SectionName::from(text).to_string(), expected);
        }
    }

    mod variable_name {
        use super::*;

        use std::collections::HashSet;

        #[rstest]
        fn variable_names_ignore_case() {
            assert_eq!(VariableName::from("NAME"), VariableName::from("name"));
            assert_eq!(VariableName::from("NAME").as_str(), "name");
        }

        #[rstest]
        fn spelling_keeps_case_while_as_str_folds() {
            let name = VariableName::from("NAME");
            assert_eq!(name.spelling(), "NAME");
            assert_eq!(name.as_str(), "name");
        }

        #[rstest]
        fn equal_spellings_hash_equal() {
            let set = HashSet::from([VariableName::from("A")]);
            assert!(set.contains(&VariableName::from("a")));
        }

        #[rstest]
        fn ordering_compares_folded_text() {
            assert_eq!(
                VariableName::from("A").cmp(&VariableName::from("b")),
                Ordering::Less
            );
            assert_eq!(
                VariableName::from("B").cmp(&VariableName::from("a")),
                Ordering::Greater
            );
        }

        #[rstest]
        fn from_name_keeps_spelling() {
            assert_eq!(VariableName::from_name(BStr::new("Url")).spelling(), "Url");
        }

        #[rstest]
        #[case::owned(String::from("Core"), "core")]
        fn from_string_folds_ascii(#[case] text: String, #[case] expected: &str) {
            assert_eq!(VariableName::from(text), VariableName::from(expected));
        }

        #[rstest]
        #[case::mixed_case("NaMe-1", "name-1")]
        #[case::lower("core", "core")]
        fn from_name_folds_ascii(#[case] text: &str, #[case] expected: &str) {
            assert_eq!(
                VariableName::from_name(BStr::new(text)),
                VariableName::from(expected)
            );
        }

        #[rstest]
        #[case::mixed_case("Core", "core")]
        fn display(#[case] text: &str, #[case] expected: &str) {
            assert_eq!(VariableName::from(text).to_string(), expected);
        }
    }

    mod subsection {
        use super::*;

        #[rstest]
        fn from_keeps_exact_bytes_from_text_bytes_and_gix_values() {
            let bstr: &BStr = "Sub".into();
            assert_eq!(Subsection::from("Sub"), Subsection::from(b"Sub"));
            assert_eq!(Subsection::from(bstr), Subsection::from("Sub"));
            assert_ne!(Subsection::from(bstr), Subsection::from("sub"));
        }

        #[rstest]
        fn invalid_utf8_has_no_text() -> Result<(), TestError> {
            let Err(_) = Subsection::from([0xff]).to_text() else {
                return Err(TestError::Failure(
                    "bytes [0xff] decoded as text".to_owned(),
                ));
            };
            Ok(())
        }

        #[rstest]
        fn display_escapes_invalid_bytes() {
            assert_eq!(Subsection::from([b'x', 0xff, b'y']).to_string(), "x\\xFFy");
        }

        #[rstest]
        fn display_keeps_valid_text() {
            assert_eq!(Subsection::from("a b").to_string(), "a b");
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
            assert_eq!(parsed.section(), &SectionName::from(section));
            assert_eq!(
                parsed.subsection(),
                subsection
                    .map(|value| Subsection::from(value.as_bytes()))
                    .as_ref()
            );
            assert_eq!(parsed.name(), &VariableName::from(name));
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
                    section: SectionName::from("a"),
                    subsection: Some(Subsection::from(b"")),
                    name: VariableName::from("b"),
                }
            );
            assert_ne!(parsed, ConfigKey::parse("a.b")?);
            Ok(())
        }

        #[rstest]
        fn text_rule_keeps_url_subsection() -> Result<(), TestError> {
            let parsed = ConfigKey::parse("url.https://example.com/.insteadOf")?;
            assert_eq!(
                parsed.subsection(),
                Some(&Subsection::from("https://example.com/"))
            );
            assert_eq!(parsed.name(), &VariableName::from("insteadof"));
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
                section: SectionName::from("a"),
                subsection: Some(Subsection::from([b'x', 0xff, b'y'])),
                name: VariableName::from("k"),
            };
            assert_eq!(key.to_string(), "a.x\\xFFy.k");
        }

        #[rstest]
        fn name_as_subsection_keeps_non_utf8_bytes() {
            let key = ConfigKey::new(
                SectionName::from("a"),
                Some(Subsection::from([0xff])),
                VariableName::from("K"),
            );
            assert_eq!(
                key.name_as_subsection(),
                Subsection::from([0xff, b'.', b'K'])
            );
        }

        mod from_members {
            use super::*;

            #[rstest]
            fn dotted_subsection_member_equals_text_key() -> Result<(), TestError> {
                assert_eq!(
                    ConfigKey::from_members(["remote", "a.b", "url"])?,
                    ConfigKey::parse("remote.a.b.url")?
                );
                Ok(())
            }

            #[rstest]
            fn three_members_split_into_subsection_and_name() -> Result<(), TestError> {
                let key = ConfigKey::from_members(["remote", "a", "b"])?;
                assert_eq!(key.subsection(), Some(&Subsection::from("a")));
                assert_eq!(key.name(), &VariableName::from("b"));
                Ok(())
            }

            #[rstest]
            fn two_members_have_no_subsection() -> Result<(), TestError> {
                let key = ConfigKey::from_members(["core", "bare"])?;
                assert_eq!(key.subsection(), None);
                assert_eq!(key.section(), &SectionName::from("core"));
                Ok(())
            }

            #[rstest]
            fn empty_subsection_member_is_valid() -> Result<(), TestError> {
                let key = ConfigKey::from_members(["a", "", "b"])?;
                assert_eq!(key.subsection(), Some(&Subsection::from("")));
                Ok(())
            }

            #[rstest]
            #[case::none(&[])]
            #[case::one(&["core"])]
            #[case::four(&["a", "b", "c", "d"])]
            #[case::dot_in_section(&["re.mote", "x"])]
            #[case::dot_in_name_and_subsection(&["remote", "a.b", "c.d"])]
            #[case::dot_in_name(&["remote", "x", "b.c"])]
            #[case::dot_in_two_member_name(&["remote", "b.c"])]
            #[case::empty_section(&["", "x"])]
            #[case::empty_name(&["core", ""])]
            #[case::empty_name_with_subsection(&["a", "b", ""])]
            fn rejects_invalid_members(#[case] members: &[&str]) {
                assert_matches!(
                    ConfigKey::from_members(members),
                    Err(GixError::InvalidConfigKey { .. })
                );
            }

            #[rstest]
            fn error_key_is_members_joined_with_dots() {
                assert_matches!(
                    ConfigKey::from_members(["a", "b", "c", "d"]),
                    Err(GixError::InvalidConfigKey { key }) if key == "a.b.c.d"
                );
            }
        }
    }

    mod group_path {
        use super::*;

        #[rstest]
        #[case::section(GroupPath::Section(SectionName::from("core")), "core")]
        #[case::subsection(
            GroupPath::Subsection(SectionName::from("remote"), Subsection::from(b"Origin")),
            "remote.Origin"
        )]
        #[case::dotted_subsection(
            GroupPath::Subsection(SectionName::from("remote"), Subsection::from(b"a.b")),
            "remote.a.b"
        )]
        fn display(#[case] group: GroupPath, #[case] expected: &str) {
            assert_eq!(group.to_string(), expected);
        }
    }

    mod lookup_path {
        use super::*;

        fn dotted(text: &str) -> Result<(ConfigKey, Subsection), TestError> {
            match LookupPath::parse(text)? {
                LookupPath::Dotted(variable) => {
                    let subsection = variable.name_as_subsection();
                    Ok((variable, subsection))
                }
                other => Err(TestError::Failure(format!(
                    "{text} parsed as {other:?}, not dotted"
                ))),
            }
        }

        #[rstest]
        fn text_without_dot_is_a_section_case_folded() -> Result<(), TestError> {
            assert_eq!(
                LookupPath::parse("Core")?,
                LookupPath::Group(GroupPath::Section(SectionName::from("core")))
            );
            Ok(())
        }

        #[rstest]
        #[case::plain_variable("remote.origin", "remote", None, "origin")]
        #[case::name_ignores_case("remote.Origin", "remote", None, "origin")]
        #[case::last_dot_starts_name("remote.a.b", "remote", Some("a"), "b")]
        #[case::empty_subsection("a..b", "a", Some(""), "b")]
        fn variable_reading(
            #[case] text: &str,
            #[case] section: &str,
            #[case] subsection: Option<&str>,
            #[case] name: &str,
        ) -> Result<(), TestError> {
            assert_eq!(
                dotted(text)?.0,
                ConfigKey::new(
                    SectionName::from(section),
                    subsection.map(|value| Subsection::from(value.as_bytes())),
                    VariableName::from(name),
                )
            );
            Ok(())
        }

        #[rstest]
        #[case::plain("remote.origin", "remote", "origin")]
        #[case::keeps_case("remote.Origin", "remote", "Origin")]
        #[case::dotted_rest("remote.a.b", "remote", "a.b")]
        #[case::dotted_rest_keeps_case("remote.a.B", "remote", "a.B")]
        #[case::empty_first_component("a..b", "a", ".b")]
        fn group_reading(
            #[case] text: &str,
            #[case] section: &str,
            #[case] expected: &str,
        ) -> Result<(), TestError> {
            assert_eq!(dotted(text)?.0.section(), &SectionName::from(section));
            assert_eq!(dotted(text)?.1, Subsection::from(expected.as_bytes()));
            Ok(())
        }

        #[rstest]
        #[case::empty_text("")]
        #[case::empty_name("remote.")]
        #[case::empty_section(".x")]
        #[case::empty_name_with_subsection("a.b.")]
        fn invalid_paths(#[case] text: &str) {
            assert_matches!(
                LookupPath::parse(text),
                Err(GixError::InvalidConfigKey { .. })
            );
        }

        mod from_members {
            use super::*;

            #[rstest]
            fn two_members_are_dotted_with_the_spelled_group() -> Result<(), TestError> {
                let LookupPath::Dotted(key) = LookupPath::from_members(["remote", "Origin"])?
                else {
                    return Err(TestError::Failure("not Dotted".to_owned()));
                };
                assert_eq!(key.name_as_subsection(), Subsection::from("Origin"));
                Ok(())
            }

            #[rstest]
            fn second_member_with_dot_is_only_a_group() -> Result<(), TestError> {
                assert_eq!(
                    LookupPath::from_members(["remote", "a.b"])?,
                    LookupPath::Group(GroupPath::Subsection(
                        SectionName::from("remote"),
                        Subsection::from("a.b")
                    ))
                );
                Ok(())
            }

            #[rstest]
            fn three_members_are_a_variable_without_group() -> Result<(), TestError> {
                let LookupPath::Variable(key) = LookupPath::from_members(["remote", "a", "B"])?
                else {
                    return Err(TestError::Failure("not Variable".to_owned()));
                };
                assert_eq!(key, ConfigKey::parse("remote.a.b")?);
                Ok(())
            }

            #[rstest]
            #[case::none(&[], "")]
            #[case::one_member(&["core"], "core")]
            #[case::four(&["a", "b", "c", "d"], "a.b.c.d")]
            #[case::dot_in_section(&["re.mote", "x"], "re.mote.x")]
            #[case::dot_in_section_of_group(&["re.mote", "x.y"], "re.mote.x.y")]
            #[case::empty_section_of_group(&["", "x.y"], ".x.y")]
            fn rejects_invalid_members(#[case] members: &[&str], #[case] expected_key: &str) {
                assert_matches!(
                    LookupPath::from_members(members),
                    Err(GixError::InvalidConfigKey { key }) if key == expected_key
                );
            }
        }
    }
}
