//! Reading and writing references: HEAD state, listing, and compare-and-swap updates.

use crate::error::GixError;
use crate::repository::OpenRepository;

use gix::head::Kind;
use gix::refs::transaction::{PreviousValue, RefEdit};
use gix::refs::{FullName, Target, TargetRef};

use std::fmt;

const REFS_PREFIX: &str = "refs/";

/// The maximum number of symbolic links followed when resolving a reference chain,
/// mirroring git's own limit.
const MAX_SYMBOLIC_DEPTH: usize = 5;

/// A reference's full name together with what it points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefInfo {
    /// Full name of the reference.
    pub name: String,
    /// What the reference points at.
    pub target: RefTarget,
}

/// What a reference points at: an object id for a direct reference, or the full name of
/// the reference it names for a symbolic one. Symbolic targets are not followed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefTarget {
    /// Hex id of the object a direct reference points at.
    Object(String),
    /// Full name of the reference a symbolic reference points at.
    Symbolic(String),
}

impl RefTarget {
    /// The text the target is spelled as in reference errors: the hex object id, or the full
    /// name of a symbolic target.
    #[must_use = "the spelling is meant for error reports"]
    pub const fn spelled(&self) -> &str {
        match self {
            Self::Object(text) | Self::Symbolic(text) => text.as_str(),
        }
    }
}

/// The state of `HEAD`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadInfo {
    /// Hex id of the object HEAD points at.
    pub oid: String,
    /// Full name of the branch HEAD points at; `None` when detached.
    pub name: Option<String>,
}

/// The value an update requires a reference to currently hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefExpect {
    /// The reference must not exist.
    Missing,
    /// The reference must exist and point at this object. The command side never names or
    /// constructs the contained `gix::ObjectId`; it only obtains a `RefExpect` through
    /// `RefExpect::parse`, so the gix type stays opaque across the boundary.
    Oid(gix::ObjectId),
}

impl RefExpect {
    /// The spelling of [`RefExpect::Missing`] in expectation values and error reports.
    pub const MISSING: &str = "missing";

    /// Parses an expectation value: `missing` or a full hex object id.
    ///
    /// # Errors
    ///
    /// Returns [`GixError::InvalidOid`] when the value is neither the sentinel nor a
    /// full hex object id.
    pub fn parse(value: &str) -> Result<Self, GixError> {
        if value == Self::MISSING {
            return Ok(Self::Missing);
        }

        parse_oid(value).map(Self::Oid)
    }

    /// Whether a reference currently holding `observed` (`None` when it does not exist)
    /// satisfies this expectation.
    #[must_use = "the verdict decides which error is reported"]
    fn is_met_by(self, observed: Option<&RefTarget>) -> bool {
        match (self, observed) {
            (Self::Missing, None) => true,
            (Self::Oid(oid), Some(RefTarget::Object(actual))) => {
                let expected = oid.to_string();
                *actual == expected
            }
            (Self::Missing, Some(_)) | (Self::Oid(_), None | Some(RefTarget::Symbolic(_))) => false,
        }
    }
}

impl fmt::Display for RefExpect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str(Self::MISSING),
            Self::Oid(oid) => write!(f, "{oid}"),
        }
    }
}

/// Reads the state of `HEAD`.
///
/// A detached HEAD reports its peeled object id, so an annotated tag resolves to the commit
/// it names. A symbolic HEAD follows its reference chain and reports the id the final
/// reference stores, without peeling it.
///
/// # Errors
///
/// Returns [`GixError::UnbornHead`] when HEAD points at a branch without commits,
/// [`GixError::DanglingRef`] when its symbolic chain is broken, and
/// [`GixError::Head`] or [`GixError::RefRead`] when git itself fails.
pub fn head(opened: &OpenRepository) -> Result<HeadInfo, GixError> {
    let repo = opened.repo();
    let head = repo.head().map_err(GixError::Head)?;

    match head.kind {
        Kind::Detached { target, peeled } => Ok(HeadInfo {
            oid: peeled.unwrap_or(target).to_string(),
            name: None,
        }),
        Kind::Symbolic(reference) => {
            let name = reference.name.to_string();
            let oid = resolve_symbolic(&repo, &name)?;
            Ok(HeadInfo {
                oid,
                name: Some(name),
            })
        }
        Kind::Unborn(name) => Err(GixError::UnbornHead {
            name: name.to_string(),
        }),
    }
}

