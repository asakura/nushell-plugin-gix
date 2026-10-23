//! The `<key>` argument of `gix config`: a cell path or a string, turned into the split parts core looks up.

use super::CommandError;

use nu_plugin_gix_core as git;
use nu_protocol::ast::PathMember;
use nu_protocol::{Spanned, Value};

/// The separator of a key string.
const KEY_SEPARATOR: &str = ".";

/// The key argument: the split parts core looks up, the key as written, and whether a `?` makes a miss null.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct KeyArg {
    /// The split parts core looks up.
    pub path: git::ConfigPath,
    /// The key as written: a string, or the members of a cell path joined by `.`; used in messages.
    pub text: String,
    /// Whether some member of a cell path ends in `?`, so a key that names nothing is null, not an error.
    pub optional: bool,
}

/// Reads the key argument: a string is split as text, a cell path gives its members.
///
/// # Errors
///
/// Returns [`CommandError::Located`], at the key, for a key that is malformed, and
/// [`CommandError::InvalidArgument`] for a value that is neither a string nor a cell path.
pub(super) fn key_arg(key: &Spanned<Value>) -> Result<KeyArg, CommandError> {
    let located = |source| CommandError::Located {
        span: key.span,
        source,
    };

    if let Ok(text) = key.item.as_str() {
        return path_from_text(text)
            .map(|path| KeyArg {
                path,
                text: text.to_owned(),
                optional: false,
            })
            .map_err(located);
    }

    match key.item.as_cell_path() {
        Ok(cell_path) => {
            let members: Vec<String> = cell_path.members.iter().map(member_text).collect();
            let optional = cell_path.members.iter().any(is_optional);

            path_from_members(&members)
                .map(|path| KeyArg {
                    path,
                    text: members.join(KEY_SEPARATOR),
                    optional,
                })
                .map_err(located)
        }
        Err(source) => Err(CommandError::InvalidArgument {
            source: Box::new(source),
            span: key.span,
        }),
    }
}

/// The text of a cell path member: a number is its decimal text; `?` and `!` markers are ignored.
fn member_text(member: &PathMember) -> String {
    match member {
        PathMember::String {
            val,
            span: _span,
            optional: _optional,
            casing: _casing,
        } => val.clone(),
        PathMember::Int {
            val,
            span: _span,
            optional: _optional,
        } => val.to_string(),
    }
}

/// Whether a cell path member ends in `?`.
const fn is_optional(member: &PathMember) -> bool {
    match member {
        PathMember::String {
            optional,
            val: _val,
            span: _span,
            casing: _casing,
        } => *optional,
        PathMember::Int {
            optional,
            val: _val,
            span: _span,
        } => *optional,
    }
}

/// Splits a key string at its first and last dot: no dot is a section, one dot a section and a name, more a section,
/// a subsection (which may hold dots) and a name.
fn path_from_text(text: &str) -> Result<git::ConfigPath, git::GixError> {
    let Some((section, rest)) = text.split_once(KEY_SEPARATOR) else {
        return git::ConfigPath::section(text);
    };

    match rest.rsplit_once(KEY_SEPARATOR) {
        None => git::ConfigPath::member(section, rest),
        Some((subsection, name)) => git::ConfigPath::variable(section, subsection, name),
    }
}

/// A `ConfigKey` from text, for tests of this crate: `path_from_text` splits it. A key with no `.` is an error.
#[cfg(test)]
pub(super) fn key_for_test(text: &str) -> Result<git::ConfigKey, git::GixError> {
    match path_from_text(text)? {
        git::ConfigPath::Section(_) => Err(git::GixError::InvalidConfigKey {
            key: text.to_owned(),
        }),
        git::ConfigPath::Member { section, name } => Ok(git::ConfigKey::new(
            section,
            None,
            git::VariableName::from(&*name),
        )),
        git::ConfigPath::Variable {
            section,
            subsection,
            name,
        } => Ok(git::ConfigKey::new(section, Some(subsection), name)),
    }
}

