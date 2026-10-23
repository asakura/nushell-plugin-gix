//! The raw occurrences of the layered configuration: every `name = value` as git reads it, nothing selected or decoded.

use super::{
    ConfigKey, Origin, SectionHeader, SectionName, Subsection, VariableName, tree::ConfigTree,
};
use crate::error::{GixError, SectionFault};
use crate::repository::OpenRepository;

use gix::{
    bstr::BStr,
    config::{
        parse::{EventRef, Events},
        section::is_trusted,
        value::normalize,
    },
};

use std::{collections::BTreeMap, sync::Arc};

/// The value of one occurrence, as git stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawValue {
    /// A name written without `=`.
    Valueless,
    /// The text after `=`, quotes and continuations already resolved; empty for `name =`.
    Bytes(Vec<u8>),
}

/// One `name = value` occurrence of the layered configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Occurrence {
    /// The value as written.
    pub value: RawValue,
    /// Where the occurrence was read from; shared by all occurrences of one section.
    pub origin: Arc<Origin>,
    /// Whether its section passes gix's trust filter, so that its path values may be used.
    pub trusted: bool,
}

/// Every occurrence of the layered configuration, in load order; nothing selected.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// The occurrences of each key in load order; a key is present only with at least one occurrence.
    variables: BTreeMap<ConfigKey, Vec<Occurrence>>,
}

impl Config {
    /// Reads every occurrence of the layered configuration of `opened`, in load order. Subsections and values are
    /// kept as bytes.
    ///
    /// # Errors
    ///
    /// Returns [`GixError::ConfigSectionUnreadable`] when a section cannot be parsed again from its own bytes, and
    /// [`GixError::ConfigSectionMalformed`] when its parse events break the grammar of a variable.
    pub fn read(opened: &OpenRepository) -> Result<Self, GixError> {
        let repo = opened.repo();
        let snapshot = repo.config_snapshot();
        let mut config = Self::default();

        for section in snapshot.plumbing().sections() {
            let header = section.header();
            let section_name = SectionName::from_name(header.name());

            // git lowercases a legacy header's subsection; every lookup is then exact.
            let subsection = header.subsection_name().map(|bytes| {
                if header.is_legacy() {
                    Subsection::from(bytes.to_ascii_lowercase())
                } else {
                    Subsection::from(bytes)
                }
            });

            let group = || SectionHeader {
                section: section_name.clone(),
                subsection: subsection.clone(),
            };

            let origin = Arc::new(Origin::new(section.meta()));
            let trusted = is_trusted(section.meta());

            let bytes = section.to_bstring();

            let events = Events::from_bytes(&bytes, None).map_err(|source| {
                GixError::ConfigSectionUnreadable {
                    section: group(),
                    source,
                }
            })?;

            for entry in section_values(events.iter()) {
                let (name, value) = entry.map_err(|fault| GixError::ConfigSectionMalformed {
                    section: group(),
                    fault,
                })?;

                let key = ConfigKey::new(section_name.clone(), subsection.clone(), name);

                let occurrence = Occurrence {
                    value,
                    origin: Arc::clone(&origin),
                    trusted,
                };

                config.insert(key, occurrence);
            }
        }

        Ok(config)
    }

    /// The occurrences of `key`, in load order; empty when the key is unset.
    #[must_use = "the occurrences are the only result of this lookup"]
    pub fn occurrences(&self, key: &ConfigKey) -> &[Occurrence] {
        self.variables.get(key).map_or(&[], Vec::as_slice)
    }

    /// Every key with its occurrences, in key order.
    #[must_use = "the iterator is the only result of this lookup"]
    pub fn iter(&self) -> impl Iterator<Item = (&ConfigKey, &[Occurrence])> {
        self.variables
            .iter()
            .map(|(key, occurrences)| (key, occurrences.as_slice()))
    }

