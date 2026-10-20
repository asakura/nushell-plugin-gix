//! Fixture repositories whose histories fork, diverge, discard a side or skew commit dates.

use crate::basic::test_repo;
use crate::error::FixtureError;
use crate::repo::{
    COMMITTER_TIME, EDITED_CONTENT, FixtureRepo, HEAD_REF, MAIN_BRANCH, ORIGINAL_CONTENT,
    PATHS_FILE, commit_on_head, committer, init_isolated, point_head_at, tree_with,
};

use gix::objs::{Blob, Tree};

use gix::{create, open};
use rstest::fixture;
use tempfile::TempDir;

/// Subject of the fork point of [`forked_repo`], which is also its oldest commit.
pub const FORK_ROOT_MESSAGE: &str = "root";

/// Subject of the first parent of [`forked_repo`]'s merge; older than the second parent.
pub const FORK_FIRST_PARENT_MESSAGE: &str = "side";

/// Subject of the second parent of [`forked_repo`]'s merge; the newest commit besides `HEAD`.
pub const FORK_SECOND_PARENT_MESSAGE: &str = "branch";

/// Subject of the merge commit that [`forked_repo`] makes `HEAD`; the newest commit.
pub const FORK_MERGE_MESSAGE: &str = "merge";

/// Timestamp of [`forked_repo`]'s first parent (the root carries [`COMMITTER_TIME`]).
pub const FORK_FIRST_PARENT_TIME: &str = "1700000100 +0000";

/// Timestamp of [`forked_repo`]'s second parent, in a `+02:00` offset.
pub const FORK_SECOND_PARENT_TIME: &str = "1700000200 +0200";

/// Timestamp of [`forked_repo`]'s merge commit, in a `-05:00` offset.
pub const FORK_MERGE_TIME: &str = "1700000300 -0500";

/// Full name of the branch [`diverged_repo`] adds beside `HEAD`'s history; `HEAD` cannot reach its commit.
pub const OTHER_BRANCH_REF: &str = "refs/heads/other";

/// Subject of the only commit on [`OTHER_BRANCH_REF`].
pub const OTHER_COMMIT_MESSAGE: &str = "other";

/// Timestamp of the commit on [`OTHER_BRANCH_REF`], newer than every commit of [`test_repo`].
pub const OTHER_COMMIT_TIME: &str = "1700000400 +0000";

/// Top-level file that [`discarded_side_repo`] adds beside [`PATHS_FILE`] and that only its first parent edits.
pub const UNRELATED_FILE: &str = "other.txt";

/// Subject of the root of [`discarded_side_repo`], which adds [`PATHS_FILE`] and [`UNRELATED_FILE`].
pub const DISCARDED_ROOT_MESSAGE: &str = "discard root";

/// Subject of the first parent of [`discarded_side_repo`]'s merge, which changes only [`UNRELATED_FILE`].
pub const DISCARDED_FIRST_PARENT_MESSAGE: &str = "first parent";

/// Subject of the second parent of [`discarded_side_repo`]'s merge, which changes [`PATHS_FILE`] and is discarded.
pub const DISCARDED_SIDE_MESSAGE: &str = "discarded side";

/// Subject of the merge that [`discarded_side_repo`] makes `HEAD`, whose tree equals its first parent's.
pub const DISCARDED_MERGE_MESSAGE: &str = "discard merge";

/// Subject of the root of [`second_parent_wins_repo`], which adds [`PATHS_FILE`].
pub const SECOND_WINS_ROOT_MESSAGE: &str = "second wins root";

/// Subject of the first parent of [`second_parent_wins_repo`]'s merge, which edits [`PATHS_FILE`] to [`EDITED_CONTENT`].
pub const SECOND_WINS_FIRST_PARENT_MESSAGE: &str = "second wins first parent";

/// Subject of the second parent of [`second_parent_wins_repo`]'s merge, which edits [`PATHS_FILE`] to [`THIRD_CONTENT`].
pub const SECOND_WINS_SECOND_PARENT_MESSAGE: &str = "second wins second parent";

