//! Resolving revision expressions to object metadata.

use crate::error::GixError;
use crate::repository::OpenRepository;

use gix::{hash::Prefix, object::Kind};

/// The kind of object a revision resolved to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevKind {
    /// A commit object.
    Commit,
    /// A tree object.
    Tree,
    /// A blob object.
    Blob,
    /// A tag object.
    Tag,
}

impl RevKind {
    /// The lower-case git object type name, as shown in the `kind` column.
    #[must_use = "the object type name is the user-facing spelling of this kind"]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Commit => "commit",
            Self::Tree => "tree",
            Self::Blob => "blob",
            Self::Tag => "tag",
        }
    }

    /// Maps the total-in-practice `gix` object kind onto the public kind.
    const fn from_object(kind: Kind) -> Self {
        match kind {
            Kind::Commit => Self::Commit,
            Kind::Tree => Self::Tree,
            Kind::Blob => Self::Blob,
            Kind::Tag => Self::Tag,
        }
    }
}

/// A revision resolved to the data shown by `gix rev`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevInfo {
    /// Full hex object id.
    pub oid: String,
    /// Kind of the resolved object.
    pub kind: RevKind,
    /// Full hex id of the object's tree; `None` for blobs.
    pub tree: Option<String>,
    /// Shortened hex object id.
    pub short: String,
}

/// Resolves `spec` in `opened` and collects the data shown by `gix rev`.
///
/// # Errors
///
/// Returns [`GixError::Revision`] when the expression cannot be resolved,
/// [`GixError::Shorten`] when the resolved id cannot be shortened, and
/// [`GixError::ShortLen`] when `short_len` is out of range.
pub fn resolve(
    opened: &OpenRepository,
    spec: &str,
    short_len: Option<usize>,
) -> Result<RevInfo, GixError> {
    let repo = opened.repo();

    let id = repo
        .rev_parse_single(spec)
        .map_err(|source| GixError::Revision {
            expression: spec.to_owned(),
            source,
        })?;

    let oid = id.detach();

    let object = repo.find_object(oid).map_err(|source| GixError::Revision {
        expression: spec.to_owned(),
        source,
    })?;

    let kind = RevKind::from_object(object.kind);

    let tree = if kind == RevKind::Blob {
        None
    } else {
        let tree = object
            .peel_to_kind(Kind::Tree)
            .map_err(|source| GixError::Revision {
                expression: spec.to_owned(),
                source,
            })?;

        Some(tree.id.to_string())
    };

    let short = shorten(&repo, spec, id, short_len)?;

    Ok(RevInfo {
        oid: oid.to_string(),
        kind,
        tree,
        short,
    })
}

