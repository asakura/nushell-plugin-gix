//! Serializable descriptions of the sides of a repository comparison.
//!
//! A [`Side`] names where a Git operation reads content from or writes content to: a
//! symbolic side (`head`, `index`, `worktree`) or a revision resolved to object IDs.
//! It holds no repository-bound `gix` types, so it can be embedded in a Nushell custom
//! value and rebuilt after a plugin restart; the `gix` types it originates from stay
//! inside the git layer. [`gix::ObjectId`] and [`gix::revision::Spec`] are allowed here:
//! the workspace controls the compiled feature set, both are `Copy`, `Eq` and serde-aware,
//! and `Spec` carries only detached object IDs, so they are plain data on this boundary.

use crate::error::GixError;

use gix::revision::plumbing::Spec as DetachedSpec;
use serde::{Deserialize, Serialize};

use std::fmt;

/// Spelling of the symbolic `head` side.
const HEAD: &str = "head";

/// Spelling of the symbolic `index` side.
const INDEX: &str = "index";

/// Spelling of the symbolic `worktree` side.
const WORKTREE: &str = "worktree";

/// A revision resolved to the plain data `gix` detaches from a revision specification.
pub type ResolvedRevision = DetachedSpec;

/// One side of a repository comparison or content operation, in its serializable form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    /// The commit `HEAD` points at.
    Head,
    /// The staged snapshot, or a scratch index.
    Index,
    /// Files on disk in the working tree.
    Worktree,
    /// A revision resolved to plain object IDs.
    Rev(ResolvedRevision),
}

impl Side {
    /// Returns whether this side cannot be written to.
    #[must_use = "the mutability classification is needed to choose a valid operation"]
    pub const fn is_read_only(&self) -> bool {
        matches!(self, Self::Head | Self::Rev(_))
    }

    /// Returns whether this side reflects live repository state.
    #[must_use = "the liveness classification is needed to validate handle state"]
    pub const fn is_live(&self) -> bool {
        matches!(self, Self::Index | Self::Worktree)
    }

