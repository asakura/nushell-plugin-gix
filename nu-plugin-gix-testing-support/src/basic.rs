//! Single-purpose fixture repositories: unborn, bare, corrupt, linked-worktree, tagged, binary and the two-commit `test_repo`.

use crate::error::FixtureError;
use crate::repo::{
    COMMITTER_EMAIL, COMMITTER_NAME, COMMITTER_TIME, CONFIG_FILE, FIXTURE_REFLOG_MESSAGE,
    FixtureRepo, GIT_DIR, HEAD_REF, MAIN_BRANCH, commit_on_head, committer, init_isolated,
    point_head_at, tree_with,
};

use gix::create;
use gix::objs::{Blob, Kind, Tag, Tree};
use gix::refs::Target;
use gix::refs::transaction::{PreviousValue, RefEdit};
use rstest::fixture;
use tempfile::TempDir;

use std::{fs, path::PathBuf};

/// Subject of the first commit made by [`test_repo`].
pub const FIRST_COMMIT_MESSAGE: &str = "first";

/// Subject of the second commit made by [`test_repo`], which becomes `HEAD`.
pub const SECOND_COMMIT_MESSAGE: &str = "second";

/// Name a mailmap maps the fixture committer to.
pub const MAILMAP_NAME: &str = "Ada Lovelace";

/// E-mail address a mailmap maps the fixture committer to.
pub const MAILMAP_EMAIL: &str = "ada@lovelace.org";

/// Worktree file that git reads as the mailmap.
const MAILMAP_FILE: &str = ".mailmap";

/// Name of the file inside a git directory that records what `HEAD` names.
pub const HEAD_FILE: &str = "HEAD";

/// Repository configuration that cannot be parsed: the `[core]` section header is never closed.
const CORRUPT_CONFIG: &str = "[core\n";

/// Directory inside the main git directory that holds one private git directory per linked worktree.
const WORKTREES_DIR: &str = "worktrees";

/// Name of the directory the linked worktree is created in, next to the main worktree.
const LINKED_WORKTREE_NAME: &str = "linked";

/// File in a linked worktree's git directory that points back at the main git directory.
const COMMON_DIR_FILE: &str = "commondir";

/// Contents of [`COMMON_DIR_FILE`]: the main git directory, relative to the linked git directory.
const COMMON_DIR_TARGET: &str = "../..";

/// File in a linked worktree's git directory that records the path of the worktree's `.git` file.
const GITDIR_FILE: &str = "gitdir";

/// Prefix of a `.git` file that points at the git directory of a linked worktree.
const GITDIR_POINTER_PREFIX: &str = "gitdir: ";

/// Prefix of a `HEAD` file that names the branch it points at.
const SYMBOLIC_REF_PREFIX: &str = "ref: ";

/// Full name of the branch that [`tagged_repo`]'s `HEAD` names; it holds the fixture's commit.
pub const FEATURE_BRANCH_REF: &str = "refs/heads/feature";

/// Short name of [`FEATURE_BRANCH_REF`], as a revision expression spells it.
pub const FEATURE_BRANCH: &str = "feature";

/// Full name of the lightweight tag that [`tagged_repo`] points at its commit.
pub const LIGHT_TAG_REF: &str = "refs/tags/light";

/// Full name of the annotated tag that [`tagged_repo`] points at its tag object.
pub const ANNOTATED_TAG_REF: &str = "refs/tags/annotated";

/// Short name of the annotated tag, as a revision expression spells it.
pub const ANNOTATED_TAG: &str = "annotated";

/// Message of the annotated tag object that [`tagged_repo`] writes.
pub const ANNOTATED_TAG_MESSAGE: &str = "release";

/// Subject of the commit that [`tagged_repo`] makes.
pub const INITIAL_MESSAGE: &str = "initial";

/// Content of the blob that [`tagged_repo`] writes.
pub const BLOB_CONTENT: &[u8] = b"blob content";

/// A throwaway repository whose `HEAD` names [`MAIN_BRANCH`] without commits.
///
/// Returned by [`unborn_repo`] (a repository with a worktree) and by [`bare_repo`] (a bare repository, whose `dir` is
/// its git directory).
#[derive(Debug)]
pub struct UnbornTestRepo {
    /// The temporary directory backing the repository; dropping it deletes the repository.
    pub dir: TempDir,
}

impl FixtureRepo for UnbornTestRepo {
    fn dir(&self) -> &TempDir {
        &self.dir
    }
}

/// A throwaway repository whose `.git/config` is corrupt, so that opening it fails.
///
/// Returned by [`corrupt_repo`]; `dir` is the worktree root.
#[derive(Debug)]
pub struct CorruptTestRepo {
    /// The temporary directory backing the repository; dropping it deletes the repository.
    pub dir: TempDir,
}

