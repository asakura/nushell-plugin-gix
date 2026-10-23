//! Tests of the nested configuration tree: construction, the checked path constructors and lookup.

use super::*;

use crate::config::{
    ConfigKey, Occurrence, Origin, RawValue, SectionName, Subsection, VariableName,
};

use gix::config::Source;
use rstest::rstest;

use std::{assert_matches, sync::Arc};

#[derive(Debug, thiserror::Error)]
enum TestError {
    #[error(transparent)]
    Gix(#[from] GixError),

    #[error("Test failure: {0}")]
    Failure(String),
}

type Row<'a> = (&'a str, Option<&'a str>, &'a str, &'a str);

fn occurrence(value: RawValue) -> Occurrence {
    Occurrence {
        value,
        origin: Arc::new(Origin {
            path: None,
            source: Source::Local,
        }),
        trusted: true,
    }
}

fn key(section: &str, subsection: Option<&str>, name: &str) -> ConfigKey {
    ConfigKey::new(
        SectionName::from(section),
        subsection.map(Subsection::from),
        VariableName::from(name),
    )
}

fn config(rows: &[Row<'_>]) -> Config {
    Config::from_occurrences(rows.iter().map(|(section, subsection, name, value)| {
        (
            key(section, *subsection, name),
            occurrence(RawValue::Bytes(value.as_bytes().to_vec())),
        )
    }))
}

mod path {
    use super::*;

    #[rstest]
    #[case::empty_section(ConfigPath::section(""), "")]
    #[case::dot_in_section(ConfigPath::section("a.b"), "a.b")]
    #[case::member_empty_name(ConfigPath::member("a", ""), "a.")]
    #[case::member_empty_section(ConfigPath::member("", "x"), ".x")]
    #[case::variable_empty_name(ConfigPath::variable("a", "s", ""), "a.s.")]
    #[case::variable_dot_in_name(ConfigPath::variable("a", "s", "x.y"), "a.s.x.y")]
    fn rejects_invalid_parts(#[case] path: Result<ConfigPath, GixError>, #[case] expected: &str) {
        assert_matches!(path, Err(GixError::InvalidConfigKey { key }) if key == expected);
    }

    #[rstest]
    #[case::section(ConfigPath::section("core"))]
    #[case::member_with_dot(ConfigPath::member("remote", "a.b"))]
    #[case::variable_with_empty_subsection(ConfigPath::variable("a", "", "b"))]
    fn accepts_valid_parts(#[case] path: Result<ConfigPath, GixError>) {
        assert_matches!(path, Ok(_));
    }
}

mod tree {
    use super::*;

    fn section_of<'a>(
        tree: &'a ConfigTree<'a>,
        name: &str,
    ) -> Result<&'a SectionNode<'a>, TestError> {
        match tree.get(&ConfigPath::section(name)?) {
            Some(Node::Section(section)) => Ok(section),
            other => Err(TestError::Failure(format!("no section {name}: {other:?}"))),
        }
    }

    #[rstest]
    fn groups_by_section_and_subsection() -> Result<(), TestError> {
        let config = config(&[
            ("core", None, "bare", "false"),
            ("core", None, "editor", "nano"),
            ("remote", Some("origin"), "url", "u"),
        ]);
        let tree = config.tree();

        let names: Vec<&str> = tree.sections().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, vec!["core", "remote"]);

        let core = section_of(&tree, "core")?;
        let entries: Vec<&Entry<'_>> = core.entries().collect();
        assert_eq!(entries.len(), 2);
        assert_matches!(entries.as_slice(), [Entry::Setting(_), Entry::Setting(_)]);

        let remote = section_of(&tree, "remote")?;
        let remote_entries: Vec<&Entry<'_>> = remote.entries().collect();
        assert_matches!(
            remote_entries.as_slice(),
            [Entry::Subsection(sub)]
                if sub.subsection().as_bytes() == b"origin"
                    && sub.settings().map(|s| s.key().name().as_str()).collect::<Vec<_>>() == vec!["url"]
        );

        Ok(())
    }

