//! # Sides
//!
//! A side names where a Git operation reads content from, or writes content to.
//!
//! | Side | Meaning | Mutable | Live |
//! | --- | --- | --- | --- |
//! | `head` | The commit currently selected by `HEAD`. | No | No |
//! | `index` | The staged snapshot, or a scratch index. | Yes | Yes |
//! | `worktree` | Files on disk in the working tree. | Yes | Yes |
//! | `<rev>` | A Git revision expression: branch, tag, object ID, or revision range. | No | No |
//!
//! `HEAD` is accepted as an alias for `head`. Revisions are retained exactly as written while
//! parsing, then resolved by the core layer against a repository. Blank or whitespace-only
//! revisions are rejected. A string as long as a full object id (40 characters for SHA-1, 64
//! for SHA-256) must be a valid hexadecimal digest; shorter spellings, including abbreviated
//! object ids, are accepted as written and validated by resolution.
//!
//! Sides are independent of path selection. Commands use `--paths` to limit their scope; a side
//! never implies a path filter. Git's normal ignore rules apply when a command reads the working
//! tree, while explicit paths remain available for operations that need ignored files.

use crate::commands::CommandError;
use crate::commands::{is_full_oid, is_hexadecimal};

use nu_plugin_gix_core::{OpenRepository, Side};
use nu_protocol::{Span, Value, record};

const HEAD: &str = "head";
const HEAD_UPPER: &str = "HEAD";
const INDEX: &str = "index";
const REV_FIELD: &str = "rev";
const WORKTREE: &str = "worktree";

/// A syntactically valid side before the core layer resolves its revision expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SideArg {
    /// The commit selected by `HEAD`.
    Head,
    /// The staged snapshot.
    Index,
    /// Files in the working tree.
    Worktree,
    /// A non-empty revision expression awaiting repository resolution.
    Rev(String),
}

impl SideArg {
    /// Resolves this side in `opened`: symbolic sides map directly, a revision is resolved by git.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::Located`] at `span` when the revision does not resolve.
    #[must_use = "the resolved side is needed to read or write the operand"]
    pub fn resolve(&self, opened: &OpenRepository, span: Span) -> Result<Side, CommandError> {
        let side = match self {
            Self::Head => Side::Head,
            Self::Index => Side::Index,
            Self::Worktree => Side::Worktree,
            Self::Rev(expression) => Side::resolve_revision(opened, expression)
                .map_err(|source| CommandError::Located { span, source })?,
        };

        Ok(side)
    }
}

impl TryFrom<&Value> for SideArg {
    type Error = CommandError;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        match value {
            Value::String {
                val, internal_span, ..
            } => parse_string(val, *internal_span),
            Value::Record {
                val, internal_span, ..
            } => parse_record(val, *internal_span),
            _ => Err(CommandError::InvalidSideType {
                got: value.get_type().to_string(),
                span: value.span(),
            }),
        }
    }
}

impl TryFrom<Value> for SideArg {
    type Error = CommandError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        Self::try_from(&value)
    }
}

fn parse_string(value: &str, span: nu_protocol::Span) -> Result<SideArg, CommandError> {
    if value.trim().is_empty() {
        return Err(CommandError::EmptyRevision { span });
    }

    let side = match value {
        HEAD | HEAD_UPPER => SideArg::Head,
        INDEX => SideArg::Index,
        WORKTREE => SideArg::Worktree,
        revision => parse_revision(revision, span)?,
    };

    Ok(side)
}

fn parse_record(value: &record::Record, span: nu_protocol::Span) -> Result<SideArg, CommandError> {
    if value.len() != 1 {
        return Err(CommandError::InvalidSideRecord { span });
    }

    let Some(revision) = value.get(REV_FIELD) else {
        return Err(CommandError::InvalidSideRecord { span });
    };

    match revision {
        Value::String {
            val, internal_span, ..
        } => parse_revision(val, *internal_span),
        _ => Err(CommandError::InvalidSideField {
            field: REV_FIELD,
            got: revision.get_type().to_string(),
            span: revision.span(),
        }),
    }
}