    /// The nested tree of every variable, built from [`Config::iter`] without re-reading anything.
    #[must_use = "the tree is the only result"]
    pub fn tree(&self) -> ConfigTree<'_> {
        ConfigTree::new(self)
    }

    /// Groups `occurrences` under their key, keeping the order of each key's occurrences. Called by [`Config::read`] and
    /// by callers that need a `Config` without a repository.
    #[must_use = "the configuration is the only result of this constructor"]
    pub fn from_occurrences<I>(occurrences: I) -> Self
    where
        I: IntoIterator<Item = (ConfigKey, Occurrence)>,
    {
        let mut config = Self::default();

        for (key, occurrence) in occurrences {
            config.insert(key, occurrence);
        }

        config
    }

    /// Appends `occurrence` to the occurrences of `key`, after those already read.
    fn insert(&mut self, key: ConfigKey, occurrence: Occurrence) {
        self.variables.entry(key).or_default().push(occurrence);
    }
}

/// A variable whose value is still being read from the parse events of its section.
struct Pending {
    /// The variable name, lowercased.
    name: VariableName,
    /// Whether a `=` followed the name.
    has_separator: bool,
    /// The raw bytes of the value so far, continuations included.
    parts: Vec<u8>,
}

impl Pending {
    /// Ends the value with `last` and returns it; a name without `=` is valueless.
    fn finish(mut self, last: &[u8]) -> (VariableName, RawValue) {
        self.parts.extend_from_slice(last);

        let value = if self.has_separator {
            RawValue::Bytes(Vec::from(normalize(self.parts.as_slice()).into_owned()))
        } else {
            RawValue::Valueless
        };

        (self.name, value)
    }
}

/// Where the walk over the parse events of one section stands.
enum Reading {
    /// Outside any variable: the next name starts one.
    Between,
    /// A name was read and its value is still being read.
    Name(Pending),
}

/// What one parse event means for the walk over a section.
enum EventClass<'a> {
    /// The name of a variable.
    Name(&'a BStr),
    /// The `=` after a name.
    Separator,
    /// A part of a value continued over lines.
    Part(&'a [u8]),
    /// The last part of a value.
    End(&'a [u8]),
    /// A header, comment or whitespace: no variable data.
    Skipped,
}

/// Names what `event` means for the walk over a section.
#[expect(
    clippy::rest_pattern_accessible_field,
    reason = "Comment and SectionHeader carry no variable data; naming their fields trips unneeded_field_pattern, and an or-pattern cannot bind different names"
)]
fn classify(event: EventRef<'_>) -> EventClass<'_> {
    match event {
        EventRef::SectionValueName(name) => EventClass::Name(name),
        EventRef::KeyValueSeparator => EventClass::Separator,
        EventRef::ValueNotDone(part) => EventClass::Part(part),
        EventRef::Value(last) | EventRef::ValueDone(last) => EventClass::End(last),
        EventRef::Comment { .. }
        | EventRef::SectionHeader { .. }
        | EventRef::Newline(_)
        | EventRef::Whitespace(_) => EventClass::Skipped,
    }
}

/// The `(name, value)` pairs of the parse events of one section, in order.
///
/// gix emits, for each variable, a `SectionValueName`, then an optional `KeyValueSeparator`, then zero or more
/// `ValueNotDone` and one terminal `Value` or `ValueDone` (a valueless name is `Value("")` without a separator). This is
/// the grammar enforced here: any other transition is a [`SectionFault`], never skipped and never replaced. A name followed
/// by `=` is `Bytes`, possibly empty; a name without `=` is `Valueless`. A value continued over lines is joined before it is
/// normalized, as gix's body iterator does.
///
/// Each variable is yielded as soon as its value ends. A fault is yielded as the last item. After the last item, or after
/// a fault, the iterator is finished and stays so.
///
/// `Whitespace`, `Newline`, `Comment` and header events carry no variable data and are valid in both states.
///
/// # Errors
///
/// Yields [`SectionFault::NameWithoutValue`] when a name is followed by another name or by the end of the events, and
/// [`SectionFault::ValueWithoutName`] when a value event arrives with no name pending.
struct SectionValues<I> {
    /// The parse events still to read.
    events: I,
    /// Where the walk stands; `None` once the iterator is finished.
    reading: Option<Reading>,
}

impl<'a, I> Iterator for SectionValues<I>
where
    I: Iterator<Item = EventRef<'a>>,
{
    type Item = Result<(VariableName, RawValue), SectionFault>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let state = self.reading.take()?;
            let class = self.events.next().map(classify);

            match (state, class) {
                (Reading::Between, None) => return None,
                (Reading::Between, Some(EventClass::Name(name))) => {
                    self.reading = Some(Reading::Name(Pending {
                        name: VariableName::from_name(name),
                        has_separator: false,
                        parts: Vec::new(),
                    }));
                }
                (Reading::Between, Some(EventClass::Skipped)) => {
                    self.reading = Some(Reading::Between);
                }
                (
                    Reading::Between,
                    Some(EventClass::Separator | EventClass::Part(_) | EventClass::End(_)),
                ) => return Some(Err(SectionFault::ValueWithoutName)),
                (Reading::Name(pending), Some(EventClass::Name(_)) | None) => {
                    return Some(Err(SectionFault::NameWithoutValue(pending.name)));
                }
                (Reading::Name(mut pending), Some(EventClass::Separator)) => {
                    pending.has_separator = true;
                    self.reading = Some(Reading::Name(pending));
                }
                (Reading::Name(mut pending), Some(EventClass::Part(part))) => {
                    pending.parts.extend_from_slice(part);
                    self.reading = Some(Reading::Name(pending));
                }
                (Reading::Name(pending), Some(EventClass::End(last))) => {
                    self.reading = Some(Reading::Between);
                    return Some(Ok(pending.finish(last)));
                }
                (Reading::Name(pending), Some(EventClass::Skipped)) => {
                    self.reading = Some(Reading::Name(pending));
                }
            }
        }
    }
}