    /// Resolves `expression` in `repository` and retains only its detached revision data.
    ///
    /// # Errors
    ///
    /// Returns [`GixError::Revision`] when the revision expression cannot be resolved.
    pub fn resolve_revision(
        repository: &gix::Repository,
        expression: &str,
    ) -> Result<Self, GixError> {
        repository
            .rev_parse(expression)
            .map_err(|source| GixError::Revision {
                expression: expression.to_owned(),
                source,
            })
            .map(gix::revision::Spec::detach)
            .map(Self::Rev)
    }
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Head => f.write_str(HEAD),
            Self::Index => f.write_str(INDEX),
            Self::Worktree => f.write_str(WORKTREE),
            Self::Rev(revision) => revision.fmt(f),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use gix::actor::SignatureRef;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use std::io;

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Io(#[from] io::Error),

        #[error(transparent)]
        Gix(#[from] gix::Error),

        #[error(transparent)]
        Git(#[from] GixError),

        #[error(transparent)]
        Json(#[from] serde_json::Error),
    }

    const COMMIT_MESSAGE: &str = "initial";
    const COMMIT_TIME: &str = "1700000000 +0000";

    const HEX: &str = "0123456789012345678901234567890123456789";
    const OID: gix::ObjectId = gix::ObjectId::Sha1([
        0x01, 0x23, 0x45, 0x67, 0x89, 0x01, 0x23, 0x45, 0x67, 0x89, 0x01, 0x23, 0x45, 0x67, 0x89,
        0x01, 0x23, 0x45, 0x67, 0x89,
    ]);
    const EXCLUDE_DISPLAY: &str = "^0123456789012345678901234567890123456789";
    const RANGE_DISPLAY: &str =
        "0123456789012345678901234567890123456789..0123456789012345678901234567890123456789";
    const MERGE_DISPLAY: &str =
        "0123456789012345678901234567890123456789...0123456789012345678901234567890123456789";
    const INCLUDE_ONLY_PARENTS_DISPLAY: &str = "0123456789012345678901234567890123456789^@";
    const EXCLUDE_PARENTS_DISPLAY: &str = "0123456789012345678901234567890123456789^!";

    /// A repository in a temporary directory with one commit on `HEAD`.
    #[fixture]
    fn repo() -> Result<(TempDir, gix::Repository), TestError> {
        let dir = TempDir::new()?;
        let repo = gix::init(dir.path())?;
        let signature = SignatureRef {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            time: COMMIT_TIME,
        };

        repo.commit_as(
            signature,
            signature,
            "HEAD",
            COMMIT_MESSAGE,
            repo.empty_tree().id,
            Vec::<gix::ObjectId>::new(),
        )?;

        Ok((dir, repo))
    }

    mod resolved_revision {
        use super::*;

        #[rstest]
        #[case::include(ResolvedRevision::Include(OID), HEX)]
        #[case::exclude(ResolvedRevision::Exclude(OID), EXCLUDE_DISPLAY)]
        #[case::range(ResolvedRevision::Range { from: OID, to: OID }, RANGE_DISPLAY)]
        #[case::merge(ResolvedRevision::Merge { theirs: OID, ours: OID }, MERGE_DISPLAY)]
        #[case::include_only_parents(
            ResolvedRevision::IncludeOnlyParents(OID),
            INCLUDE_ONLY_PARENTS_DISPLAY
        )]
        #[case::exclude_parents(ResolvedRevision::ExcludeParents(OID), EXCLUDE_PARENTS_DISPLAY)]
        fn displays_like_the_git_revision_syntax(
            #[case] revision: ResolvedRevision,
            #[case] expected: &str,
        ) {
            assert_eq!(revision.to_string(), expected);
        }
    }

    mod side {
        use super::*;

        #[rstest]
        #[case::head(Side::Head, true, false, "head")]
        #[case::index(Side::Index, false, true, "index")]
        #[case::worktree(Side::Worktree, false, true, "worktree")]
        fn classifies_and_formats(
            #[case] side: Side,
            #[case] read_only: bool,
            #[case] live: bool,
            #[case] spelling: &str,
        ) {
            assert_eq!(side.is_read_only(), read_only);
            assert_eq!(side.is_live(), live);
            assert_eq!(side.to_string(), spelling);
        }

        #[rstest]
        fn classifies_and_formats_a_revision() {
            let side = Side::Rev(ResolvedRevision::Include(OID));

            assert!(side.is_read_only());
            assert!(!side.is_live());
            assert_eq!(side.to_string(), HEX);
        }

        #[rstest]
        #[case::head(Side::Head)]
        #[case::index(Side::Index)]
        #[case::worktree(Side::Worktree)]
        #[case::revision(Side::Rev(ResolvedRevision::Include(OID)))]
        #[case::range(Side::Rev(ResolvedRevision::Range { from: OID, to: OID }))]
        fn round_trips_through_serde(#[case] side: Side) -> Result<(), TestError> {
            let encoded = serde_json::to_string(&side)?;
            let decoded: Side = serde_json::from_str(&encoded)?;

            assert_eq!(decoded, side);

            Ok(())
        }
    }

    mod resolve_revision {
        use super::*;

        #[rstest]
        fn resolves_head_to_a_detached_object_id(
            repo: Result<(TempDir, gix::Repository), TestError>,
        ) -> Result<(), TestError> {
            let (_dir, repo) = repo?;
            let head = repo.rev_parse_single("HEAD")?.detach();

            let side = Side::resolve_revision(&repo, "HEAD")?;

            assert!(matches!(
                side,
                Side::Rev(ResolvedRevision::Include(oid)) if oid == head
            ));

            Ok(())
        }

        #[rstest]
        fn reports_an_unresolvable_expression(
            repo: Result<(TempDir, gix::Repository), TestError>,
        ) -> Result<(), TestError> {
            let (_dir, repo) = repo?;
            let result = Side::resolve_revision(&repo, "not-a-revision");

            assert!(
                matches!(result, Err(GixError::Revision { expression, .. }) if expression == "not-a-revision")
            );

            Ok(())
        }
    }
}
