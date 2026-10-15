//! Reading git history with `gix`; independent of Nushell types.

use crate::error::GixError;

use chrono::{DateTime, FixedOffset};
use gix::{revision::walk::Sorting, traverse::commit::simple::CommitTimeOrder};

use std::{
    path::Path,
    sync::mpsc::{self, Receiver, SyncSender},
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

/// Opens the repository at exactly `path` (no parent-directory discovery).
///
/// # Errors
///
/// Returns [`GixError::Open`] when `path` is not a git repository.
pub fn open(path: &Path) -> Result<gix::Repository, GixError> {
    gix::open(path).map_err(|source| GixError::Open {
        path: path.to_owned(),
        source,
    })
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
/// Returns a [`GixError`] when the repository cannot be opened, HEAD has no commit, or the
/// walker thread cannot be started. Failures while reading individual commits are yielded by
/// the returned stream.
pub fn log(path: &Path, limit: Option<usize>) -> Result<CommitLog, GixError> {
    let repo = open(path)?;
    let tip = repo.head_id().map_err(GixError::Head)?.detach();
    let shared = repo.into_sync();
    let (sender, receiver) = mpsc::sync_channel(CHANNEL_CAPACITY);

    thread::Builder::new()
        .name(WALKER_THREAD_NAME.to_owned())
        .spawn(move || feed(&shared.to_thread_local(), tip, limit, &sender))
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
        .sorting(Sorting::ByCommitTime(CommitTimeOrder::NewestFirst))
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

        #[error("Test failure: {0}")]
        Failure(String),
    }

    /// `(message, raw git timestamp)` pairs, oldest first.
    const HISTORY: [(&str, &str); 3] = [
        ("first", "1700000000 +0000"),
        ("second\n\nbody text", "1700000060 +0200"),
        ("third", "1700000120 -0500"),
    ];

    /// Initialises a repository in `dir` with one empty-tree commit per entry of `history`.
    fn init_with_history(dir: &Path, history: &[(&str, &str)]) -> Result<(), TestError> {
        let repo = gix::init(dir)?;
        let tree = repo.empty_tree().id;
        let mut parents = Vec::new();

        for (message, time) in history {
            let signature = SignatureRef {
                name: "Ada".into(),
                email: "ada@example.com".into(),
                time,
            };

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

    fn collect(path: &Path, limit: Option<usize>) -> Result<Vec<CommitInfo>, TestError> {
        Ok(log(path, limit)?.collect::<Result<_, _>>()?)
    }

    mod open {
        use super::*;

        #[rstest]
        fn opens_existing_repository(dir: Result<TempDir, TestError>) -> Result<(), TestError> {
            let dir = dir?;

            init_with_history(dir.path(), &[])?;
            open(dir.path())?;

            Ok(())
        }

        #[rstest]
        fn rejects_plain_directory(dir: Result<TempDir, TestError>) -> Result<(), TestError> {
            let dir = dir?;

            let err = open(dir.path())
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert!(matches!(err, GixError::Open { .. }), "got {err:?}");

            Ok(())
        }
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
            assert_eq!(second.author, "Ada");
            assert_eq!(second.email, "ada@example.com");

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

            let mut stream = log(dir.path(), None)?;

            let first = stream
                .next()
                .ok_or_else(|| TestError::Failure("stream ended early".to_owned()))??;

            assert_eq!(first.summary, "third");
            assert_eq!(stream.count(), 2);

            Ok(())
        }

        #[rstest]
        fn fails_on_repository_without_commits(
            dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            let dir = dir?;

            init_with_history(dir.path(), &[])?;

            let err = log(dir.path(), None)
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert!(matches!(err, GixError::Head(_)), "got {err:?}");

            Ok(())
        }
    }
}