    #[rstest]
    fn keeps_every_occurrence_in_load_order() {
        let config = config(&[("user", None, "name", "Ada"), ("user", None, "name", "Bob")]);
        let tree = config.tree();

        let settings = tree.settings();
        assert_matches!(settings.as_slice(), [setting] if matches!(
            setting.occurrences(),
            [first, second]
                if first.value == RawValue::Bytes(b"Ada".to_vec())
                    && second.value == RawValue::Bytes(b"Bob".to_vec())
        ));
    }

    #[rstest]
    fn subsection_hides_variable_of_its_name() -> Result<(), TestError> {
        let config = config(&[("a", None, "x", "1"), ("a", Some("x"), "y", "2")]);
        let tree = config.tree();

        let a = section_of(&tree, "a")?;
        let entries: Vec<&Entry<'_>> = a.entries().collect();
        assert_matches!(
            entries.as_slice(),
            [Entry::Subsection(sub)]
                if sub.subsection().as_bytes() == b"x"
                    && sub.settings().map(|s| s.key().name().as_str()).collect::<Vec<_>>() == vec!["y"]
        );

        let keys: Vec<String> = tree
            .settings()
            .iter()
            .map(|setting| setting.key().to_string())
            .collect();
        assert_eq!(keys, vec!["a.x".to_owned(), "a.x.y".to_owned()]);

        Ok(())
    }

    #[rstest]
    fn settings_are_sorted_by_key_and_complete() {
        let config = config(&[
            ("remote", Some("origin"), "url", "u"),
            ("a", Some("x"), "y", "2"),
            ("core", None, "bare", "false"),
            ("a", None, "x", "1"),
        ]);
        let tree = config.tree();

        let keys: Vec<ConfigKey> = tree
            .settings()
            .iter()
            .map(|setting| setting.key().clone())
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();

        assert_eq!(keys, sorted);
        assert_eq!(keys.len(), 4);
    }

    #[rstest]
    fn keeps_reset_marker_and_valueless_raw() {
        let key = key("credential", None, "helper");
        let config = Config::from_occurrences([
            (key.clone(), occurrence(RawValue::Bytes(b"a".to_vec()))),
            (key.clone(), occurrence(RawValue::Bytes(Vec::new()))),
            (key, occurrence(RawValue::Valueless)),
        ]);
        let tree = config.tree();

        let settings = tree.settings();
        assert_matches!(settings.as_slice(), [setting] if matches!(
            setting.occurrences(),
            [first, second, third]
                if first.value == RawValue::Bytes(b"a".to_vec())
                    && second.value == RawValue::Bytes(Vec::new())
                    && third.value == RawValue::Valueless
        ));
    }

    #[rstest]
    fn subsection_with_other_case_does_not_hide_variable() -> Result<(), TestError> {
        let config = config(&[("a", None, "x", "1"), ("a", Some("X"), "y", "2")]);
        let tree = config.tree();

        let a = section_of(&tree, "a")?;
        let entries: Vec<&Entry<'_>> = a.entries().collect();
        assert_matches!(
            entries.as_slice(),
            [Entry::Subsection(sub), Entry::Setting(setting)]
                if sub.subsection().as_bytes() == b"X" && setting.key().name().as_str() == "x"
        );

        Ok(())
    }

    #[rstest]
    fn entries_ascend_by_name_bytes() -> Result<(), TestError> {
        let config = config(&[
            ("a", None, "zeta", "1"),
            ("a", Some("Mid"), "y", "2"),
            ("a", None, "alpha", "3"),
        ]);
        let tree = config.tree();

        let a = section_of(&tree, "a")?;
        let entries: Vec<&Entry<'_>> = a.entries().collect();
        assert_matches!(
            entries.as_slice(),
            [Entry::Subsection(sub), Entry::Setting(first), Entry::Setting(last)]
                if sub.subsection().as_bytes() == b"Mid"
                    && first.key().name().as_str() == "alpha"
                    && last.key().name().as_str() == "zeta"
        );

        Ok(())
    }
}