impl FixtureRepo for CorruptTestRepo {
    fn dir(&self) -> &TempDir {
        &self.dir
    }
}

/// A throwaway repository with a linked worktree, laid out as `git worktree add` lays it out.
///
/// Returned by [`linked_repo`]. The worktree directory can be deleted while its private git directory under the main
/// repository remains.
#[derive(Debug)]
pub struct LinkedTestRepo {
    /// The temporary directory backing the main repository and the linked worktree (dropped last).
    pub dir: TempDir,
    /// The absolute path of the linked worktree's root, inside `dir`.
    pub worktree: PathBuf,
}

/// A throwaway repository in a temporary directory with two commits on `refs/heads/main`.
///
/// Keeps the `TempDir` alive; pass [`FixtureRepo::path`] to `gix open --repo`.
#[derive(Debug)]
pub struct TestRepo {
    /// The temporary directory backing the repository (dropped last).
    pub dir: TempDir,
    /// Hex id of the newest commit (`HEAD`).
    pub head: String,
    /// Hex id of its parent commit.
    pub parent: String,
}

impl FixtureRepo for TestRepo {
    fn dir(&self) -> &TempDir {
        &self.dir
    }
}

/// A throwaway repository whose `HEAD` names [`FEATURE_BRANCH_REF`], with one commit, a lightweight tag, an annotated
/// tag, and a blob.
///
/// Returned by [`tagged_repo`]. The commit is the target of the branch, of the lightweight tag, and of the annotated
/// tag object, so each name resolves to a known object. The repository's configuration names [`COMMITTER_NAME`], so
/// reflog entries written through it have an identity.
#[derive(Debug)]
pub struct TaggedTestRepo {
    /// The temporary directory backing the repository; dropping it deletes the repository.
    pub dir: TempDir,
    /// Id of the commit that `HEAD`, the branch, and the lightweight tag all name.
    pub commit: gix::ObjectId,
    /// Id of the empty tree that the commit points at.
    pub tree: gix::ObjectId,
    /// Id of the annotated tag object, which points at the commit.
    pub tag: gix::ObjectId,
    /// Id of the blob with content [`BLOB_CONTENT`].
    pub blob: gix::ObjectId,
}

impl FixtureRepo for TaggedTestRepo {
    fn dir(&self) -> &TempDir {
        &self.dir
    }
}

/// A repository whose `HEAD` names [`MAIN_BRANCH`] (`refs/heads/main`) without commits.
///
/// Ignores the user's and the system's git configuration (including `init.defaultBranch` and the
/// `GIT_CONFIG_*` environment), so the branch name is always `main`.
#[fixture]
pub fn unborn_repo() -> Result<UnbornTestRepo, FixtureError> {
    let dir = TempDir::new()?;
    let repo = init_isolated(dir.path(), create::Kind::WithWorktree)?;

    point_head_at(&repo, MAIN_BRANCH)?;

    Ok(UnbornTestRepo { dir })
}

/// A bare repository, whose directory is its git directory and which has no worktree.
///
/// Returned as [`UnbornTestRepo`] because its `HEAD` is unborn as well: it names [`MAIN_BRANCH`] without commits,
/// like [`unborn_repo`]. It ignores the user's and the system's git configuration.
#[fixture]
pub fn bare_repo() -> Result<UnbornTestRepo, FixtureError> {
    let dir = TempDir::new()?;
    let repo = init_isolated(dir.path(), create::Kind::Bare)?;

    point_head_at(&repo, MAIN_BRANCH)?;

    Ok(UnbornTestRepo { dir })
}

/// A repository like [`unborn_repo`] whose `.git/config` cannot be parsed, so opening it fails.
#[fixture]
pub fn corrupt_repo() -> Result<CorruptTestRepo, FixtureError> {
    let repo = unborn_repo()?;

    fs::write(repo.path().join(GIT_DIR).join(CONFIG_FILE), CORRUPT_CONFIG)?;

    Ok(CorruptTestRepo { dir: repo.dir })
}

/// A main repository like [`unborn_repo`] plus one linked worktree whose `HEAD` names [`MAIN_BRANCH`].
#[fixture]
pub fn linked_repo() -> Result<LinkedTestRepo, FixtureError> {
    let dir = unborn_repo()?.dir;
    let worktree = dir.path().join(LINKED_WORKTREE_NAME);

    let git_dir = dir
        .path()
        .join(GIT_DIR)
        .join(WORKTREES_DIR)
        .join(LINKED_WORKTREE_NAME);

    fs::create_dir_all(&worktree)?;
    fs::create_dir_all(&git_dir)?;

    fs::write(
        git_dir.join(COMMON_DIR_FILE),
        format!("{COMMON_DIR_TARGET}\n"),
    )?;

    fs::write(
        git_dir.join(GITDIR_FILE),
        format!("{}\n", worktree.join(GIT_DIR).display()),
    )?;

    fs::write(
        git_dir.join(HEAD_FILE),
        format!("{SYMBOLIC_REF_PREFIX}{MAIN_BRANCH}\n"),
    )?;

    fs::write(
        worktree.join(GIT_DIR),
        format!("{GITDIR_POINTER_PREFIX}{}\n", git_dir.display()),
    )?;

    Ok(LinkedTestRepo { dir, worktree })
}