/// Computes the short hex id: auto-length via `core.abbrev` when `short_len` is `None`,
/// otherwise the requested length after validating the range.
fn shorten(
    repo: &gix::Repository,
    spec: &str,
    id: gix::Id<'_>,
    short_len: Option<usize>,
) -> Result<String, GixError> {
    match short_len {
        None => Ok(id
            .shorten()
            .map_err(|source| GixError::Shorten {
                expression: spec.to_owned(),
                source,
            })?
            .to_string()),
        Some(len) => {
            let min = Prefix::MIN_HEX_LEN;
            let max = repo.object_hash().len_in_hex();

            if len < min || len > max {
                let detail = format!("expected {min}..={max}");
                return Err(GixError::ShortLen { len, detail });
            }

            let prefix = Prefix::new(&id.detach(), len).map_err(|error| GixError::ShortLen {
                len,
                detail: error.to_string(),
            })?;

            Ok(prefix.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fixtures::open_fixture;
    use nu_plugin_gix_testing_support::{
        ANNOTATED_TAG, FEATURE_BRANCH, FixtureError, TaggedTestRepo, tagged_repo,
    };
    use rstest::rstest;

    use std::assert_matches;

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Gix(#[from] gix::Error),

        #[error(transparent)]
        Git(#[from] GixError),

        #[error(transparent)]
        Fixture(#[from] FixtureError),

        #[error("Test failure: {0}")]
        Failure(String),
    }

    const HEAD: &str = "HEAD";
    const RANGE_SPEC: &str = "HEAD~1..HEAD";
    const UNRESOLVABLE_SPEC: &str = "not-a-revision";
    const NO_ERROR_FAILURE: &str = "expected an error";

    mod resolve {
        use super::*;

        #[rstest]
        fn resolves_head_to_a_commit(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;
            let info = resolve(&repo, HEAD, None)?;

            assert_eq!(info.oid, fixture.commit.to_string());
            assert_eq!(info.kind, RevKind::Commit);

            assert_eq!(
                info.tree.as_deref(),
                Some(fixture.tree.to_string().as_str())
            );

            Ok(())
        }

        #[rstest]
        fn resolves_a_tree_object(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;
            let info = resolve(&repo, &fixture.tree.to_string(), None)?;

            assert_eq!(info.kind, RevKind::Tree);

            assert_eq!(
                info.tree.as_deref(),
                Some(fixture.tree.to_string().as_str())
            );

            Ok(())
        }

        #[rstest]
        fn resolves_a_blob_without_a_tree(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;
            let info = resolve(&repo, &fixture.blob.to_string(), None)?;

            assert_eq!(info.kind, RevKind::Blob);
            assert_eq!(info.tree, None);

            Ok(())
        }

        #[rstest]
        fn resolves_an_annotated_tag_to_its_peeled_tree(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;
            let info = resolve(&repo, ANNOTATED_TAG, None)?;

            assert_eq!(info.oid, fixture.tag.to_string());
            assert_eq!(info.kind, RevKind::Tag);

            assert_eq!(
                info.tree.as_deref(),
                Some(fixture.tree.to_string().as_str())
            );

            Ok(())
        }

        #[rstest]
        fn resolves_a_branch(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;
            let info = resolve(&repo, FEATURE_BRANCH, None)?;

            assert_eq!(info.oid, fixture.commit.to_string());
            assert_eq!(info.kind, RevKind::Commit);

            Ok(())
        }

        #[rstest]
        fn auto_shortens_to_a_prefix_of_the_object_id(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;
            let info = resolve(&repo, HEAD, None)?;

            assert!(
                fixture.commit.to_string().starts_with(&info.short),
                "short `{}` is not a prefix of `{oid}`",
                info.short
            );

            Ok(())
        }

        #[rstest]
        fn shortens_to_the_requested_length(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            const SHORT_LEN: usize = 8;
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;
            let info = resolve(&repo, HEAD, Some(SHORT_LEN))?;

            assert_eq!(info.short.len(), SHORT_LEN);

            Ok(())
        }

        #[rstest]
        fn reports_an_unresolvable_expression(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;

            let err = resolve(&repo, UNRESOLVABLE_SPEC, None)
                .err()
                .ok_or_else(|| TestError::Failure(NO_ERROR_FAILURE.to_owned()))?;

            assert_matches!(
                &err,
                GixError::Revision { expression, .. } if expression == UNRESOLVABLE_SPEC,
                "got {err:?}"
            );

            Ok(())
        }

        #[rstest]
        fn reports_a_range_expression(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;

            let err = resolve(&repo, RANGE_SPEC, None)
                .err()
                .ok_or_else(|| TestError::Failure(NO_ERROR_FAILURE.to_owned()))?;

            assert_matches!(err, GixError::Revision { expression, .. } if expression == RANGE_SPEC);

            Ok(())
        }

        #[rstest]
        fn rejects_a_too_short_length(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            const TOO_SHORT: usize = 3;

            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;

            let err = resolve(&repo, HEAD, Some(TOO_SHORT))
                .err()
                .ok_or_else(|| TestError::Failure(NO_ERROR_FAILURE.to_owned()))?;

            assert_matches!(
                err,
                GixError::ShortLen { len: TOO_SHORT, .. },
                "got {err:?}"
            );

            Ok(())
        }

        #[rstest]
        fn rejects_a_too_long_length(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            const TOO_LONG: usize = 41;

            let fixture = tagged_repo?;
            let repo = open_fixture(&fixture)?;

            let err = resolve(&repo, HEAD, Some(TOO_LONG))
                .err()
                .ok_or_else(|| TestError::Failure(NO_ERROR_FAILURE.to_owned()))?;

            assert_matches!(err, GixError::ShortLen { len: TOO_LONG, .. }, "got {err:?}");

            Ok(())
        }
    }
}