/// Maps the members of a cell path: one is a section, two a section and a name, three a section, a subsection and
/// a name.
fn path_from_members(members: &[String]) -> Result<git::ConfigPath, git::GixError> {
    match members {
        [section] => git::ConfigPath::section(section),
        [section, name] => git::ConfigPath::member(section, name),
        [section, subsection, name] => git::ConfigPath::variable(section, subsection, name),
        _ => Err(git::GixError::InvalidConfigKey {
            key: members.join(KEY_SEPARATOR),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use nu_protocol::Span;
    use nu_protocol::ast::CellPath;
    use nu_protocol::casing::Casing;
    use rstest::rstest;

    use std::assert_matches;

    #[derive(thiserror::Error, Debug)]
    enum TestError {
        #[error(transparent)]
        Gix(#[from] git::GixError),

        #[error(transparent)]
        Command(#[from] Box<CommandError>),
    }

    impl From<CommandError> for TestError {
        fn from(error: CommandError) -> Self {
            Self::Command(Box::new(error))
        }
    }

    fn members(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_owned()).collect()
    }

    mod path_from_text {
        use super::*;

        #[rstest]
        #[case::no_dot_is_a_section("core", git::ConfigPath::section("core"))]
        #[case::one_dot("user.name", git::ConfigPath::member("user", "name"))]
        #[case::two_dots(
            "remote.origin.url",
            git::ConfigPath::variable("remote", "origin", "url")
        )]
        #[case::subsection_with_dots(
            "url.https://a.b/.insteadof",
            git::ConfigPath::variable("url", "https://a.b/", "insteadof")
        )]
        #[case::empty_subsection("a..b", git::ConfigPath::variable("a", "", "b"))]
        fn splits_the_text(
            #[case] text: &str,
            #[case] expected: Result<git::ConfigPath, git::GixError>,
        ) -> Result<(), TestError> {
            assert_eq!(path_from_text(text)?, expected?);

            Ok(())
        }

        #[rstest]
        #[case::trailing_dot_is_invalid("remote.")]
        #[case::leading_dot_is_invalid(".x")]
        #[case::empty_is_invalid("")]
        fn rejects_a_malformed_key(#[case] text: &str) {
            assert_matches!(
                path_from_text(text),
                Err(git::GixError::InvalidConfigKey { .. })
            );
        }
    }

    mod path_from_members {
        use super::*;

        #[rstest]
        #[case::one_member(&["core"], git::ConfigPath::section("core"))]
        #[case::two_members(&["user", "name"], git::ConfigPath::member("user", "name"))]
        #[case::three_members(&["remote", "origin", "url"], git::ConfigPath::variable("remote", "origin", "url"))]
        #[case::member_with_a_dot_is_a_subsection_name(
            &["remote", "a.b"],
            git::ConfigPath::member("remote", "a.b")
        )]
        fn maps_the_members(
            #[case] parts: &[&str],
            #[case] expected: Result<git::ConfigPath, git::GixError>,
        ) -> Result<(), TestError> {
            assert_eq!(path_from_members(&members(parts))?, expected?);

            Ok(())
        }

        #[rstest]
        fn four_members_is_invalid() {
            assert_matches!(
                path_from_members(&members(&["a", "b", "c", "d"])),
                Err(git::GixError::InvalidConfigKey { key }) if key == "a.b.c.d"
            );
        }

        #[rstest]
        fn zero_members_is_invalid() {
            assert_matches!(
                path_from_members(&[]),
                Err(git::GixError::InvalidConfigKey { .. })
            );
        }
    }

    mod key_arg {
        use super::*;

        /// A cell path key whose members are `parts`: `(text, optional, casing)` each.
        fn cell_key(parts: &[(&str, bool, Casing)]) -> Spanned<Value> {
            let span = Span::test_data();
            let members = parts
                .iter()
                .map(|(text, optional, casing)| {
                    PathMember::string((*text).to_owned(), *optional, *casing, span)
                })
                .collect();

            Spanned {
                item: Value::cell_path(CellPath { members }, span),
                span,
            }
        }

        #[rstest]
        fn int_member_is_its_text() -> Result<(), TestError> {
            let span = Span::test_data();
            let path = CellPath {
                members: vec![
                    PathMember::string("remote".to_owned(), false, Casing::Sensitive, span),
                    PathMember::int(0, false, span),
                    PathMember::string("url".to_owned(), false, Casing::Sensitive, span),
                ],
            };
            let key = Spanned {
                item: Value::cell_path(path, span),
                span,
            };

            assert_eq!(
                key_arg(&key)?.path,
                git::ConfigPath::variable("remote", "0", "url")?
            );

            Ok(())
        }

        #[rstest]
        fn string_value_is_split_as_text() -> Result<(), TestError> {
            let span = Span::test_data();
            let key = Spanned {
                item: Value::string("url.https://a.b/.insteadof", span),
                span,
            };

            assert_eq!(
                key_arg(&key)?.path,
                git::ConfigPath::variable("url", "https://a.b/", "insteadof")?
            );

            Ok(())
        }

        #[rstest]
        #[case::question_on_the_section(&[("a", true, Casing::Sensitive), ("b", false, Casing::Sensitive)])]
        #[case::question_on_the_name(&[("a", false, Casing::Sensitive), ("b", true, Casing::Sensitive)])]
        fn a_question_mark_makes_the_key_optional(
            #[case] parts: &[(&str, bool, Casing)],
        ) -> Result<(), TestError> {
            assert!(key_arg(&cell_key(parts))?.optional);

            Ok(())
        }

        #[rstest]
        #[case::plain(&[("a", false, Casing::Sensitive), ("b", false, Casing::Sensitive)])]
        #[case::case_insensitive_marker(&[("a", false, Casing::Insensitive), ("b", false, Casing::Insensitive)])]
        fn plain_and_case_insensitive_keys_are_not_optional(
            #[case] parts: &[(&str, bool, Casing)],
        ) -> Result<(), TestError> {
            assert!(!key_arg(&cell_key(parts))?.optional);

            Ok(())
        }

        #[rstest]
        fn cell_path_text_joins_the_members() -> Result<(), TestError> {
            let key = cell_key(&[
                ("user", false, Casing::Sensitive),
                ("name", false, Casing::Sensitive),
            ]);

            assert_eq!(key_arg(&key)?.text, "user.name");

            Ok(())
        }

        #[rstest]
        fn string_text_is_the_string_and_not_optional() -> Result<(), TestError> {
            let span = Span::test_data();
            let key = Spanned {
                item: Value::string("user.name", span),
                span,
            };
            let arg = key_arg(&key)?;

            assert_eq!(arg.text, "user.name");
            assert!(!arg.optional);

            Ok(())
        }

        #[rstest]
        fn non_key_value_is_invalid_argument() {
            let span = Span::test_data();
            let key = Spanned {
                item: Value::int(1, span),
                span,
            };

            assert_matches!(key_arg(&key), Err(CommandError::InvalidArgument { .. }));
        }
    }
}
