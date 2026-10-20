//! Building blocks shared by every fixture repository: the committer identity, isolated initialisation, and the temporary-directory fixtures.

use crate::error::FixtureError;

use gix::actor::SignatureRef;
use gix::discover::upwards::{Error as UpwardsError, Options as UpwardsOptions, TrustPolicy};
use gix::objs::tree::EntryKind;
use gix::refs::Target;
use gix::refs::transaction::{PreviousValue, RefEdit};
use gix::{create, discover, open, sec};
use rstest::fixture;
use tempfile::TempDir;

use std::{fs, io::Write as _, path::Path};

pub const HEAD_REF: &str = "HEAD";

/// Reflog message the fixtures record for their ref edits.
pub const FIXTURE_REFLOG_MESSAGE: &str = "fixture";

/// Name of the fixture committer.
pub const COMMITTER_NAME: &str = "Ada";

/// E-mail address of the fixture committer.
pub const COMMITTER_EMAIL: &str = "ada@example.com";

/// Timestamp of every fixture commit, as a Unix time followed by a UTC offset.
pub const COMMITTER_TIME: &str = "1700000000 +0000";

/// Top-level file that [`paths_repo`](crate::paths_repo) adds and later edits.
pub const PATHS_FILE: &str = "a.txt";

/// Content [`paths_repo`](crate::paths_repo) first writes to each file.
pub const ORIGINAL_CONTENT: &[u8] = b"one\n";

/// Content that the edits of [`paths_repo`](crate::paths_repo) write.
pub const EDITED_CONTENT: &[u8] = b"two\n";

/// Name of a repository's git directory inside its worktree.
pub const GIT_DIR: &str = ".git";

/// Name of the repository configuration file inside the git directory.
pub const CONFIG_FILE: &str = "config";

/// Name of the object database directory inside the git directory.
const OBJECTS_DIR: &str = "objects";

/// Configuration section that names the committer of a repository.
const USER_SECTION: &str = "[user]";

/// Branch that every fixture repository's `HEAD` names; [`test_repo`](crate::test_repo) also commits to it.
pub const MAIN_BRANCH: &str = "refs/heads/main";

/// A fixture repository living in a temporary directory that the fixture keeps alive.
pub trait FixtureRepo {
    /// The temporary directory holding the repository; dropping the fixture deletes it.
    fn dir(&self) -> &TempDir;

    /// The repository's path: its worktree, or the git directory of a bare fixture.
    #[must_use = "returns the path; the repository is unchanged"]
    fn path(&self) -> &Path {
        self.dir().path()
    }
}

/// The fixture committer and author, dated `time` (a raw git timestamp such as [`COMMITTER_TIME`]).
///
/// Every fixture commit needs an identity, since reflogs record one; pass the result to `commit_as` for both author and
/// committer.
#[must_use = "returns the identity; discarding it names no committer"]
pub fn committer(time: &str) -> SignatureRef<'_> {
    SignatureRef {
        name: COMMITTER_NAME.into(),
        email: COMMITTER_EMAIL.into(),
        time,
    }
}

/// Initialises a repository in `dir` with the isolated open options, so the result does not depend
/// on the user's or the system's git configuration.
///
/// Also writes the fixture committer into the repository's configuration, so reflog writes work without any
/// user identity. The returned handle is reopened afterwards, because the configuration snapshot taken by
/// `init_opts` predates that write.
pub fn init_isolated(dir: &Path, kind: create::Kind) -> Result<gix::Repository, FixtureError> {
    let repo = gix::ThreadSafeRepository::init_opts(
        dir,
        kind,
        create::Options::default(),
        open::Options::isolated(),
    )?
    .to_thread_local();

    add_committer_identity(repo.git_dir())?;

    let reopened = gix::ThreadSafeRepository::open_opts(repo.git_dir(), open::Options::isolated())?;

    Ok(reopened.to_thread_local())
}

/// Points `HEAD` of `repo` at the full branch name `branch`, pinning the unborn branch name whatever `init.defaultBranch`
/// says.
pub fn point_head_at(repo: &gix::Repository, branch: &str) -> Result<(), FixtureError> {
    repo.edit_reference(RefEdit::update(
        HEAD_REF.try_into()?,
        Target::Symbolic(branch.try_into()?),
        PreviousValue::Any,
        FIXTURE_REFLOG_MESSAGE,
    ))?;

    Ok(())
}

/// Appends the fixture committer to the configuration in the git directory `git_dir`, keeping the `init` settings.
///
/// Git refuses to write a reflog without a committer, and the isolated options ignore the user's identity, so the
/// repository carries its own. `git_dir` is the repository's git directory (`repo.git_dir()`), which is `dir/.git`
/// for a worktree and `dir` itself for a bare repository.
fn add_committer_identity(git_dir: &Path) -> Result<(), FixtureError> {
    let config = git_dir.join(CONFIG_FILE);
    let mut file = fs::OpenOptions::new().append(true).open(config)?;

    let identity =
        format!("{USER_SECTION}\n\tname = {COMMITTER_NAME}\n\temail = {COMMITTER_EMAIL}\n");

    file.write_all(identity.as_bytes())?;

    Ok(())
}