/// Subject of the merge that [`second_parent_wins_repo`] makes `HEAD`, whose [`PATHS_FILE`] equals its second parent's.
pub const SECOND_WINS_MERGE_MESSAGE: &str = "second wins merge";

/// Subject of the root of [`skewed_repo`], the oldest commit.
pub const SKEW_ROOT_MESSAGE: &str = "skew root";

/// Subject of the parent of [`skewed_repo`]'s child and side, which edits [`PATHS_FILE`] and is newer than both.
pub const SKEW_PARENT_MESSAGE: &str = "skew parent";

/// Subject of the child of [`skewed_repo`]'s parent that the merge takes as its first parent.
pub const SKEW_CHILD_MESSAGE: &str = "skew child";

/// Subject of the side branch of [`skewed_repo`], the second parent of the merge.
pub const SKEW_SIDE_MESSAGE: &str = "skew side";

/// Subject of the merge that [`skewed_repo`] makes `HEAD`, whose tree equals its first parent's.
pub const SKEW_MERGE_MESSAGE: &str = "skew merge";

/// Timestamp of the first parent of [`discarded_side_repo`], one minute after its root (which carries [`COMMITTER_TIME`]).
const DISCARDED_FIRST_PARENT_TIME: &str = "1700000060 +0000";

/// Timestamp of the second parent of [`discarded_side_repo`], one minute after its first parent.
const DISCARDED_SIDE_TIME: &str = "1700000120 +0000";

/// Timestamp of the merge of [`discarded_side_repo`], the newest commit.
const DISCARDED_MERGE_TIME: &str = "1700000180 +0000";

/// Timestamp of the first parent of [`second_parent_wins_repo`], one minute after its root (which carries [`COMMITTER_TIME`]).
const SECOND_WINS_FIRST_PARENT_TIME: &str = "1700000060 +0000";

/// Timestamp of the second parent of [`second_parent_wins_repo`], one minute after its first parent.
const SECOND_WINS_SECOND_PARENT_TIME: &str = "1700000120 +0000";

/// Timestamp of the merge of [`second_parent_wins_repo`], the newest commit.
const SECOND_WINS_MERGE_TIME: &str = "1700000180 +0000";

/// Timestamp of the root of [`skewed_repo`], the oldest commit.
const SKEW_ROOT_TIME: &str = "1700001000 +0000";

/// Timestamp of the parent of [`skewed_repo`], newer than both of its children.
const SKEW_PARENT_TIME: &str = "1700005000 +0000";

/// Timestamp of the child of [`skewed_repo`]'s parent, older than the parent.
const SKEW_CHILD_TIME: &str = "1700002000 +0000";

/// Timestamp of the side branch of [`skewed_repo`], older than the parent.
const SKEW_SIDE_TIME: &str = "1700004000 +0000";

/// Timestamp of the merge of [`skewed_repo`], the newest commit.
const SKEW_MERGE_TIME: &str = "1700006000 +0000";

/// Content that the second parent and the merge of [`second_parent_wins_repo`] write, differing from both other versions.
pub const THIRD_CONTENT: &[u8] = b"three\n";

/// A throwaway repository whose `HEAD` is a merge of two commits that fork from one root, each with its own commit time.
///
/// Returned by [`forked_repo`]. The first parent is older than the second, and the second is the newest commit besides
/// `HEAD`, so newest-first by commit time differs from a first-parent walk (`merge`, `side`, `root`, `branch`).
#[derive(Debug)]
pub struct ForkedTestRepo {
    /// The temporary directory backing the repository (dropped last).
    pub dir: TempDir,
    /// Hex id of the merge commit (`HEAD`), the newest commit.
    pub head: String,
    /// Hex id of the first parent of the merge.
    pub first_parent: String,
    /// Hex id of the second parent of the merge.
    pub second_parent: String,
    /// Hex id of the fork point, the oldest commit.
    pub root: String,
}

