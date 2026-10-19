//! Streaming commit history from a repository.

use super::{GixError, OpenRepository};

use chrono::{DateTime, FixedOffset};
use commit::simple;
use gix::{head::Kind, revision::walk::Sorting, traverse::commit};

use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
};

/// Commits buffered between the walker thread and the consumer; bounds memory and applies backpressure.
const CHANNEL_CAPACITY: usize = 256;
const WALKER_THREAD_NAME: &str = "gix-log-walker";

/// One commit as exposed to Nushell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitInfo {
    /// Full hex object id.
    pub id: String,
    /// Author name.
    pub author: String,
    /// Author email.
    pub email: String,
    /// Author time, in the author's own UTC offset.
    pub time: DateTime<FixedOffset>,
    /// First line of the commit message.
    pub summary: String,
}

/// A lazily produced, newest-first sequence of commits.
///
/// The walk runs on a background thread feeding a bounded channel, so only a small window of
/// commits is ever in memory. Dropping the stream stops the walk. After the first error the
/// stream ends.
#[derive(Debug)]
pub struct CommitLog {
    receiver: Receiver<Result<CommitInfo, GixError>>,
}

impl Iterator for CommitLog {
    type Item = Result<CommitInfo, GixError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.receiver.recv().ok()
    }
}

/// Streams commits reachable from HEAD, newest first, at most `limit` of them.
///
/// # Errors
///
/// Returns [`GixError::UnbornHead`] when HEAD names a branch without commits,
/// [`GixError::Head`] when HEAD cannot be resolved to an object, and another [`GixError`] when
/// the walker thread cannot be started. Failures while reading individual commits are yielded
/// by the returned stream.
pub fn log(opened: Arc<OpenRepository>, limit: Option<usize>) -> Result<CommitLog, GixError> {
    let repo = opened.repo();
    let head = repo.head().map_err(GixError::Head)?;
    if let Kind::Unborn(name) = &head.kind {
        return Err(GixError::UnbornHead {
            name: name.to_string(),
        });
    }
    let tip = head.into_peeled_id().map_err(GixError::Head)?.detach();
    let (sender, receiver) = mpsc::sync_channel(CHANNEL_CAPACITY);

    thread::Builder::new()
        .name(WALKER_THREAD_NAME.to_owned())
        .spawn(move || feed(&opened.repo(), tip, limit, &sender))
        .map_err(GixError::Spawn)?;

    Ok(CommitLog { receiver })
}

/// Pushes commits into `sender` until the walk ends, fails, or the receiver is dropped.
fn feed(
    repo: &gix::Repository,
    tip: gix::ObjectId,
    limit: Option<usize>,
    sender: &SyncSender<Result<CommitInfo, GixError>>,
) {
    match commits(repo, tip, limit) {
        Ok(items) => {
            for item in items {
                let failed = item.is_err();
                if sender.send(item).is_err() || failed {
                    break;
                }
            }
        }
        Err(err) => drop(sender.send(Err(err))),
    }
}

fn commits(
    repo: &gix::Repository,
    tip: gix::ObjectId,
    limit: Option<usize>,
) -> Result<impl Iterator<Item = Result<CommitInfo, GixError>>, GixError> {
    let walk = repo
        .rev_walk([tip])
        .sorting(Sorting::ByCommitTime(simple::CommitTimeOrder::NewestFirst))
        .all()
        .map_err(GixError::Walk)?;

    Ok(walk.take(limit.unwrap_or(usize::MAX)).map(|info| {
        let info = info.map_err(GixError::Walk)?;
        commit_info(repo, info.id)
    }))
}