/// Removes the fixture committer from the worktree repository in `dir`, so that git cannot write a reflog there.
///
/// # Errors
///
/// Returns [`FixtureError::Io`] when the configuration cannot be read or written.
pub fn remove_committer_identity(dir: &Path) -> Result<(), FixtureError> {
    let config = dir.join(GIT_DIR).join(CONFIG_FILE);
    let text = fs::read_to_string(&config)?;

    let kept = text
        .split_once(USER_SECTION)
        .map_or(text.as_str(), |(head, _)| head);

    fs::write(config, kept)?;

    Ok(())
}

/// Deletes the loose object `id` from the worktree repository in `dir`, so that reading it fails.
///
/// # Errors
///
/// Returns [`FixtureError::Io`] when the object file cannot be removed, and [`FixtureError::Failure`] when the
/// id is too short to name a loose object.
pub fn remove_loose_object(dir: &Path, id: gix::ObjectId) -> Result<(), FixtureError> {
    let hex = id.to_hex().to_string();
    let (fan_out, rest) = hex
        .split_at_checked(2)
        .ok_or_else(|| FixtureError::Failure("object id is too short".to_owned()))?;

    fs::remove_file(dir.join(GIT_DIR).join(OBJECTS_DIR).join(fan_out).join(rest))?;

    Ok(())
}

/// Writes a tree holding each `(repository-relative path, blob id)` of `entries`, creating directories on the way.
pub fn tree_with(
    repo: &gix::Repository,
    entries: &[(&str, gix::ObjectId)],
) -> Result<gix::ObjectId, FixtureError> {
    let mut editor = repo.edit_tree(repo.object_hash().empty_tree())?;

    for (path, blob) in entries {
        editor.upsert(*path, EntryKind::Blob, *blob)?;
    }

    Ok(editor.write()?.detach())
}

/// Commits `tree` on `HEAD` as the fixture committer at `time`, on top of `parent` when given.
pub fn commit_on_head(
    repo: &gix::Repository,
    message: &str,
    tree: gix::ObjectId,
    time: &str,
    parent: Option<gix::ObjectId>,
) -> Result<gix::ObjectId, FixtureError> {
    let signature = committer(time);
    let parents: Vec<gix::ObjectId> = parent.into_iter().collect();

    Ok(repo
        .commit_as(signature, signature, HEAD_REF, message, tree, parents)?
        .detach())
}

/// Fails with [`FixtureError::EnclosedInRepository`] when `dir` or any of its ancestors is a git repository.
///
/// Discovery ignores repository ownership and the `GIT_CEILING_DIRECTORIES` environment, so any repository on the
/// path is reported, whoever owns it. Any other discovery failure is returned as [`FixtureError::Gix`].
fn ensure_not_enclosed(dir: &Path) -> Result<(), FixtureError> {
    let options = UpwardsOptions {
        trust: TrustPolicy::Assume(sec::Trust::Full),
        ..UpwardsOptions::default()
    };

    match discover::upwards_opts(dir, options) {
        Ok((path, _trust)) => {
            let (git_dir, _work_tree) = path.into_repository_and_work_tree_directories();

            Err(FixtureError::EnclosedInRepository {
                dir: dir.to_owned(),
                git_dir,
            })
        }
        Err(error) if is_no_repository(&error) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Whether `error` is discovery's own report that no repository was found.
///
/// Under [`TrustPolicy::Assume`] discovery reports no other [`UpwardsError`]: the untrusted-repository variant
/// requires ownership checks. Other failures (I/O, malformed `.git` files) keep their own causes.
fn is_no_repository(error: &gix::Error) -> bool {
    error.downcast_any_ref::<UpwardsError>().is_some()
}

/// A temporary directory that is deliberately not a git repository.
///
/// Guarantees that neither the directory nor any of its ancestors is a repository. `TempDir` honours `TMPDIR`, so
/// when that places the directory inside a checkout the fixture fails with
/// [`FixtureError::EnclosedInRepository`] instead of returning a directory that a repository would enclose.
#[fixture]
pub fn plain_dir() -> Result<TempDir, FixtureError> {
    let dir = TempDir::new()?;
    ensure_not_enclosed(dir.path())?;

    Ok(dir)
}

/// A fresh temporary directory, for tests and fixtures that build a repository inside it.
#[fixture]
pub fn dir() -> Result<TempDir, FixtureError> {
    Ok(TempDir::new()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::basic::{UnbornTestRepo, unborn_repo};

    use rstest::rstest;

    use std::assert_matches;
    use std::io;

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

    mod plain_dir_fixture {
        use super::*;

        #[rstest]
        fn succeeds_outside_any_repository(
            plain_dir: Result<TempDir, FixtureError>,
        ) -> Result<(), TestError> {
            let _dir = plain_dir?;

            Ok(())
        }
    }

    mod ensure_not_enclosed {
        use super::*;

        #[rstest]
        fn rejects_dir_inside_repository(
            unborn_repo: Result<UnbornTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = unborn_repo?;
            let nested = repo.path().join("nested");
            fs::create_dir_all(&nested)?;

            assert_matches!(
                ensure_not_enclosed(&nested),
                Err(FixtureError::EnclosedInRepository { dir, git_dir })
                    if dir == nested && git_dir == repo.path().join(GIT_DIR)
            );

            Ok(())
        }

        #[rstest]
        fn accepts_dir_outside_any_repository(
            plain_dir: Result<TempDir, FixtureError>,
        ) -> Result<(), TestError> {
            let dir = plain_dir?;

            assert_matches!(ensure_not_enclosed(dir.path()), Ok(()));

            Ok(())
        }
    }
}
