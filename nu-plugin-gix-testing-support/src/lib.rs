//! Shared test fixtures for `nushell-plugin-gix`: throwaway git repositories built with `gix`,
//! and the in-process plugin-engine helpers (`plugin_test_for`, `custom_value_input`, `pin_pwd`)
//! used by plugin tests.

use gix::actor::SignatureRef;
use gix::discover::upwards::{Error as UpwardsError, Options as UpwardsOptions, TrustPolicy};
use gix::objs::Tree;
use gix::refs::{
    Target,
    name::Error as RefsNameError,
    transaction::{PreviousValue, RefEdit},
};
use gix::{create, discover, open, sec};
use nu_cmd_lang::Describe;
use nu_plugin::Plugin;
use nu_plugin_test_support::PluginTest;
use nu_protocol::{CustomValue, IntoPipelineData as _, PipelineData, Span, Value};
use rstest::fixture;
use tempfile::TempDir;

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Name of the environment variable that holds the engine's working directory.
const PWD_VAR: &str = "PWD";

const HEAD_REF: &str = "HEAD";

/// Reflog message the fixtures record for their ref edits.
pub const FIXTURE_REFLOG_MESSAGE: &str = "fixture";

/// Subject of the first commit made by [`test_repo`].
pub const FIRST_COMMIT_MESSAGE: &str = "first";

/// Subject of the second commit made by [`test_repo`], which becomes `HEAD`.
pub const SECOND_COMMIT_MESSAGE: &str = "second";

/// Name of the fixture committer.
pub const COMMITTER_NAME: &str = "Ada";

/// E-mail address of the fixture committer.
pub const COMMITTER_EMAIL: &str = "ada@example.com";

/// Timestamp of every fixture commit, as a Unix time followed by a UTC offset.
pub const COMMITTER_TIME: &str = "1700000000 +0000";

/// Name of a repository's git directory inside its worktree.
pub const GIT_DIR: &str = ".git";

/// Name of the file inside a git directory that records what `HEAD` names.
pub const HEAD_FILE: &str = "HEAD";

/// Name of the repository configuration file inside the git directory.
const CONFIG_FILE: &str = "config";

/// Repository configuration that cannot be parsed: the `[core]` section header is never closed.
const CORRUPT_CONFIG: &str = "[core\n";

/// Branch that every fixture repository's `HEAD` names; [`test_repo`] also commits to it.
pub const MAIN_BRANCH: &str = "refs/heads/main";

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