mod get {
    use super::*;

    fn sample() -> Config {
        config(&[
            ("core", None, "bare", "false"),
            ("remote", Some("Origin"), "url", "u1"),
            ("remote", Some("a.b"), "url", "u2"),
            ("a", None, "x", "1"),
            ("a", Some("x"), "y", "2"),
            ("a", Some(""), "b", "3"),
        ])
    }

    #[rstest]
    fn section_ignores_case() -> Result<(), TestError> {
        let config = sample();
        let tree = config.tree();

        assert_matches!(
            tree.get(&ConfigPath::section("CORE")?),
            Some(Node::Section(_))
        );

        Ok(())
    }

    #[rstest]
    fn member_variable_ignores_case() -> Result<(), TestError> {
        let config = sample();
        let tree = config.tree();

        assert_matches!(
            tree.get(&ConfigPath::member("Core", "BARE")?),
            Some(Node::Setting(setting)) if setting.key().name().as_str() == "bare"
        );

        Ok(())
    }

    #[rstest]
    fn member_variable_folds_a_mixed_case_name() -> Result<(), TestError> {
        let config = sample();
        let tree = config.tree();

        assert_matches!(
            tree.get(&ConfigPath::member("Core", "BaRe")?),
            Some(Node::Setting(setting)) if setting.key().name().as_str() == "bare"
        );

        Ok(())
    }

    #[rstest]
    #[case::exact("Origin", true)]
    #[case::lowercase("origin", false)]
    #[case::uppercase("ORIGIN", false)]
    fn member_subsection_is_exact(
        #[case] name: &str,
        #[case] found: bool,
    ) -> Result<(), TestError> {
        let config = sample();
        let tree = config.tree();

        let node = tree.get(&ConfigPath::member("remote", name)?);
        if found {
            assert_matches!(node, Some(Node::Subsection(_)));
        } else {
            assert_matches!(node, None);
        }

        Ok(())
    }

    #[rstest]
    fn member_with_dot_names_a_subsection() -> Result<(), TestError> {
        let config = sample();
        let tree = config.tree();

        assert_matches!(
            tree.get(&ConfigPath::member("remote", "a.b")?),
            Some(Node::Subsection(_))
        );

        Ok(())
    }

    #[rstest]
    fn member_prefers_the_subsection_over_a_hidden_variable() -> Result<(), TestError> {
        let config = sample();
        let tree = config.tree();

        assert_matches!(
            tree.get(&ConfigPath::member("a", "x")?),
            Some(Node::Subsection(_))
        );

        Ok(())
    }

    #[rstest]
    #[case::folded_names("REMOTE", "Origin", "URL", true)]
    #[case::subsection_other_case("remote", "origin", "url", false)]
    fn variable_subsection_exact_name_folded(
        #[case] section: &str,
        #[case] subsection: &str,
        #[case] name: &str,
        #[case] found: bool,
    ) -> Result<(), TestError> {
        let config = sample();
        let tree = config.tree();

        let node = tree.get(&ConfigPath::variable(section, subsection, name)?);
        if found {
            assert_matches!(node, Some(Node::Setting(_)));
        } else {
            assert_matches!(node, None);
        }

        Ok(())
    }

    #[rstest]
    fn variable_with_empty_subsection() -> Result<(), TestError> {
        let config = sample();
        let tree = config.tree();

        assert_matches!(
            tree.get(&ConfigPath::variable("a", "", "b")?),
            Some(Node::Setting(_))
        );

        Ok(())
    }

    #[rstest]
    fn missing_section_is_none() -> Result<(), TestError> {
        let config = sample();
        let tree = config.tree();

        assert_matches!(tree.get(&ConfigPath::section("nope")?), None);

        Ok(())
    }

    #[rstest]
    fn missing_name_is_none() -> Result<(), TestError> {
        let config = sample();
        let tree = config.tree();

        assert_matches!(tree.get(&ConfigPath::member("core", "nope")?), None);

        Ok(())
    }
}