/// The `(name, value)` pairs of the parse `events` of one section, as a [`SectionValues`] iterator.
fn section_values<'a, I>(events: I) -> SectionValues<I::IntoIter>
where
    I: IntoIterator<Item = EventRef<'a>>,
{
    SectionValues {
        events: events.into_iter(),
        reading: Some(Reading::Between),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fixtures::{open_path, open_untrusted};
    use gix::{
        config::Source,
        open::{Options, Permissions, permissions},
    };
    use nu_plugin_gix_testing_support::{
        FixtureError, FixtureRepo as _, TestRepo, append_config, test_repo,
    };
    use rstest::rstest;

    use std::{assert_matches, fs, io};

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

    const GIXTEST_CONFIG: &[u8] = b"[gixtest]\n\tanswer = 42\n";

    /// Appends `text` to the local configuration of `repo` and reads the configuration of the repository.
    fn read_config(repo: &TestRepo, text: &[u8]) -> Result<Config, TestError> {
        append_config(repo.path(), text)?;

        let opened = open_path(repo.path())?;

        Ok(Config::read(&opened)?)
    }

    mod section_values {
        use super::*;

        use gix::bstr::BStr;

        use std::{collections::VecDeque, iter};

        /// The items of [`section_values`] for `events`, collected until the first fault.
        fn values_of(
            events: Vec<EventRef<'_>>,
        ) -> Result<Vec<(VariableName, RawValue)>, SectionFault> {
            section_values(events).collect()
        }

        #[rstest]
        #[case::name_then_name(vec![
            EventRef::SectionValueName(BStr::new("a")),
            EventRef::SectionValueName(BStr::new("b")),
            EventRef::Value(BStr::new("1")),
        ])]
        #[case::name_at_end(vec![EventRef::SectionValueName(BStr::new("a"))])]
        #[case::name_then_separator_at_end(vec![
            EventRef::SectionValueName(BStr::new("a")),
            EventRef::KeyValueSeparator,
        ])]
        fn rejects_name_without_value(#[case] events: Vec<EventRef<'static>>) {
            assert_matches!(
                values_of(events),
                Err(SectionFault::NameWithoutValue(name)) if name == VariableName::from("a"),
            );
        }

        #[rstest]
        #[case::separator_without_name(vec![EventRef::KeyValueSeparator])]
        #[case::value_not_done_without_name(vec![EventRef::ValueNotDone(BStr::new("x"))])]
        #[case::value_without_name(vec![EventRef::Value(BStr::new("1"))])]
        #[case::value_done_without_name(vec![EventRef::ValueDone(BStr::new("1"))])]
        fn rejects_value_without_name(#[case] events: Vec<EventRef<'static>>) {
            assert_matches!(values_of(events), Err(SectionFault::ValueWithoutName),);
        }

        #[rstest]
        fn skips_comments_and_whitespace() {
            let events = vec![
                EventRef::Comment {
                    tag: b'#',
                    text: BStr::new(" note"),
                },
                EventRef::Whitespace(BStr::new(" ")),
                EventRef::Newline(BStr::new("\n")),
                EventRef::SectionHeader {
                    name: BStr::new("a"),
                    separator: None,
                    subsection_name: None,
                },
                EventRef::SectionValueName(BStr::new("b")),
                EventRef::Value(BStr::new("1")),
            ];

            assert_matches!(
                values_of(events).as_deref(),
                Ok([(name, RawValue::Valueless)]) if *name == VariableName::from("b"),
            );
        }

        #[rstest]
        fn stops_after_a_fault() {
            let mut values = section_values(vec![
                EventRef::SectionValueName(BStr::new("a")),
                EventRef::SectionValueName(BStr::new("b")),
                EventRef::SectionValueName(BStr::new("c")),
                EventRef::Value(BStr::new("1")),
            ]);

            assert_matches!(
                values.next(),
                Some(Err(SectionFault::NameWithoutValue(name))) if name == VariableName::from("a"),
            );
            assert_matches!(values.next(), None);
        }

        #[rstest]
        fn is_fused() {
            // `None` is the end of one batch of events; the source yields `b = 2` only on later calls
            let mut queue: VecDeque<Option<EventRef<'static>>> = VecDeque::from(vec![
                Some(EventRef::SectionValueName(BStr::new("a"))),
                Some(EventRef::KeyValueSeparator),
                Some(EventRef::Value(BStr::new("1"))),
                None,
                Some(EventRef::SectionValueName(BStr::new("b"))),
                Some(EventRef::KeyValueSeparator),
                Some(EventRef::Value(BStr::new("2"))),
            ]);
            let mut values = section_values(iter::from_fn(move || queue.pop_front().flatten()));

            let mut names = Vec::new();
            while let Some(Ok((name, _))) = values.next() {
                names.push(name);
            }

            assert_eq!(names, vec![VariableName::from("a")]);
            assert_matches!(values.next(), None);
            assert_matches!(values.next(), None);
        }

        #[rstest]
        fn passes_pairs_before_a_fault() {
            let mut values = section_values(vec![
                EventRef::SectionValueName(BStr::new("a")),
                EventRef::Value(BStr::new("1")),
                EventRef::SectionValueName(BStr::new("b")),
            ]);

            assert_matches!(
                values.next(),
                Some(Ok((name, RawValue::Valueless))) if name == VariableName::from("a"),
            );
            assert_matches!(
                values.next(),
                Some(Err(SectionFault::NameWithoutValue(name))) if name == VariableName::from("b"),
            );
            assert_matches!(values.next(), None);
        }
    }

    mod from_occurrences {
        use super::*;

        fn occurrence(text: &[u8]) -> Occurrence {
            Occurrence {
                value: RawValue::Bytes(text.to_vec()),
                origin: Arc::new(Origin {
                    path: None,
                    source: Source::Api,
                }),
                trusted: true,
            }
        }

        #[rstest]
        fn groups_occurrences_under_their_key_in_order() {
            let key = ConfigKey::for_test("c.k");

            let config = Config::from_occurrences([
                (key.clone(), occurrence(b"first")),
                (key.clone(), occurrence(b"second")),
            ]);

            assert_eq!(
                config.occurrences(&key),
                [occurrence(b"first"), occurrence(b"second")].as_slice()
            );
        }

        #[rstest]
        fn iterates_keys_in_key_order_whatever_the_input_order() {
            let first = ConfigKey::for_test("a.z");
            let second = ConfigKey::for_test("b.a");

            let config = Config::from_occurrences([
                (second.clone(), occurrence(b"2")),
                (first.clone(), occurrence(b"1")),
            ]);

            assert_eq!(
                config.iter().map(|(key, _)| key).collect::<Vec<_>>(),
                vec![&first, &second]
            );
        }
    }

    mod read {
        use super::*;

        mod subsections {
            use super::*;

            #[rstest]
            fn exact_header_keeps_its_case(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;

                let config = read_config(&repo, b"[a \"Sub\"]\n\tx = 1\n[a.Sub]\n\ty = 2\n")?;

                assert_eq!(
                    config
                        .occurrences(&ConfigKey::for_test("a.Sub.x"))
                        .iter()
                        .map(|occurrence| occurrence.value.clone())
                        .collect::<Vec<_>>(),
                    vec![RawValue::Bytes(b"1".to_vec())]
                );
                Ok(())
            }

            #[rstest]
            fn legacy_header_is_lowercased(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;

                let config = read_config(&repo, b"[a \"Sub\"]\n\tx = 1\n[a.Sub]\n\ty = 2\n")?;

                assert_eq!(
                    config
                        .occurrences(&ConfigKey::for_test("a.sub.y"))
                        .iter()
                        .map(|occurrence| occurrence.value.clone())
                        .collect::<Vec<_>>(),
                    vec![RawValue::Bytes(b"2".to_vec())]
                );
                assert_eq!(
                    config.occurrences(&ConfigKey::for_test("a.Sub.y")),
                    Vec::<Occurrence>::new().as_slice()
                );
                Ok(())
            }

            #[rstest]
            fn empty_subsection_is_a_key_of_its_own(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;

                let config = read_config(&repo, b"[a \"\"]\n\tb = 1\n")?;

                assert_eq!(config.occurrences(&ConfigKey::for_test("a..b")).len(), 1);
                assert_eq!(
                    config.occurrences(&ConfigKey::for_test("a.b")),
                    Vec::<Occurrence>::new().as_slice()
                );
                Ok(())
            }

            #[rstest]
            fn reads_past_a_non_utf8_subsection(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;

                let config = read_config(
                    &repo,
                    b"[gixtest]\n\tanswer = 42\n[remote \"\xff\"]\n\turl = x\n",
                )?;

                assert_eq!(
                    config
                        .occurrences(&ConfigKey::for_test("gixtest.answer"))
                        .iter()
                        .map(|occurrence| occurrence.value.clone())
                        .collect::<Vec<_>>(),
                    vec![RawValue::Bytes(b"42".to_vec())]
                );
                let remote_keys: Vec<&ConfigKey> = config
                    .iter()
                    .map(|(key, _)| key)
                    .filter(|key| *key.section() == SectionName::from("remote"))
                    .collect();
                let [key] = remote_keys.as_slice() else {
                    return Err(TestError::Failure(format!("got {remote_keys:?}")));
                };
                assert_matches!(key.subsection().map(Subsection::to_text), Some(Err(_)),);
                Ok(())
            }
        }

        mod values {
            use super::*;

            #[rstest]
            #[case::valueless("b.flag", RawValue::Valueless)]
            #[case::empty_value("b.empty", RawValue::Bytes(Vec::new()))]
            #[case::continued("b.multi", RawValue::Bytes(b"a  b".to_vec()))]
            #[case::continued_tab("b.tab", RawValue::Bytes(b"a \tb".to_vec()))]
            #[case::continued_mixed("b.lead", RawValue::Bytes(b"a    \t b".to_vec()))]
            #[case::continued_trailing("b.trail", RawValue::Bytes(b"a  b".to_vec()))]
            #[case::quoted("b.quoted", RawValue::Bytes(b"x y".to_vec()))]
            fn reads_the_raw_value(
                test_repo: Result<TestRepo, FixtureError>,
                #[case] text: &str,
                #[case] expected: RawValue,
            ) -> Result<(), TestError> {
                let repo = test_repo?;

                let config = read_config(
                    &repo,
                    b"[b]\n\tflag\n\tempty =\n\tmulti = a \\\n b\n\tquoted = \"x y\"\n\ttab = a \\\n\tb\n\tlead = a \\\n   \t b\n\ttrail = a \\\n b  \n",
                )?;

                let occurrences = config.occurrences(&ConfigKey::for_test(text));
                assert_matches!(
                    occurrences,
                    [occurrence] if occurrence.value == expected,
                    "got {occurrences:?}"
                );
                Ok(())
            }

            #[rstest]
            fn repeated_key_keeps_load_order(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;

                let config = read_config(&repo, b"[c]\n\tk = first\n[c]\n\tk = second\n")?;

                assert_eq!(
                    config
                        .occurrences(&ConfigKey::for_test("c.k"))
                        .iter()
                        .map(|occurrence| occurrence.value.clone())
                        .collect::<Vec<_>>(),
                    vec![
                        RawValue::Bytes(b"first".to_vec()),
                        RawValue::Bytes(b"second".to_vec()),
                    ]
                );
                Ok(())
            }
        }

        mod origins {
            use super::*;

            #[rstest]
            fn local_occurrence_names_the_repository_config(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;

                let config = read_config(&repo, GIXTEST_CONFIG)?;
                let expected = repo.path().join(".git").join("config");

                assert_matches!(
                    config.occurrences(&ConfigKey::for_test("gixtest.answer")),
                    [Occurrence { origin, .. }]
                        if origin.path.as_deref() == Some(expected.as_path()),
                );
                Ok(())
            }

            #[rstest]
            fn occurrences_of_one_section_share_one_origin(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;

                let config = read_config(&repo, b"[gixtest]\n\tfirst = 1\n\tsecond = 2\n")?;
                let first = config.occurrences(&ConfigKey::for_test("gixtest.first"));
                let second = config.occurrences(&ConfigKey::for_test("gixtest.second"));
                let first_origin = &first
                    .first()
                    .ok_or_else(|| TestError::Failure(format!("got {first:?}")))?
                    .origin;
                let second_origin = &second
                    .first()
                    .ok_or_else(|| TestError::Failure(format!("got {second:?}")))?
                    .origin;

                assert!(Arc::ptr_eq(first_origin, second_origin));
                Ok(())
            }

            #[rstest]
            fn included_occurrence_names_the_included_file(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;

                append_config(repo.path(), b"[include]\n\tpath = extra.cfg\n")?;
                fs::write(
                    repo.path().join(".git").join("extra.cfg"),
                    b"[gixtest]\n\tinc = 9\n",
                )?;

                // Includes are off in the isolated options that `open_path` uses.
                let open_permissions = Permissions {
                    config: permissions::Config {
                        includes: true,
                        ..permissions::Config::isolated()
                    },
                    ..Permissions::isolated()
                };
                let opened = OpenRepository::from_repository(gix::open_opts(
                    repo.path(),
                    Options::isolated().permissions(open_permissions),
                )?);

                let config = Config::read(&opened)?;

                assert_matches!(
                    config.occurrences(&ConfigKey::for_test("gixtest.inc")),
                    [Occurrence { origin, .. }]
                        if origin.path.as_deref().is_some_and(|path| path.ends_with("extra.cfg")),
                );
                Ok(())
            }

            #[rstest]
            fn untrusted_local_occurrence_is_not_trusted(
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let repo = test_repo?;

                append_config(repo.path(), GIXTEST_CONFIG)?;

                let opened = open_untrusted(repo.path())?;
                let config = Config::read(&opened)?;

                assert_matches!(
                    config.occurrences(&ConfigKey::for_test("gixtest.answer")),
                    [Occurrence { trusted: false, .. }],
                );
                Ok(())
            }
        }
    }

    mod lookup {
        use super::*;

        #[rstest]
        fn names_ignore_case(test_repo: Result<TestRepo, FixtureError>) -> Result<(), TestError> {
            let repo = test_repo?;

            let config = read_config(&repo, b"[A]\n\tNaMe = 1\n")?;

            assert_matches!(
                config.occurrences(&ConfigKey::for_test("a.name")),
                [Occurrence { value: RawValue::Bytes(value), .. }] if value == b"1",
            );
            Ok(())
        }

        #[rstest]
        fn unset_key_has_no_occurrences(
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = test_repo?;

            let config = read_config(&repo, GIXTEST_CONFIG)?;

            assert_eq!(
                config.occurrences(&ConfigKey::for_test("gixtest.missing")),
                Vec::<Occurrence>::new().as_slice()
            );
            Ok(())
        }
    }
}