/// Errors that shared fixture setup can raise.
#[derive(Debug, thiserror::Error)]
pub enum FixtureError {
    /// A git operation in the fixture repository failed.
    #[error(transparent)]
    Gix(#[from] gix::Error),

    /// A reference name in the fixture was invalid (fixture bug, not test subject).
    #[error(transparent)]
    Name(#[from] RefsNameError),

    /// A temporary directory could not be created.
    #[error(transparent)]
    Io(#[from] io::Error),

    /// The temporary directory lies inside an existing git repository, so it is not a "plain" directory.
    #[error(
        "temporary directory `{}` lies inside the git repository `{}`; set TMPDIR to a directory outside any checkout",
        .dir.display(),
        .git_dir.display()
    )]
    EnclosedInRepository {
        /// The directory that was checked.
        dir: PathBuf,
        /// The git directory of the repository that encloses `dir`.
        git_dir: PathBuf,
    },

    /// An assertion about fixture setup failed.
    #[error("Fixture failure: {0}")]
    Failure(String),
}

/// A throwaway repository whose `HEAD` names [`MAIN_BRANCH`] without commits.
///
/// Returned by [`unborn_repo`] (a repository with a worktree) and by [`bare_repo`] (a bare repository, whose `dir` is
/// its git directory).
#[derive(Debug)]
pub struct UnbornTestRepo {
    /// The temporary directory backing the repository; dropping it deletes the repository.
    pub dir: TempDir,
}

impl UnbornTestRepo {
    /// The absolute path of the repository: its worktree root, or its git directory when bare.
    #[must_use = "returns the path; the repository is unchanged"]
    pub fn path(&self) -> &Path {
        self.dir.path()
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

impl CorruptTestRepo {
    /// The absolute path of the worktree root of the corrupt repository.
    #[must_use = "returns the path; the repository is unchanged"]
    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

/// A throwaway repository in a temporary directory with two commits on `refs/heads/main`.
///
/// Keeps the `TempDir` alive; pass [`TestRepo::path`] to `gix open --repo`.
#[derive(Debug)]
pub struct TestRepo {
    /// The temporary directory backing the repository (dropped last).
    pub dir: TempDir,
    /// Hex id of the newest commit (`HEAD`).
    pub head: String,
    /// Hex id of its parent commit.
    pub parent: String,
}

impl TestRepo {
    /// The absolute path of the repository; safe to pass to `gix open --repo` from any
    /// engine working directory.
    #[must_use = "returns the path; the repository is unchanged"]
    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

/// Initialises a repository in `dir` with the isolated open options, so the result does not depend
/// on the user's or the system's git configuration.
fn init_isolated(dir: &Path, kind: create::Kind) -> Result<gix::Repository, FixtureError> {
    let repo = gix::ThreadSafeRepository::init_opts(
        dir,
        kind,
        create::Options::default(),
        open::Options::isolated(),
    )?;

    Ok(repo.to_thread_local())
}

/// Points `HEAD` of `repo` at [`MAIN_BRANCH`], pinning the unborn branch name whatever `init.defaultBranch` says.
fn point_head_at_main(repo: &gix::Repository) -> Result<(), FixtureError> {
    repo.edit_reference(RefEdit::update(
        HEAD_REF.try_into()?,
        Target::Symbolic(MAIN_BRANCH.try_into()?),
        PreviousValue::Any,
        FIXTURE_REFLOG_MESSAGE,
    ))?;

    Ok(())
}

/// A repository whose `HEAD` names [`MAIN_BRANCH`] (`refs/heads/main`) without commits.
///
/// Ignores the user's and the system's git configuration (including `init.defaultBranch` and the
/// `GIT_CONFIG_*` environment), so the branch name is always `main`.
#[fixture]
pub fn unborn_repo() -> Result<UnbornTestRepo, FixtureError> {
    let dir = TempDir::new()?;
    let repo = init_isolated(dir.path(), create::Kind::WithWorktree)?;
    point_head_at_main(&repo)?;

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
    point_head_at_main(&repo)?;

    Ok(UnbornTestRepo { dir })
}

/// A repository like [`unborn_repo`] whose `.git/config` cannot be parsed, so opening it fails.
#[fixture]
pub fn corrupt_repo() -> Result<CorruptTestRepo, FixtureError> {
    let repo = unborn_repo()?;

    fs::write(repo.path().join(GIT_DIR).join(CONFIG_FILE), CORRUPT_CONFIG)?;

    Ok(CorruptTestRepo { dir: repo.dir })
}

/// Builds the two-commit fixture repository.
#[fixture]
pub fn test_repo() -> Result<TestRepo, FixtureError> {
    let dir = TempDir::new()?;
    let repo = init_isolated(dir.path(), create::Kind::WithWorktree)?;
    point_head_at_main(&repo)?;

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

/// A fresh in-process [`PluginTest`] with `plugin` registered as `name` and Nushell's `describe`
/// available.
///
/// Callers pass a new plugin instance per test to keep each registry cache isolated.
///
/// # Errors
///
/// Returns [`FixtureError::Failure`] when the engine rejects the plugin or the `describe` command.
pub fn plugin_test_for<P>(name: &str, plugin: Arc<P>) -> Result<PluginTest, FixtureError>
where
    P: Plugin + Send + 'static,
{
    let mut test = PluginTest::new(name, plugin)
        .map_err(|error| FixtureError::Failure(format!("plugin registration failed: {error}")))?;

    test.add_decl(Box::new(Describe))
        .map_err(|error| FixtureError::Failure(format!("describe registration failed: {error}")))?;

    Ok(test)
}

/// Pipeline input carrying `value` as a custom value, as a command receives it from the engine.
#[must_use = "returns the pipeline input; discarding it runs nothing"]
pub fn custom_value_input<V>(value: V) -> PipelineData
where
    V: CustomValue + 'static,
{
    Value::test_custom_value(Box::new(value)).into_pipeline_data()
}

/// Pins `$env.PWD` of `test` to `dir`: the bare test engine has no working directory, and
/// commands such as `gix open` resolve their paths against it.
pub fn pin_pwd<P>(test: &mut PluginTest, dir: P)
where
    P: AsRef<Path>,
{
    test.engine_state_mut().add_env_var(
        PWD_VAR.to_owned(),
        Value::string(dir.as_ref().to_string_lossy(), Span::test_data()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    use rstest::rstest;

    use std::assert_matches;

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