impl FixtureRepo for ForkedTestRepo {
    fn dir(&self) -> &TempDir {
        &self.dir
    }
}

/// A throwaway repository like [`test_repo`] plus a second branch whose single root commit `HEAD` cannot reach.
///
/// Returned by [`diverged_repo`]. `head` is the newest commit of `HEAD`'s history, and `other` is the root commit of
/// [`OTHER_BRANCH_REF`].
#[derive(Debug)]
pub struct DivergedTestRepo {
    /// The temporary directory backing the repository (dropped last).
    pub dir: TempDir,
    /// Hex id of the commit `HEAD` names.
    pub head: String,
    /// Hex id of the commit on [`OTHER_BRANCH_REF`], which `HEAD` cannot reach.
    pub other: String,
}

impl FixtureRepo for DivergedTestRepo {
    fn dir(&self) -> &TempDir {
        &self.dir
    }
}

/// A throwaway repository whose merge discards the change one parent makes to [`PATHS_FILE`].
///
/// Returned by [`discarded_side_repo`]. The root adds [`PATHS_FILE`] and [`UNRELATED_FILE`]. The first parent of the
/// merge changes only [`UNRELATED_FILE`], and the second parent changes only [`PATHS_FILE`]; the merge's tree equals its
/// first parent's. Newest first by commit time the walk visits the merge, the second parent, the first parent, then the
/// root.
#[derive(Debug)]
pub struct DiscardedSideTestRepo {
    /// The temporary directory backing the repository (dropped last).
    pub dir: TempDir,
    /// Id of the root commit, the oldest commit.
    pub root: gix::ObjectId,
    /// Id of the first parent of the merge.
    pub first_parent: gix::ObjectId,
    /// Id of the second parent of the merge, whose change to [`PATHS_FILE`] the merge discards.
    pub side: gix::ObjectId,
    /// Id of `HEAD`, the merge.
    pub merge: gix::ObjectId,
}

impl FixtureRepo for DiscardedSideTestRepo {
    fn dir(&self) -> &TempDir {
        &self.dir
    }
}

/// A throwaway repository whose merge equals its second parent at [`PATHS_FILE`] and differs from its first parent there.
///
/// Returned by [`second_parent_wins_repo`]. The root holds [`ORIGINAL_CONTENT`] at [`PATHS_FILE`], the first parent
/// [`EDITED_CONTENT`], and the second parent and the merge (`HEAD`) [`THIRD_CONTENT`]. Only the first parent is compared
/// under `--first-parent`, so the merge is listed; the default walk follows the second parent, which is identical to it.
#[derive(Debug)]
pub struct SecondParentWinsTestRepo {
    /// The temporary directory backing the repository (dropped last).
    pub dir: TempDir,
    /// Id of the root commit, the oldest commit.
    pub root: gix::ObjectId,
    /// Id of the first parent of the merge, which differs from the merge at [`PATHS_FILE`].
    pub first_parent: gix::ObjectId,
    /// Id of the second parent of the merge, whose tree at [`PATHS_FILE`] equals the merge's.
    pub second_parent: gix::ObjectId,
    /// Id of `HEAD`, the merge.
    pub merge: gix::ObjectId,
}

impl FixtureRepo for SecondParentWinsTestRepo {
    fn dir(&self) -> &TempDir {
        &self.dir
    }
}

/// A throwaway repository whose parent is newer than both of its children, so a walk by commit time pops it first.
///
/// Returned by [`skewed_repo`]. The root adds [`PATHS_FILE`] and [`UNRELATED_FILE`]; the parent edits [`PATHS_FILE`]; the
/// child and the side each edit only [`UNRELATED_FILE`]; the merge `HEAD` has the child's tree. Commit time rises from the
/// root to the parent, which is newer than the child and the side that follow it.
#[derive(Debug)]
pub struct SkewedTestRepo {
    /// The temporary directory backing the repository (dropped last).
    pub dir: TempDir,
    /// Id of the root commit, the oldest commit.
    pub root: gix::ObjectId,
    /// Id of the parent of the child and the side, newer than both.
    pub parent: gix::ObjectId,
    /// Id of the first parent of the merge, a child of [`SkewedTestRepo::parent`].
    pub child: gix::ObjectId,
    /// Id of the second parent of the merge, a second child of [`SkewedTestRepo::parent`].
    pub side: gix::ObjectId,
    /// Id of `HEAD`, the merge.
    pub merge: gix::ObjectId,
}