/// Follows a reference chain from `start` until an object id is reached.
fn resolve_symbolic(repo: &gix::Repository, start: &str) -> Result<String, GixError> {
    let mut name = start.to_owned();

    for _ in 0..MAX_SYMBOLIC_DEPTH {
        let reference = repo
            .try_find_reference(name.as_str())
            .map_err(GixError::RefRead)?
            .ok_or_else(|| GixError::DanglingRef { name: name.clone() })?;

        match reference.target() {
            TargetRef::Object(oid) => return Ok(oid.to_string()),
            TargetRef::Symbolic(target) => name = target.to_string(),
        }
    }

    Err(GixError::DanglingRef { name })
}

/// Lists references whose full name starts with `prefix`, sorted by name.
///
/// The prefix may be a full `refs/` path like `refs/tags` or a shorthand like
/// `heads`; an empty prefix lists every reference.
///
/// # Errors
///
/// Returns [`GixError::RefRead`] when the reference list cannot be read.
pub fn list(opened: &OpenRepository, prefix: &str) -> Result<Vec<RefInfo>, GixError> {
    let repo = opened.repo();
    let platform = repo.references().map_err(GixError::RefRead)?;

    let iter = if prefix.is_empty() {
        platform.all().map_err(GixError::RefRead)?
    } else {
        let normalized = normalize_prefix(prefix);

        platform
            .prefixed(normalized.as_str())
            .map_err(GixError::RefRead)?
    };

    let mut rows = iter
        .map(|reference| {
            let reference = reference.map_err(GixError::RefRead)?;
            Ok(ref_info(&reference))
        })
        .collect::<Result<Vec<_>, GixError>>()?;

    rows.sort_by(|left, right| left.name.cmp(&right.name));

    Ok(rows)
}

/// Prepends [`REFS_PREFIX`] to a shorthand prefix like `heads`.
fn normalize_prefix(prefix: &str) -> String {
    if prefix.starts_with(REFS_PREFIX) {
        prefix.to_owned()
    } else {
        format!("{REFS_PREFIX}{prefix}")
    }
}

/// Converts a reference into a [`RefInfo`] row.
fn ref_info(reference: &gix::Reference<'_>) -> RefInfo {
    RefInfo {
        name: reference.name().to_string(),
        target: ref_target(reference),
    }
}

/// Reads what a reference points at, without following symbolic targets.
#[must_use = "the target is the reason the reference was read"]
fn ref_target(reference: &gix::Reference<'_>) -> RefTarget {
    match reference.target() {
        TargetRef::Object(oid) => RefTarget::Object(oid.to_string()),
        TargetRef::Symbolic(target) => RefTarget::Symbolic(target.to_string()),
    }
}