fn parse_revision(value: &str, span: nu_protocol::Span) -> Result<SideArg, CommandError> {
    if value.trim().is_empty() {
        return Err(CommandError::EmptyRevision { span });
    }

    if is_full_oid(value) && !value.bytes().all(is_hexadecimal) {
        return Err(CommandError::InvalidObjectId {
            expression: value.to_owned(),
            span,
        });
    }

    Ok(SideArg::Rev(value.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    use nu_plugin_gix_core::{GixError, OpenRepository, RepositoryLocation};
    use nu_plugin_gix_testing_support::{FixtureError, MAIN_BRANCH, TestRepo, test_repo};
    use nu_protocol::{Span, Value, record};
    use rstest::{fixture, rstest};

    use std::assert_matches;

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Fixture(#[from] FixtureError),

        #[error(transparent)]
        Git(#[from] GixError),

        #[error("Test failure: {0}")]
        Failure(String),
    }

    fn span(start: usize) -> Span {
        Span::new(start, start.saturating_add(1))
    }

    /// A full valid SHA-1 object id.
    const SHA1_HEX: &str = "0123456789abcdef0123456789abcdef01234567";

    /// A full valid SHA-256 object id.
    const SHA256_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// A valid SHA-1 abbreviation shorter than the full digest.
    const SHA1_ABBREVIATION: &str = "0123456";

    /// A valid SHA-256 abbreviation shorter than the full digest.
    const SHA256_ABBREVIATION: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde";

    /// A 40-character string with non-hexadecimal characters.
    const SHA1_HEX_INVALID: &str = "0123456789ABCDEF0123456789ABCDEF0123456z";

    /// A 64-character string with non-hexadecimal characters.
    const SHA256_HEX_INVALID: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdeg";

    mod try_from {
        use super::*;

        #[rstest]
        fn converts_a_borrowed_value_through_try_from() {
            let value = Value::string("head", span(1));
            let result = SideArg::try_from(&value);

            assert_matches!(result, Ok(SideArg::Head));
        }

        #[rstest]
        fn converts_an_owned_value_through_try_from() {
            let result = SideArg::try_from(Value::string("index", span(1)));

            assert_matches!(result, Ok(SideArg::Index));
        }

        #[rstest]
        #[case::head("head", SideArg::Head)]
        #[case::upper_head("HEAD", SideArg::Head)]
        #[case::index("index", SideArg::Index)]
        #[case::worktree("worktree", SideArg::Worktree)]
        #[case::revision("main~1", SideArg::Rev("main~1".to_owned()))]
        #[case::preserves_revision_whitespace(" main ", SideArg::Rev(" main ".to_owned()))]
        fn parses_strings(#[case] input: &str, #[case] expected: SideArg) {
            let found = SideArg::try_from(&Value::string(input, span(1)));

            assert_matches!(found, Ok(actual) if actual == expected);
        }

        #[rstest]
        #[case::sha1(SHA1_HEX)]
        #[case::sha256(SHA256_HEX)]
        #[case::sha1_abbreviation(SHA1_ABBREVIATION)]
        #[case::sha256_abbreviation(SHA256_ABBREVIATION)]
        fn accepts_object_ids_and_their_hex_shortcuts(#[case] input: &str) {
            let found = SideArg::try_from(&Value::string(input, span(1)));

            assert_matches!(found, Ok(SideArg::Rev(revision)) if revision == input);
        }

        #[rstest]
        #[case::sha1(SHA1_HEX_INVALID)]
        #[case::sha256(SHA256_HEX_INVALID)]
        fn rejects_object_id_shaped_strings_that_are_not_hexadecimal(#[case] input: &str) {
            let input_span = span(41);
            let err = SideArg::try_from(&Value::string(input, input_span));

            assert_matches!(err, Err(CommandError::InvalidObjectId { span: found, .. }) if found == input_span);
        }

        #[rstest]
        fn parses_a_revision_record() {
            let input = Value::record(
                record! { REV_FIELD => Value::string("refs/heads/main", span(7)) },
                span(3),
            );

            assert_matches!(SideArg::try_from(&input), Ok(SideArg::Rev(revision)) if revision == "refs/heads/main");
        }

        #[rstest]
        fn rejects_an_invalid_object_id_in_a_record_at_the_field_span() {
            let field_span = span(17);
            let input = Value::record(
                record! { REV_FIELD => Value::string(SHA1_HEX_INVALID, field_span) },
                span(3),
            );
            let err = SideArg::try_from(&input);

            assert_matches!(err, Err(CommandError::InvalidObjectId { span: found, .. }) if found == field_span);
        }

        #[rstest]
        #[case::empty("")]
        #[case::whitespace(" \t\n")]
        fn rejects_blank_strings(#[case] input: &str) {
            let input_span = span(11);
            let err = SideArg::try_from(&Value::string(input, input_span));

            assert_matches!(err, Err(CommandError::EmptyRevision { span: found }) if found == input_span);
        }

        #[rstest]
        fn rejects_a_blank_record_revision_at_the_field_span() {
            let field_span = span(17);
            let input = Value::record(
                record! { REV_FIELD => Value::string(" ", field_span) },
                span(3),
            );
            let err = SideArg::try_from(&input);

            assert_matches!(err, Err(CommandError::EmptyRevision { span: found }) if found == field_span);
        }

        #[rstest]
        #[case::empty(record! {})]
        #[case::missing_revision(record! { "branch" => Value::string("main", span(5)) })]
        #[case::extra_field(record! {
            REV_FIELD => Value::string("main", span(5)),
            "other" => Value::string("x", span(8)),
        })]
        fn rejects_invalid_records(#[case] record: record::Record) {
            let record_span = span(23);
            let err = SideArg::try_from(&Value::record(record, record_span));

            assert_matches!(err, Err(CommandError::InvalidSideRecord { span: found }) if found == record_span);
        }

        #[rstest]
        #[case::integer(Value::int(1, span(31)), "int")]
        #[case::boolean(Value::bool(true, span(31)), "bool")]
        #[case::nothing(Value::nothing(span(31)), "nothing")]
        #[case::list(Value::list(Vec::new(), span(31)), "list<any>")]
        fn rejects_non_side_types(#[case] input: Value, #[case] got: &str) {
            let input_span = input.span();
            let err = SideArg::try_from(&input);

            assert_matches!(err, Err(CommandError::InvalidSideType { got: found, span }) if found == got && span == input_span);
        }

        #[rstest]
        fn rejects_a_non_string_revision_at_the_field_span() {
            let field_span = span(37);
            let input = Value::record(record! { REV_FIELD => Value::int(3, field_span) }, span(3));
            let err = SideArg::try_from(&input);

            assert_matches!(err, Err(CommandError::InvalidSideField { field: REV_FIELD, got, span: found }) if got == "int" && found == field_span);
        }
    }

    mod resolve {
        use super::*;

        /// A repository with two commits on `main`, opened for revision resolution.
        #[fixture]
        fn opened_repo() -> Result<(TestRepo, OpenRepository), TestError> {
            let repo = test_repo()?;
            let location = RepositoryLocation::discover(repo.dir.path())?;
            let opened = OpenRepository::try_from(&location)?;

            Ok((repo, opened))
        }

        #[rstest]
        #[case::head(SideArg::Head, Side::Head)]
        #[case::index(SideArg::Index, Side::Index)]
        #[case::worktree(SideArg::Worktree, Side::Worktree)]
        fn maps_symbolic_sides_to_their_side(
            opened_repo: Result<(TestRepo, OpenRepository), TestError>,
            #[case] side: SideArg,
            #[case] expected: Side,
        ) -> Result<(), TestError> {
            let (_repo, opened) = opened_repo?;

            assert_matches!(side.resolve(&opened, span(1)), Ok(actual) if actual == expected);
            Ok(())
        }

        #[rstest]
        fn resolves_a_revision_to_a_rev_side(
            opened_repo: Result<(TestRepo, OpenRepository), TestError>,
        ) -> Result<(), TestError> {
            let (_repo, opened) = opened_repo?;
            let side = SideArg::Rev(MAIN_BRANCH.to_owned());

            assert_matches!(side.resolve(&opened, span(1)), Ok(Side::Rev(_)));
            Ok(())
        }

        #[rstest]
        fn unresolvable_revision_surfaces_as_a_coded_labeled_error(
            opened_repo: Result<(TestRepo, OpenRepository), TestError>,
        ) -> Result<(), TestError> {
            let (_repo, opened) = opened_repo?;
            let input_span = span(9);
            let err = SideArg::Rev("no-such-branch".to_owned())
                .resolve(&opened, input_span)
                .err()
                .ok_or_else(|| TestError::Failure("unresolvable revision resolved".to_owned()))?;
            let labeled = err.into_labeled(input_span);

            assert_matches!(labeled.code.as_deref(), Some(code) if !code.is_empty());
            Ok(())
        }
    }
}