impl FixtureRepo for SkewedTestRepo {
    fn dir(&self) -> &TempDir {
        &self.dir
    }
}

/// Builds the forked fixture repository: a root, two commits on it with distinct commit times, and a merge `HEAD`.
///
/// The first parent is committed through `HEAD`, which the merge then requires to name it. The second parent is
/// written without a reference, so the merge can be committed on top of both.
#[fixture]
pub fn forked_repo() -> Result<ForkedTestRepo, FixtureError> {
    let dir = TempDir::new()?;
    let repo = init_isolated(dir.path(), create::Kind::WithWorktree)?;
    point_head_at(&repo, MAIN_BRANCH)?;

    let tree = repo.write_object(Tree::empty())?.detach();

    let root = repo.commit_as(
        committer(COMMITTER_TIME),
        committer(COMMITTER_TIME),
        HEAD_REF,
        FORK_ROOT_MESSAGE,
        tree,
        Vec::<gix::ObjectId>::new(),
    )?;

    let first_parent = repo.commit_as(
        committer(FORK_FIRST_PARENT_TIME),
        committer(FORK_FIRST_PARENT_TIME),
        HEAD_REF,
        FORK_FIRST_PARENT_MESSAGE,
        tree,
        vec![root.detach()],
    )?;

    let second_parent = repo
        .new_commit_as(
            committer(FORK_SECOND_PARENT_TIME),
            committer(FORK_SECOND_PARENT_TIME),
            FORK_SECOND_PARENT_MESSAGE,
            tree,
            vec![root.detach()],
        )?
        .id()
        .detach();

    let head = repo.commit_as(
        committer(FORK_MERGE_TIME),
        committer(FORK_MERGE_TIME),
        HEAD_REF,
        FORK_MERGE_MESSAGE,
        tree,
        vec![first_parent.detach(), second_parent],
    )?;

    Ok(ForkedTestRepo {
        dir,
        head: head.detach().to_string(),
        first_parent: first_parent.detach().to_string(),
        second_parent: second_parent.to_string(),
        root: root.detach().to_string(),
    })
}

/// Builds a repository like [`test_repo`] plus [`OTHER_BRANCH_REF`], whose single root commit `HEAD` cannot reach.
#[fixture]
pub fn diverged_repo() -> Result<DivergedTestRepo, FixtureError> {
    let base = test_repo()?;
    let repo = gix::open_opts(base.path(), open::Options::isolated())?;
    let tree = repo.write_object(Tree::empty())?.detach();

    let other = repo.commit_as(
        committer(OTHER_COMMIT_TIME),
        committer(OTHER_COMMIT_TIME),
        OTHER_BRANCH_REF,
        OTHER_COMMIT_MESSAGE,
        tree,
        Vec::<gix::ObjectId>::new(),
    )?;

    Ok(DivergedTestRepo {
        dir: base.dir,
        head: base.head,
        other: other.detach().to_string(),
    })
}