/// Updates the full reference name `name` to point at `new_target`, requiring it to
/// currently hold the value described by `expect`, and records `message` in its reflog.
///
/// Only full names under `refs/` are accepted. `RefExpect::Missing` is additionally
/// enforced by an explicit read before the transaction, because gix-ref 0.67.1 waives
/// a `MustNotExist` precondition when the new value equals the value the reference
/// already holds (a no-op rewrite).
///
/// # Errors
///
/// Returns [`GixError::InvalidRefName`] when `name` is not a valid full reference name,
/// [`GixError::InvalidOid`] when `new_target` is malformed, [`GixError::RefMoved`] when
/// the reference does not hold the expected value, and [`GixError::RefUpdate`] when the
/// write fails for another reason.
pub fn set(
    opened: &OpenRepository,
    name: &str,
    new_target: &str,
    expect: RefExpect,
    message: &str,
) -> Result<RefInfo, GixError> {
    let repo = opened.repo();

    if !name.starts_with(REFS_PREFIX) {
        return Err(GixError::InvalidRefName {
            name: name.to_owned(),
            detail: format!("expected a full name starting with '{REFS_PREFIX}'"),
        });
    }

    let full_name = FullName::try_from(name).map_err(|error| GixError::InvalidRefName {
        name: name.to_owned(),
        detail: error.to_string(),
    })?;

    let oid = parse_oid(new_target)?;

    if expect == RefExpect::Missing
        && let Some(actual) = observed_target(&repo, name)?
    {
        return Err(GixError::RefMoved {
            name: name.to_owned(),
            expected: RefExpect::Missing,
            actual: Some(actual),
        });
    }

    let edit = RefEdit::update(
        full_name,
        Target::Object(oid),
        expected_value(expect),
        message,
    );

    match repo.edit_reference(edit) {
        Ok(_) => Ok(RefInfo {
            name: name.to_owned(),
            target: RefTarget::Object(oid.to_string()),
        }),
        Err(source) => Err(classify_failure(&repo, name, expect, source)),
    }
}

/// Parses a full hex object id.
#[expect(
    clippy::map_err_ignore,
    reason = "`GixError::InvalidOid` has no source field and its message already echoes \
              the rejected value, so the `from_hex` failure adds no user-facing detail"
)]
fn parse_oid(value: &str) -> Result<gix::ObjectId, GixError> {
    gix::ObjectId::from_hex(value.as_bytes()).map_err(|_| GixError::InvalidOid {
        value: value.to_owned(),
    })
}

/// Translates an expectation into the precondition of a reference transaction.
const fn expected_value(expect: RefExpect) -> PreviousValue {
    match expect {
        RefExpect::Missing => PreviousValue::MustNotExist,
        RefExpect::Oid(oid) => PreviousValue::MustExistAndMatch(Target::Object(oid)),
    }
}

/// Reads what the full reference name `name` currently points at; `None` when the
/// reference does not exist.
fn observed_target(repo: &gix::Repository, name: &str) -> Result<Option<RefTarget>, GixError> {
    let reference = repo.try_find_reference(name).map_err(GixError::RefRead)?;

    Ok(reference.as_ref().map(ref_target))
}