/// Builds the two-commit fixture repository.
#[fixture]
pub fn test_repo() -> Result<TestRepo, FixtureError> {
    let dir = TempDir::new()?;
    let repo = init_isolated(dir.path(), create::Kind::WithWorktree)?;
    point_head_at(&repo, MAIN_BRANCH)?;

    // `empty_tree()` is virtual and never reaches the object database, so write it once.
    let tree = repo.write_object(Tree::empty())?.detach();
    let signature = committer(COMMITTER_TIME);

    let first = repo.commit_as(
        signature,
        signature,
        HEAD_REF,
        FIRST_COMMIT_MESSAGE,
        tree,
        Vec::<gix::ObjectId>::new(),
    )?;

    let second = repo.commit_as(
        signature,
        signature,
        HEAD_REF,
        SECOND_COMMIT_MESSAGE,
        tree,
        vec![first.detach()],
    )?;

    Ok(TestRepo {
        dir,
        head: second.detach().to_string(),
        parent: first.detach().to_string(),
    })
}

/// Builds the [`test_repo`] fixture with a worktree `.mailmap` that maps the fixture committer to
/// [`MAILMAP_NAME`] and [`MAILMAP_EMAIL`].
#[fixture]
pub fn mailmap_repo() -> Result<TestRepo, FixtureError> {
    let repo = test_repo()?;

    fs::write(
        repo.path().join(MAILMAP_FILE),
        format!("{MAILMAP_NAME} <{MAILMAP_EMAIL}> {COMMITTER_NAME} <{COMMITTER_EMAIL}>\n"),
    )?;

    Ok(repo)
}

/// Builds the tagged fixture repository: a commit on [`FEATURE_BRANCH_REF`], which `HEAD` names, plus a lightweight tag,
/// an annotated tag, and a blob.
#[fixture]
pub fn tagged_repo() -> Result<TaggedTestRepo, FixtureError> {
    let dir = TempDir::new()?;
    let repo = init_isolated(dir.path(), create::Kind::WithWorktree)?;
    point_head_at(&repo, FEATURE_BRANCH_REF)?;

    // `empty_tree()` is virtual and never reaches the object database, so write it once. The
    // `Id::shorten` of a revision needs to read it back.
    let tree = repo.write_object(Tree::empty())?.detach();
    let signature = committer(COMMITTER_TIME);

    let commit = repo
        .commit_as(
            signature,
            signature,
            HEAD_REF,
            INITIAL_MESSAGE,
            tree,
            Vec::<gix::ObjectId>::new(),
        )?
        .detach();

    repo.edit_reference(RefEdit::update(
        LIGHT_TAG_REF.try_into()?,
        Target::Object(commit),
        PreviousValue::MustNotExist,
        FIXTURE_REFLOG_MESSAGE,
    ))?;

    let tag = Tag {
        target: commit,
        target_kind: Kind::Commit,
        name: ANNOTATED_TAG.into(),
        tagger: Some(signature.into()),
        message: ANNOTATED_TAG_MESSAGE.into(),
        signature: None,
    };
    let tag_id = repo.write_object(&tag)?.detach();

    repo.edit_reference(RefEdit::update(
        ANNOTATED_TAG_REF.try_into()?,
        Target::Object(tag_id),
        PreviousValue::MustNotExist,
        FIXTURE_REFLOG_MESSAGE,
    ))?;

    let blob = repo
        .write_object(&Blob {
            data: BLOB_CONTENT.to_vec(),
        })?
        .detach();

    Ok(TaggedTestRepo {
        dir,
        commit,
        tree,
        tag: tag_id,
        blob,
    })
}

/// Path of the file that [`binary_repo`] commits.
pub const BINARY_FILE: &str = "bin.dat";

/// Content of [`BINARY_FILE`]; the NUL byte makes git treat the file as binary.
pub const BINARY_CONTENT: &[u8] = b"\0binary\n";

/// Subject of the commit that [`binary_repo`] makes.
const BINARY_MESSAGE: &str = "add binary";

/// A throwaway repository whose only commit adds [`BINARY_FILE`], a file with a NUL byte.
///
/// Returned by [`binary_repo`]; `commit` names that root commit.
#[derive(Debug)]
pub struct BinaryTestRepo {
    /// The temporary directory backing the repository (dropped last).
    pub dir: TempDir,
    /// Id of the root commit, which adds [`BINARY_FILE`].
    pub commit: gix::ObjectId,
}