fn commit_info(repo: &gix::Repository, id: gix::ObjectId) -> Result<CommitInfo, GixError> {
    let hex = id.to_string();

    let wrap = |source| GixError::Commit {
        id: hex.clone(),
        source,
    };

    let commit = repo.find_commit(id).map_err(wrap)?;
    let author = commit.author().map_err(wrap)?;
    let message = commit.message().map_err(wrap)?;
    let when = author.time().map_err(wrap)?;

    let offset = FixedOffset::east_opt(when.offset)
        .ok_or_else(|| GixError::Timestamp { id: hex.clone() })?;

    let time = DateTime::from_timestamp(when.seconds, 0)
        .ok_or_else(|| GixError::Timestamp { id: hex.clone() })?
        .with_timezone(&offset);

    Ok(CommitInfo {
        id: hex.clone(),
        author: author.name.to_string(),
        email: author.email.to_string(),
        time,
        summary: message.summary().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{Registry, RepositoryLocation};

    use gix::objs::Tree;
    use nu_plugin_gix_testing_support::{
        COMMITTER_EMAIL, COMMITTER_NAME, COMMITTER_TIME, FIRST_COMMIT_MESSAGE,
        FORK_FIRST_PARENT_MESSAGE, FORK_MERGE_MESSAGE, FORK_ROOT_MESSAGE,
        FORK_SECOND_PARENT_MESSAGE, FixtureError, ForkedTestRepo, MAIN_BRANCH, UnbornTestRepo,
        committer, forked_repo, unborn_repo,
    };
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use std::{assert_matches, io, path::Path};

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Io(#[from] io::Error),

        #[error(transparent)]
        Gix(#[from] gix::Error),

        #[error(transparent)]
        Git(#[from] GixError),

        #[error(transparent)]
        Fixture(#[from] FixtureError),

        #[error("Test failure: {0}")]
        Failure(String),
    }

    /// `(message, raw git timestamp)` pairs, oldest first.
    const HISTORY: [(&str, &str); 3] = [
        (FIRST_COMMIT_MESSAGE, COMMITTER_TIME),
        ("second\n\nbody text", "1700000060 +0200"),
        ("third", "1700000120 -0500"),
    ];

    /// Initialises a repository in `dir` with one empty-tree commit per entry of `history`.
    fn init_with_history(dir: &Path, history: &[(&str, &str)]) -> Result<(), TestError> {
        let repo = gix::init(dir)?;
        // `empty_tree()` is virtual and never reaches the object database, so write it once.
        let tree = repo.write_object(Tree::empty())?.detach();
        let mut parents = Vec::new();

        for (message, time) in history {
            let signature = committer(time);

            let id = repo.commit_as(
                signature,
                signature,
                "HEAD",
                *message,
                tree,
                parents.clone(),
            )?;

            parents = vec![id.detach()];
        }

        Ok(())
    }

    #[fixture]
    fn dir() -> Result<TempDir, TestError> {
        Ok(TempDir::new()?)
    }

    /// The registered `OpenRepository` for the repository at `path`.
    fn opened_at(path: &Path) -> Result<Arc<OpenRepository>, TestError> {
        let location = RepositoryLocation::discover(path)?;
        let registry = Registry::new();
        let id = registry.register(&location)?;

        Ok(registry.acquire(id, &location)?)
    }

    fn collect(path: &Path, limit: Option<usize>) -> Result<Vec<CommitInfo>, TestError> {
        Ok(log(opened_at(path)?, limit)?.collect::<Result<_, _>>()?)
    }

    mod log {
        use super::*;

        #[rstest]
        fn lists_commits_newest_first(dir: Result<TempDir, TestError>) -> Result<(), TestError> {
            let dir = dir?;

            init_with_history(dir.path(), &HISTORY)?;

            let commits = collect(dir.path(), None)?;
            let summaries: Vec<&str> = commits.iter().map(|c| c.summary.as_str()).collect();

            assert_eq!(summaries, ["third", "second", "first"]);

            Ok(())
        }

        #[rstest]
        fn exposes_author_and_time(dir: Result<TempDir, TestError>) -> Result<(), TestError> {
            let dir = dir?;

            init_with_history(dir.path(), &HISTORY)?;

            let commits = collect(dir.path(), None)?;

            let second = commits
                .get(1)
                .ok_or_else(|| TestError::Failure("missing second commit".to_owned()))?;

            assert_eq!(second.id.len(), 40);
            assert_eq!(second.author, COMMITTER_NAME);
            assert_eq!(second.email, COMMITTER_EMAIL);

            assert_eq!(
                second.time.to_rfc3339(), // 1_700_000_060 s = 2023-11-14T22:14:20Z, shifted by +02:00
                "2023-11-15T00:14:20+02:00"
            );

            Ok(())
        }

        #[rstest]
        #[case::zero(0, 0)]
        #[case::one(1, 1)]
        #[case::exact(3, 3)]
        #[case::more_than_history(10, 3)]
        #[case::usize_max(usize::MAX, 3)]
        fn respects_limit(
            dir: Result<TempDir, TestError>,
            #[case] limit: usize,
            #[case] expected: usize,
        ) -> Result<(), TestError> {
            let dir = dir?;

            init_with_history(dir.path(), &HISTORY)?;

            assert_eq!(collect(dir.path(), Some(limit))?.len(), expected);

            Ok(())
        }

        #[rstest]
        fn yields_commits_one_at_a_time(dir: Result<TempDir, TestError>) -> Result<(), TestError> {
            let dir = dir?;

            init_with_history(dir.path(), &HISTORY)?;

            let mut stream = log(opened_at(dir.path())?, None)?;

            let first = stream
                .next()
                .ok_or_else(|| TestError::Failure("stream ended early".to_owned()))??;

            assert_eq!(first.summary, "third");
            assert_eq!(stream.count(), 2);

            Ok(())
        }

        #[rstest]
        fn reports_unborn_head_for_repository_without_commits(
            unborn_repo: Result<UnbornTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = unborn_repo?;

            let err = log(opened_at(repo.path())?, None)
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert_matches!(
                &err,
                GixError::UnbornHead { name } if name == MAIN_BRANCH,
                "got {err:?}"
            );

            Ok(())
        }

        /// The limit that keeps the two newest commits of `forked_repo`.
        const CAPPED_FORK_LIMIT: usize = 2;
        /// The merge of `forked_repo`: its author time is 1700000300 in a `-05:00` offset.
        const MERGE_TIME: &str = "2023-11-14T17:18:20-05:00";
        /// The root of `forked_repo`, which is UTC.
        const ROOT_TIME: &str = "2023-11-14T22:13:20+00:00";

        #[rstest]
        fn lists_forked_history_by_commit_time_not_first_parent(
            forked_repo: Result<ForkedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = forked_repo?;
            let summaries: Vec<String> = collect(repo.path(), None)?
                .into_iter()
                .map(|commit| commit.summary)
                .collect();

            assert_eq!(
                summaries,
                [
                    FORK_MERGE_MESSAGE,
                    FORK_SECOND_PARENT_MESSAGE,
                    FORK_FIRST_PARENT_MESSAGE,
                    FORK_ROOT_MESSAGE,
                ]
            );

            Ok(())
        }

        #[rstest]
        fn keeps_the_newest_commits_by_time_when_capped(
            forked_repo: Result<ForkedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = forked_repo?;
            let summaries: Vec<String> = collect(repo.path(), Some(CAPPED_FORK_LIMIT))?
                .into_iter()
                .map(|commit| commit.summary)
                .collect();

            assert_eq!(summaries, [FORK_MERGE_MESSAGE, FORK_SECOND_PARENT_MESSAGE]);

            Ok(())
        }

        #[rstest]
        fn carries_each_commit_time_with_its_own_offset(
            forked_repo: Result<ForkedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = forked_repo?;
            let commits = collect(repo.path(), None)?;
            let merge = commits
                .first()
                .ok_or_else(|| TestError::Failure("missing merge commit".to_owned()))?;
            let root = commits
                .last()
                .ok_or_else(|| TestError::Failure("missing root commit".to_owned()))?;

            assert_eq!(merge.time.to_rfc3339(), MERGE_TIME);
            assert_eq!(root.time.to_rfc3339(), ROOT_TIME);

            Ok(())
        }
    }

    mod fixture {
        use super::*;

        const ABBREVIATED_LEN: usize = 7;

        #[rstest]
        fn commit_tree_resolves_by_abbreviated_id(
            dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            let dir = dir?;

            init_with_history(dir.path(), &HISTORY)?;

            let repo = gix::open(dir.path())?;
            let tree_id = repo.head_commit()?.tree_id()?.detach();
            let tree_hex = tree_id.to_string();
            let abbreviated = tree_hex
                .get(..ABBREVIATED_LEN)
                .ok_or_else(|| TestError::Failure("tree id shorter than the prefix".to_owned()))?;

            assert_eq!(repo.rev_parse_single(abbreviated)?.detach(), tree_id);

            Ok(())
        }
    }
}