/// Distinguishes a concurrent move from a genuine update failure by re-reading the
/// reference and checking it against `expect`.
fn classify_failure(
    repo: &gix::Repository,
    name: &str,
    expect: RefExpect,
    source: gix::Error,
) -> GixError {
    let observed = match observed_target(repo, name) {
        Ok(observed) => observed,
        Err(error) => return error,
    };

    if expect.is_met_by(observed.as_ref()) {
        GixError::RefUpdate {
            name: name.to_owned(),
            source,
        }
    } else {
        GixError::RefMoved {
            name: name.to_owned(),
            expected: expect,
            actual: observed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fixtures::open_fixture;
    use gix::Id;
    use gix::open::Options;
    use gix::refs::Target;
    use gix::refs::name::Error as NameError;
    use gix::refs::transaction::{PreviousValue, RefEdit};
    use nu_plugin_gix_testing_support::{
        ANNOTATED_TAG_REF, FEATURE_BRANCH_REF, FIXTURE_REFLOG_MESSAGE, FixtureError, GIT_DIR,
        LIGHT_TAG_REF, MAIN_BRANCH, TaggedTestRepo, UnbornTestRepo, tagged_repo, unborn_repo,
    };
    use rstest::rstest;

    use std::assert_matches;
    use std::fs;
    use std::io;

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Io(#[from] io::Error),

        #[error(transparent)]
        Gix(#[from] gix::Error),

        #[error(transparent)]
        Name(#[from] NameError),

        #[error(transparent)]
        Git(#[from] GixError),

        #[error(transparent)]
        Fixture(#[from] FixtureError),

        #[error("Test failure: {0}")]
        Failure(String),
    }

    const HEAD_NAME: &str = "HEAD";
    const SET_MESSAGE: &str = "custom set message";
    const SHORT_TARGET: &str = "nothex";
    const SHA256_TARGET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const EXPECTED_ERROR: &str = "expected an error";

    mod ref_expect {
        use super::*;

        #[rstest]
        fn parses_the_missing_sentinel() -> Result<(), TestError> {
            const MISSING_VALUE: &str = "missing";

            assert_eq!(RefExpect::parse(MISSING_VALUE)?, RefExpect::Missing);

            Ok(())
        }

        #[rstest]
        fn parses_a_full_object_id() -> Result<(), TestError> {
            const OID_HEX: &str = "0123456789abcdef0123456789abcdef01234567";

            let expected = RefExpect::Oid(gix::ObjectId::from_hex(OID_HEX.as_bytes())?);

            assert_eq!(RefExpect::parse(OID_HEX)?, expected);

            Ok(())
        }

        #[rstest]
        fn rejects_a_short_object_id() {
            const SHORT_HEX: &str = "0123456";

            assert_matches!(
                RefExpect::parse(SHORT_HEX),
                Err(GixError::InvalidOid { value }) if value == SHORT_HEX,
                "got {:?}",
                RefExpect::parse(SHORT_HEX)
            );
        }

        #[rstest]
        fn rejects_garbage() {
            const GARBAGE: &str = "not-an-oid";

            assert_matches!(RefExpect::parse(GARBAGE), Err(GixError::InvalidOid { .. }));
        }
    }

    mod ref_moved {
        use super::*;

        const MOVED_NAME: &str = "refs/heads/main";
        const EXPECTED_HEX: &str = "0123456789abcdef0123456789abcdef01234567";
        const FOUND_HEX: &str = "fedcba9876543210fedcba9876543210fedcba98";
        const FOUND_SYMBOLIC: &str = "refs/heads/elsewhere";

        #[rstest]
        #[case::missing_actual(None, RefExpect::MISSING)]
        #[case::object_actual(Some(RefTarget::Object(FOUND_HEX.to_owned())), FOUND_HEX)]
        #[case::symbolic_actual(
            Some(RefTarget::Symbolic(FOUND_SYMBOLIC.to_owned())),
            FOUND_SYMBOLIC
        )]
        fn spells_the_actual_value_in_the_message(
            #[case] actual: Option<RefTarget>,
            #[case] found: &str,
        ) -> Result<(), TestError> {
            let err = GixError::RefMoved {
                name: MOVED_NAME.to_owned(),
                expected: RefExpect::parse(EXPECTED_HEX)?,
                actual,
            };

            assert_eq!(
                err.to_string(),
                format!(
                    "reference '{MOVED_NAME}' moved concurrently: expected {EXPECTED_HEX}, but found {found}"
                )
            );

            Ok(())
        }
    }

    mod head {
        use super::*;

        #[rstest]
        fn reports_the_branch_and_its_commit(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;
            let info = head(&repo)?;

            assert_eq!(
                info,
                HeadInfo {
                    oid: fixture.commit.to_string(),
                    name: Some(FEATURE_BRANCH_REF.to_owned()),
                }
            );

            Ok(())
        }

        #[rstest]
        fn reports_a_detached_head_without_a_name(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;

            repo.repo().edit_reference(RefEdit::update(
                HEAD_NAME.try_into()?,
                Target::Object(fixture.commit),
                PreviousValue::Any,
                FIXTURE_REFLOG_MESSAGE,
            ))?;

            let info = head(&repo)?;

            assert_eq!(
                info,
                HeadInfo {
                    oid: fixture.commit.to_string(),
                    name: None,
                }
            );

            Ok(())
        }

        #[rstest]
        fn reports_an_unborn_head(
            unborn_repo: Result<UnbornTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = unborn_repo?;

            let opened =
                OpenRepository::from_repository(gix::open_opts(repo.path(), Options::isolated())?);

            let err = head(&opened)
                .err()
                .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            assert_matches!(err, GixError::UnbornHead { .. }, "got {err:?}");

            assert_matches!(
                &err,
                GixError::UnbornHead { name } if name == MAIN_BRANCH,
                "got {err:?}"
            );

            Ok(())
        }

        #[rstest]
        fn reports_a_broken_symbolic_chain(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            const DANGLING_NAME: &str = "refs/heads/dangling";
            const MISSING_NAME: &str = "refs/heads/missing";

            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;

            repo.repo().edit_reference(RefEdit::update(
                HEAD_NAME.try_into()?,
                Target::Symbolic(DANGLING_NAME.try_into()?),
                PreviousValue::Any,
                FIXTURE_REFLOG_MESSAGE,
            ))?;

            repo.repo().edit_reference(RefEdit::update(
                DANGLING_NAME.try_into()?,
                Target::Symbolic(MISSING_NAME.try_into()?),
                PreviousValue::Any,
                FIXTURE_REFLOG_MESSAGE,
            ))?;

            let err = head(&repo)
                .err()
                .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            assert_matches!(
                &err,
                GixError::DanglingRef { name } if name == MISSING_NAME,
                "got {err:?}"
            );

            Ok(())
        }
    }

    mod list {
        use super::*;

        const SYMBOLIC_NAME: &str = "refs/heads/symbolic";

        #[rstest]
        fn lists_all_references_sorted_by_name(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;
            let rows = list(&repo, "")?;
            let names = rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>();

            assert_eq!(
                names,
                vec![FEATURE_BRANCH_REF, ANNOTATED_TAG_REF, LIGHT_TAG_REF]
            );

            let branch = rows
                .first()
                .ok_or_else(|| TestError::Failure("expected the branch row".to_owned()))?;

            assert_eq!(branch.target, RefTarget::Object(fixture.commit.to_string()));

            Ok(())
        }

        #[rstest]
        fn lists_references_under_a_shorthand_prefix(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            const SHORT_PREFIX: &str = "heads";

            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;
            let rows = list(&repo, SHORT_PREFIX)?;
            let names = rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>();

            assert_eq!(names, vec![FEATURE_BRANCH_REF]);

            Ok(())
        }

        #[rstest]
        fn lists_references_under_a_full_prefix(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            const FULL_PREFIX: &str = "refs/tags";

            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;
            let rows = list(&repo, FULL_PREFIX)?;
            let names = rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>();

            assert_eq!(names, vec![ANNOTATED_TAG_REF, LIGHT_TAG_REF]);

            Ok(())
        }

        #[rstest]
        fn shows_the_symbolic_target_without_following_it(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;

            repo.repo().edit_reference(RefEdit::update(
                SYMBOLIC_NAME.try_into()?,
                Target::Symbolic(FEATURE_BRANCH_REF.try_into()?),
                PreviousValue::MustNotExist,
                FIXTURE_REFLOG_MESSAGE,
            ))?;

            let rows = list(&repo, SYMBOLIC_NAME)?;

            assert_eq!(
                rows,
                vec![RefInfo {
                    name: SYMBOLIC_NAME.to_owned(),
                    target: RefTarget::Symbolic(FEATURE_BRANCH_REF.to_owned()),
                }]
            );

            Ok(())
        }
    }

    mod set {
        use super::*;

        const NEW_NAME: &str = "refs/heads/new";
        const ALIAS_NAME: &str = "refs/heads/alias";

        #[rstest]
        fn creates_a_reference_with_a_missing_expectation(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;

            let row = set(
                &repo,
                NEW_NAME,
                &fixture.commit.to_string(),
                RefExpect::Missing,
                SET_MESSAGE,
            )?;

            assert_eq!(
                row,
                RefInfo {
                    name: NEW_NAME.to_owned(),
                    target: RefTarget::Object(fixture.commit.to_string()),
                }
            );

            let stored = repo
                .repo()
                .try_find_reference(NEW_NAME)?
                .ok_or_else(|| TestError::Failure("expected the new reference".to_owned()))?;

            assert_eq!(stored.try_id().map(Id::detach), Some(fixture.commit));

            Ok(())
        }

        #[rstest]
        fn moves_a_reference_whose_value_matches(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;

            let row = set(
                &repo,
                LIGHT_TAG_REF,
                &fixture.blob.to_string(),
                RefExpect::Oid(fixture.commit),
                SET_MESSAGE,
            )?;

            assert_eq!(row.target, RefTarget::Object(fixture.blob.to_string()));

            Ok(())
        }

        #[rstest]
        fn reports_a_stale_expectation(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;

            let err = set(
                &repo,
                FEATURE_BRANCH_REF,
                &fixture.blob.to_string(),
                RefExpect::Oid(fixture.blob),
                SET_MESSAGE,
            )
            .err()
            .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            let expected_oid = RefExpect::Oid(fixture.blob);
            let actual_target = Some(RefTarget::Object(fixture.commit.to_string()));

            assert_matches!(
                &err,
                GixError::RefMoved { name, expected, actual }
                    if name == FEATURE_BRANCH_REF
                        && *expected == expected_oid
                        && *actual == actual_target,
                "got {err:?}"
            );

            Ok(())
        }

        #[rstest]
        fn reports_missing_when_expecting_an_object(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;

            let err = set(
                &repo,
                NEW_NAME,
                &fixture.commit.to_string(),
                RefExpect::Oid(fixture.commit),
                SET_MESSAGE,
            )
            .err()
            .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            assert_matches!(&err, GixError::RefMoved { actual: None, .. }, "got {err:?}");

            Ok(())
        }

        #[rstest]
        fn reports_an_existing_reference_when_expecting_missing(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;

            // The write must differ from the current value, since gix treats
            // a `MustNotExist` edit that re-writes the same value as idempotent.
            let actual_target = Some(RefTarget::Object(fixture.commit.to_string()));

            let err = set(
                &repo,
                FEATURE_BRANCH_REF,
                &fixture.blob.to_string(),
                RefExpect::Missing,
                SET_MESSAGE,
            )
            .err()
            .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            assert_matches!(
                &err,
                GixError::RefMoved { actual, .. } if *actual == actual_target,
                "got {err:?}"
            );

            Ok(())
        }

        #[rstest]
        fn reports_an_existing_reference_when_rewriting_its_current_value_as_missing(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;

            // gix-ref 0.67.1 waives `MustNotExist` when the new value equals the one the
            // reference already holds (a no-op rewrite), so the precondition must be
            // checked before the transaction is built.
            let actual_target = Some(RefTarget::Object(fixture.commit.to_string()));

            let err = set(
                &repo,
                LIGHT_TAG_REF,
                &fixture.commit.to_string(),
                RefExpect::Missing,
                SET_MESSAGE,
            )
            .err()
            .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            assert_matches!(
                &err,
                GixError::RefMoved { name, actual, .. }
                    if name == LIGHT_TAG_REF && *actual == actual_target,
                "got {err:?}"
            );

            Ok(())
        }

        #[rstest]
        fn creates_a_reference_named_like_the_missing_sentinel(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            const SENTINEL_NAME: &str = "refs/heads/missing";
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;

            let row = set(
                &repo,
                SENTINEL_NAME,
                &fixture.commit.to_string(),
                RefExpect::Missing,
                SET_MESSAGE,
            )?;

            assert_eq!(
                row,
                RefInfo {
                    name: SENTINEL_NAME.to_owned(),
                    target: RefTarget::Object(fixture.commit.to_string()),
                }
            );

            Ok(())
        }

        #[rstest]
        fn reports_an_existing_symbolic_reference_when_expecting_missing(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;

            repo.repo().edit_reference(RefEdit::update(
                ALIAS_NAME.try_into()?,
                Target::Symbolic(FEATURE_BRANCH_REF.try_into()?),
                PreviousValue::MustNotExist,
                FIXTURE_REFLOG_MESSAGE,
            ))?;

            let err = set(
                &repo,
                ALIAS_NAME,
                &fixture.commit.to_string(),
                RefExpect::Missing,
                SET_MESSAGE,
            )
            .err()
            .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            assert_matches!(
                &err,
                GixError::RefMoved { name, actual, .. }
                    if name == ALIAS_NAME
                        && *actual == Some(RefTarget::Symbolic(FEATURE_BRANCH_REF.to_owned())),
                "got {err:?}"
            );

            Ok(())
        }

        #[rstest]
        fn reports_a_symbolic_reference_when_expecting_an_object(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;

            repo.repo().edit_reference(RefEdit::update(
                ALIAS_NAME.try_into()?,
                Target::Symbolic(FEATURE_BRANCH_REF.try_into()?),
                PreviousValue::MustNotExist,
                FIXTURE_REFLOG_MESSAGE,
            ))?;

            let err = set(
                &repo,
                ALIAS_NAME,
                &fixture.commit.to_string(),
                RefExpect::Oid(fixture.blob),
                SET_MESSAGE,
            )
            .err()
            .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            let expected_oid = RefExpect::Oid(fixture.blob);
            let actual_symbolic = Some(RefTarget::Symbolic(FEATURE_BRANCH_REF.to_owned()));

            assert_matches!(
                &err,
                GixError::RefMoved { name, expected, actual }
                    if name == ALIAS_NAME
                        && *expected == expected_oid
                        && *actual == actual_symbolic,
                "got {err:?}"
            );

            Ok(())
        }

        #[rstest]
        fn rejects_a_shorthand_name(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            const SHORTHAND: &str = "feature";

            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;

            let err = set(
                &repo,
                SHORTHAND,
                &fixture.commit.to_string(),
                RefExpect::Missing,
                SET_MESSAGE,
            )
            .err()
            .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            assert_matches!(
                &err,
                GixError::InvalidRefName { name, .. } if name == SHORTHAND,
                "got {err:?}"
            );

            Ok(())
        }

        #[rstest]
        fn rejects_an_invalid_full_name(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            const INVALID_NAME: &str = "refs/heads/..";

            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;

            let err = set(
                &repo,
                INVALID_NAME,
                &fixture.commit.to_string(),
                RefExpect::Missing,
                SET_MESSAGE,
            )
            .err()
            .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            assert_matches!(
                err,
                GixError::InvalidRefName { name, .. } if name == INVALID_NAME
            );

            Ok(())
        }

        #[rstest]
        #[case::not_hexadecimal(SHORT_TARGET)]
        #[case::sha256_on_a_sha1_build(SHA256_TARGET)]
        fn rejects_a_malformed_target(
            #[case] target: &str,
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;

            let err = set(&repo, NEW_NAME, target, RefExpect::Missing, SET_MESSAGE)
                .err()
                .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            assert_matches!(
                &err,
                GixError::InvalidOid { value } if value == target,
                "got {err:?}"
            );

            Ok(())
        }

        #[rstest]
        fn writes_a_reflog_entry(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;

            set(
                &repo,
                NEW_NAME,
                &fixture.commit.to_string(),
                RefExpect::Missing,
                SET_MESSAGE,
            )?;

            let log_path = fixture
                .path()
                .join(GIT_DIR)
                .join("logs")
                .join("refs")
                .join("heads")
                .join("new");

            let log = fs::read_to_string(log_path)?;

            assert!(log.contains(SET_MESSAGE), "log: {log}");
            assert!(log.contains(&fixture.commit.to_string()), "log: {log}");

            Ok(())
        }

        #[rstest]
        fn reports_an_update_failure_without_committer_identity(
            unborn_repo: Result<UnbornTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            const OID_HEX: &str = "0123456789abcdef0123456789abcdef01234567";

            let opened = OpenRepository::from_repository(gix::open_opts(
                unborn_repo?.path(),
                Options::isolated(),
            )?);

            let err = set(&opened, NEW_NAME, OID_HEX, RefExpect::Missing, SET_MESSAGE)
                .err()
                .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            assert_matches!(err, GixError::RefUpdate { .. }, "got {err:?}");

            Ok(())
        }
    }
}