impl FixtureRepo for BinaryTestRepo {
    fn dir(&self) -> &TempDir {
        &self.dir
    }
}

/// Builds the binary fixture repository: a root commit on `HEAD` that adds [`BINARY_FILE`].
#[fixture]
pub fn binary_repo() -> Result<BinaryTestRepo, FixtureError> {
    let dir = TempDir::new()?;
    let repo = init_isolated(dir.path(), create::Kind::WithWorktree)?;

    point_head_at(&repo, MAIN_BRANCH)?;

    let blob = repo
        .write_object(&Blob {
            data: BINARY_CONTENT.to_vec(),
        })?
        .detach();
    let tree = tree_with(&repo, &[(BINARY_FILE, blob)])?;
    let commit = commit_on_head(&repo, BINARY_MESSAGE, tree, COMMITTER_TIME, None)?;

    Ok(BinaryTestRepo { dir, commit })
}

#[cfg(test)]
mod tests {
    use super::*;

    use gix::open;
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
        Failure(String),
    }

    /// Opens `dir` with the isolated open options that the fixtures use.
    fn open_isolated(dir: &Path) -> Result<gix::Repository, TestError> {
        let repo = gix::open_opts(dir, open::Options::isolated())?;

        Ok(repo)
    }

    /// Full name of the branch that `HEAD` symbolically targets, read with isolated open options.
    fn head_branch(dir: &Path) -> Result<String, TestError> {
        let repo = open_isolated(dir)?;
        let Some(name) = repo.head_name()? else {
            return Err(TestError::Failure("HEAD is detached".to_owned()));
        };
        Ok(name.as_bstr().to_string())
    }

    fn assert_head_targets_main(dir: &Path) -> Result<(), TestError> {
        let branch = head_branch(dir)?;
        if branch == MAIN_BRANCH {
            Ok(())
        } else {
            Err(TestError::Failure(format!(
                "HEAD targets `{branch}`, expected `{MAIN_BRANCH}`"
            )))
        }
    }

    mod unborn {
        use super::*;

        #[rstest]
        fn head_targets_main_branch(
            unborn_repo: Result<UnbornTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = unborn_repo?;

            assert_head_targets_main(repo.path())
        }

        #[rstest]
        fn head_has_no_commit(
            unborn_repo: Result<UnbornTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = unborn_repo?;
            let opened = open_isolated(repo.path())?;

            assert_matches!(opened.head_commit(), Err(_));

            Ok(())
        }
    }

    mod bare {
        use super::*;

        #[rstest]
        fn head_targets_main_branch(
            bare_repo: Result<UnbornTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = bare_repo?;

            assert_head_targets_main(repo.path())
        }

        #[rstest]
        fn opens_as_bare_repository(
            bare_repo: Result<UnbornTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = bare_repo?;
            let opened = open_isolated(repo.path())?;

            assert_matches!(opened.is_bare(), true);

            Ok(())
        }
    }

    mod corrupt {
        use super::*;

        #[rstest]
        fn fails_to_open(
            corrupt_repo: Result<CorruptTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = corrupt_repo?;

            assert_matches!(gix::open(repo.path()), Err(_));

            Ok(())
        }
    }

    mod test_repo_fixture {
        use super::*;

        #[rstest]
        fn head_targets_main_branch(
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = test_repo?;

            assert_head_targets_main(repo.path())
        }

        #[rstest]
        fn head_commit_has_fixture_parent(
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = test_repo?;
            let opened = open_isolated(repo.path())?;
            let head = opened.head_commit()?;
            let parents: Vec<String> = head.parent_ids().map(|id| id.to_string()).collect();

            assert_matches!(parents.as_slice(), [parent] if *parent == repo.parent);

            Ok(())
        }

        #[rstest]
        fn head_commit_message_is_second(
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = test_repo?;
            let opened = open_isolated(repo.path())?;
            let head = opened.head_commit()?;

            let title = head.message()?.title.to_string();

            assert_matches!(title.as_str(), SECOND_COMMIT_MESSAGE);

            Ok(())
        }

        #[rstest]
        fn parent_commit_message_is_first(
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = test_repo?;
            let opened = open_isolated(repo.path())?;
            let head = opened.head_commit()?;
            let parent_id = head
                .parent_ids()
                .next()
                .ok_or_else(|| TestError::Failure("head commit has no parent".to_owned()))?;
            let parent = parent_id.object()?.try_into_commit()?;

            let title = parent.message()?.title.to_string();

            assert_matches!(title.as_str(), FIRST_COMMIT_MESSAGE);

            Ok(())
        }
    }
}