/// Builds a merge that discards a side branch's change to [`PATHS_FILE`]: a root, a first parent that changes only
/// [`UNRELATED_FILE`], a newer second parent that changes [`PATHS_FILE`], and a merge `HEAD` whose tree equals the first
/// parent's.
///
/// The trees are written directly, not checked out. Commit times rise from the root to the merge.
#[fixture]
pub fn discarded_side_repo() -> Result<DiscardedSideTestRepo, FixtureError> {
    let dir = TempDir::new()?;
    let repo = init_isolated(dir.path(), create::Kind::WithWorktree)?;

    point_head_at(&repo, MAIN_BRANCH)?;

    let original = repo
        .write_object(&Blob {
            data: ORIGINAL_CONTENT.to_vec(),
        })?
        .detach();

    let edited = repo
        .write_object(&Blob {
            data: EDITED_CONTENT.to_vec(),
        })?
        .detach();

    let root_tree = tree_with(&repo, &[(PATHS_FILE, original), (UNRELATED_FILE, original)])?;
    let first_parent_tree = tree_with(&repo, &[(PATHS_FILE, original), (UNRELATED_FILE, edited)])?;
    let side_tree = tree_with(&repo, &[(PATHS_FILE, edited), (UNRELATED_FILE, original)])?;

    let root = commit_on_head(
        &repo,
        DISCARDED_ROOT_MESSAGE,
        root_tree,
        COMMITTER_TIME,
        None,
    )?;

    let first_parent = commit_on_head(
        &repo,
        DISCARDED_FIRST_PARENT_MESSAGE,
        first_parent_tree,
        DISCARDED_FIRST_PARENT_TIME,
        Some(root),
    )?;

    let side = repo
        .new_commit_as(
            committer(DISCARDED_SIDE_TIME),
            committer(DISCARDED_SIDE_TIME),
            DISCARDED_SIDE_MESSAGE,
            side_tree,
            vec![root],
        )?
        .id()
        .detach();

    let merge = repo
        .commit_as(
            committer(DISCARDED_MERGE_TIME),
            committer(DISCARDED_MERGE_TIME),
            HEAD_REF,
            DISCARDED_MERGE_MESSAGE,
            first_parent_tree,
            vec![first_parent, side],
        )?
        .detach();

    Ok(DiscardedSideTestRepo {
        dir,
        root,
        first_parent,
        side,
        merge,
    })
}

/// Builds a merge that matches its second parent, not its first, at [`PATHS_FILE`]: a root, a first parent that edits
/// [`PATHS_FILE`], a newer second parent that edits it again, and a merge `HEAD` whose tree equals the second parent's.
///
/// The trees are written directly, not checked out. Commit times rise from the root to the merge.
#[fixture]
pub fn second_parent_wins_repo() -> Result<SecondParentWinsTestRepo, FixtureError> {
    let dir = TempDir::new()?;
    let repo = init_isolated(dir.path(), create::Kind::WithWorktree)?;

    point_head_at(&repo, MAIN_BRANCH)?;

    let original = repo
        .write_object(&Blob {
            data: ORIGINAL_CONTENT.to_vec(),
        })?
        .detach();

    let edited = repo
        .write_object(&Blob {
            data: EDITED_CONTENT.to_vec(),
        })?
        .detach();

    let third = repo
        .write_object(&Blob {
            data: THIRD_CONTENT.to_vec(),
        })?
        .detach();

    let root_tree = tree_with(&repo, &[(PATHS_FILE, original)])?;
    let first_parent_tree = tree_with(&repo, &[(PATHS_FILE, edited)])?;
    let second_parent_tree = tree_with(&repo, &[(PATHS_FILE, third)])?;

    let root = commit_on_head(
        &repo,
        SECOND_WINS_ROOT_MESSAGE,
        root_tree,
        COMMITTER_TIME,
        None,
    )?;

    let first_parent = commit_on_head(
        &repo,
        SECOND_WINS_FIRST_PARENT_MESSAGE,
        first_parent_tree,
        SECOND_WINS_FIRST_PARENT_TIME,
        Some(root),
    )?;

    let second_parent = repo
        .new_commit_as(
            committer(SECOND_WINS_SECOND_PARENT_TIME),
            committer(SECOND_WINS_SECOND_PARENT_TIME),
            SECOND_WINS_SECOND_PARENT_MESSAGE,
            second_parent_tree,
            vec![root],
        )?
        .id()
        .detach();

    let merge = repo
        .commit_as(
            committer(SECOND_WINS_MERGE_TIME),
            committer(SECOND_WINS_MERGE_TIME),
            HEAD_REF,
            SECOND_WINS_MERGE_MESSAGE,
            second_parent_tree,
            vec![first_parent, second_parent],
        )?
        .detach();

    Ok(SecondParentWinsTestRepo {
        dir,
        root,
        first_parent,
        second_parent,
        merge,
    })
}

