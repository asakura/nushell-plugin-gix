//! Names of configuration keys: sections, variables and subsections, and the structured `section.name` key.

use gix::bstr::BStr;

use std::{
    fmt,
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

/// A variable name, ASCII, lowercased at construction (git compares variable names without case).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VariableName(Box<str>);

impl VariableName {
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

impl<S> From<S> for VariableName
where
    S: AsRef<str>,
{
    /// Lowercases `text`; an invalid name is not rejected here, it simply matches nothing.
    fn from(text: S) -> Self {
        Self(text.as_ref().to_ascii_lowercase().into_boxed_str())
    }
}

impl fmt::Display for VariableName {
    /// Writes the lowercased name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
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

/// A structured `section.name` or `section.subsection.name`. Field order is the derived `Ord`: sections, then variables
/// without a subsection before those with one.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConfigKey {
    /// The section, lowercased.
    section: SectionName,
    /// The subsection, if the key has one.
    subsection: Option<Subsection>,
    /// The variable name, lowercased.
    name: VariableName,
}

impl ConfigKey {
    /// Builds a key from components that are already structured: gix's config parser output (`Config::read`) and tests, including keys whose subsection is not UTF-8, which no text constructor can express. It does not check that the section or name is non-empty or free of `.`; user input is split by the caller first.
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

    /// `section[.subsection].name`, exactly what `Display` writes, but only when the subsection is valid UTF-8.
    ///
    /// # Errors
    ///
    /// Returns the UTF-8 error of [`Subsection::to_text`].
    pub fn to_text(&self) -> Result<String, Utf8Error> {
        if let Some(subsection) = &self.subsection {
            subsection.to_text()?;
        }
        Ok(self.to_string())
    }
}

#[cfg(test)]
impl ConfigKey {
    /// Splits `text` like the plugin does (the first `.` ends the section, the last `.` starts the name, the middle is
    /// the subsection verbatim) for tests; valid text only, anything else yields empty parts.
    pub(crate) fn for_test(text: &str) -> Self {
        let (section, rest) = text.split_once(KEY_SEPARATOR).unwrap_or_default();
        let (subsection, name) = match rest.rsplit_once(KEY_SEPARATOR) {
            Some((subsection, name)) => (Some(Subsection::from(subsection)), name),
            None => (None, rest),
        };

        Self::new(
            SectionName::from(section),
            subsection,
            VariableName::from(name),
        )
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

/// The header of a configuration section: `section` or `section.subsection`, to name it in messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionHeader {
    /// The section, such as `remote`.
    pub section: SectionName,
    /// The subsection, such as `origin`, when the header has one.
    pub subsection: Option<Subsection>,
}

impl fmt::Display for SectionHeader {
    /// Writes `section` or `section.subsection`; invalid subsection bytes are shown as `\xNN` like `ConfigKey` does, for messages only.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.section)?;
        if let Some(subsection) = &self.subsection {
            write!(f, "{KEY_SEPARATOR}{subsection}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use rstest::rstest;

    #[derive(thiserror::Error, Debug)]
    enum TestError {
        #[error(transparent)]
        Utf8(#[from] Utf8Error),
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
        fn variable_name_is_lowercased() {
            assert_eq!(VariableName::from("Bare").as_str(), "bare");
            assert_eq!(VariableName::from_name(BStr::new("Bare")).as_str(), "bare");
        }

        #[rstest]
        fn equal_spellings_hash_equal() {
            let set = HashSet::from([VariableName::from("A")]);
            assert!(set.contains(&VariableName::from("a")));
        }

        #[rstest]
        fn ordering_compares_folded_text() {
            assert!(VariableName::from("A") < VariableName::from("b"));
            assert!(VariableName::from("B") > VariableName::from("a"));
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
        fn new_keeps_parts(
            #[case] text: &str,
            #[case] section: &str,
            #[case] subsection: Option<&str>,
            #[case] name: &str,
        ) {
            let key = ConfigKey::for_test(text);
            assert_eq!(key.section(), &SectionName::from(section));
            assert_eq!(
                key.subsection(),
                subsection
                    .map(|value| Subsection::from(value.as_bytes()))
                    .as_ref()
            );
            assert_eq!(key.name(), &VariableName::from(name));
        }

        #[rstest]
        fn key_without_subsection_sorts_before_subsection() {
            assert!(ConfigKey::for_test("a.x") < ConfigKey::for_test("a.b.x"));
        }

        #[rstest]
        fn empty_subsection_is_a_subsection() {
            assert_eq!(
                ConfigKey::for_test("a..b"),
                ConfigKey::new(
                    SectionName::from("a"),
                    Some(Subsection::from(b"")),
                    VariableName::from("b"),
                )
            );
            assert_ne!(ConfigKey::for_test("a..b"), ConfigKey::for_test("a.b"));
        }

        #[rstest]
        #[case::section_and_name("user.name")]
        #[case::subsection("remote.origin.url")]
        #[case::subsection_with_dot("a.X.Y.z")]
        #[case::empty_subsection("a..b")]
        fn display_round_trips(#[case] text: &str) {
            assert_eq!(ConfigKey::for_test(text).to_string(), text);
        }

        #[rstest]
        fn display_escapes_invalid_subsection_bytes() {
            let key = ConfigKey::new(
                SectionName::from("a"),
                Some(Subsection::from([b'x', 0xff, b'y'])),
                VariableName::from("k"),
            );
            assert_eq!(key.to_string(), "a.x\\xFFy.k");
        }

        mod to_text {
            use super::*;

            use std::assert_matches;

            #[rstest]
            #[case::section_and_name("user.name")]
            #[case::subsection("remote.origin.url")]
            #[case::subsection_with_dot("a.X.Y.z")]
            #[case::empty_subsection("a..b")]
            fn is_display_when_utf8(#[case] text: &str) -> Result<(), TestError> {
                assert_eq!(ConfigKey::for_test(text).to_text()?, text);
                Ok(())
            }

            #[rstest]
            fn rejects_invalid_subsection_bytes() {
                let key = ConfigKey::new(
                    SectionName::from("a"),
                    Some(Subsection::from([b'x', 0xff, b'y'])),
                    VariableName::from("k"),
                );
                assert_matches!(key.to_text(), Err(_));
            }
        }
    }

    mod section_header {
        use super::*;

        #[rstest]
        #[case::section(None, "remote")]
        #[case::dotted_subsection(Some(Subsection::from("a.b")), "remote.a.b")]
        #[case::invalid_bytes(Some(Subsection::from([b'x', 0xff, b'y'])), "remote.x\\xFFy")]
        fn section_header_display(#[case] subsection: Option<Subsection>, #[case] expected: &str) {
            let header = SectionHeader {
                section: SectionName::from("remote"),
                subsection,
            };
            assert_eq!(header.to_string(), expected);
        }
    }
}