/// Builds a root, a parent newer than its two children, the child and side, and a merge `HEAD` taking the child first.
///
/// The trees are written directly, not checked out.
#[fixture]
pub fn skewed_repo() -> Result<SkewedTestRepo, FixtureError> {
    let dir = TempDir::new()?;
    let repo = init_isolated(dir.path(), create::Kind::WithWorktree)?;

    point_head_at(&repo, MAIN_BRANCH)?;

    let original = repo
        .write_object(&Blob {
            data: ORIGINAL_CONTENT.to_vec(),
        })?
        .detach();

    let edited = repo
        .write_object(&Blob {
            data: EDITED_CONTENT.to_vec(),
        })?
        .detach();

    let third = repo
        .write_object(&Blob {
            data: THIRD_CONTENT.to_vec(),
        })?
        .detach();

    let root_tree = tree_with(&repo, &[(PATHS_FILE, original), (UNRELATED_FILE, original)])?;
    let parent_tree = tree_with(&repo, &[(PATHS_FILE, edited), (UNRELATED_FILE, original)])?;
    let child_tree = tree_with(&repo, &[(PATHS_FILE, edited), (UNRELATED_FILE, edited)])?;
    let side_tree = tree_with(&repo, &[(PATHS_FILE, edited), (UNRELATED_FILE, third)])?;

    let root = commit_on_head(&repo, SKEW_ROOT_MESSAGE, root_tree, SKEW_ROOT_TIME, None)?;

    let parent = commit_on_head(
        &repo,
        SKEW_PARENT_MESSAGE,
        parent_tree,
        SKEW_PARENT_TIME,
        Some(root),
    )?;

    let child = commit_on_head(
        &repo,
        SKEW_CHILD_MESSAGE,
        child_tree,
        SKEW_CHILD_TIME,
        Some(parent),
    )?;

    let side = repo
        .new_commit_as(
            committer(SKEW_SIDE_TIME),
            committer(SKEW_SIDE_TIME),
            SKEW_SIDE_MESSAGE,
            side_tree,
            vec![parent],
        )?
        .id()
        .detach();

    let merge = repo
        .commit_as(
            committer(SKEW_MERGE_TIME),
            committer(SKEW_MERGE_TIME),
            HEAD_REF,
            SKEW_MERGE_MESSAGE,
            child_tree,
            vec![child, side],
        )?
        .detach();

    Ok(SkewedTestRepo {
        dir,
        root,
        parent,
        child,
        side,
        merge,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use rstest::rstest;

    use std::assert_matches;
    use std::{io, path::Path};

    #[derive(thiserror::Error, Debug)]
    enum TestError {
        #[error(transparent)]
        Fixture(#[from] FixtureError),

        #[error(transparent)]
        Gix(#[from] gix::Error),

        #[error(transparent)]
        Io(#[from] io::Error),

        #[error("Test failure: {0}")]
        #[expect(
            dead_code,
            reason = "every test error carries a Failure variant, even where no test builds one"
        )]
        Failure(String),
    }

    /// Opens `dir` with the isolated open options that the fixtures use.
    fn open_isolated(dir: &Path) -> Result<gix::Repository, TestError> {
        let repo = gix::open_opts(dir, open::Options::isolated())?;

        Ok(repo)
    }

    mod diverged_repo_fixture {
        use super::*;

        #[rstest]
        fn other_branch_names_the_other_commit(
            diverged_repo: Result<DivergedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = diverged_repo?;
            let opened = open_isolated(repo.path())?;

            let resolved = opened.rev_parse_single(OTHER_BRANCH_REF)?.to_string();

            assert_matches!(resolved.as_str(), name if name == repo.other);

            Ok(())
        }
    }
}
