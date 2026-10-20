//! The public entry point: resolves a walk, starts its thread and returns the stream.

use super::commit_log::{CHANNEL_CAPACITY, CommitLog, WALKER_THREAD_NAME};
use super::plan::Walk;
use crate::log::options::{LogSettings, LogStart};
use crate::{GixError, OpenRepository};

use gix::pathspec::Pattern;

use std::{
    sync::{Arc, atomic::AtomicBool, mpsc},
    thread,
};

/// Streams the commits selected from `start`, filtered by `paths` and ordered and trimmed by `settings`.
///
/// The pipeline and the guarantees are described in the overview of the `log` module; this documents
/// the arguments.
///
/// # Start points
///
/// Without a start, the walk begins at `HEAD`. `start` names revisions or ranges, plus every
/// reference with `start.all`.
///
/// # Filtering
///
/// With no `paths`, every commit is kept; otherwise only commits that change one of them. Make the
/// paths with [`pathspec`](crate::log::pathspec), which resolves git pathspec syntax against the
/// directory it was written in. History is simplified as `git log -- <paths>` does: a merge identical
/// to a parent at these paths is not listed, and only the first such parent's history is followed, so
/// commits reachable only through the other parents are not listed. With `settings.first_parent` only
/// the first parent is compared, as `git log --first-parent -- <paths>` does.
///
/// # Ordering and trimming
///
/// The default `settings` walk the history newest first by commit time, with no skip or limit, and
/// show signatures through the mailmap. `settings.reverse` streams the selection oldest first,
/// `settings.skip` leaves out the first commits of the selection and `settings.limit` keeps at most
/// that many. A reversed stream buffers the whole selection before its first commit.
///
/// # Cancellation
///
/// Dropping the stream, or interrupting it with `CommitLog::with_interrupt`, stops the walk even while
/// it scans commits that do not match `paths`.
///
/// # Errors
///
/// Returns [`GixError::UnbornHead`] when HEAD names a branch without commits,
/// [`GixError::Head`] when HEAD cannot be resolved to an object, [`GixError::Revision`] when a
/// revision does not resolve, [`GixError::NotACommit`] when it names a tree or blob,
/// [`GixError::RefRead`] when the references cannot be listed, [`GixError::Pathspec`] when a path is
/// not a valid pathspec, and another [`GixError`] when the walker thread cannot be started. Walk
/// failures and failures while reading individual commits are yielded by the returned stream.
pub fn log(
    opened: Arc<OpenRepository>,
    start: &LogStart,
    paths: &[Pattern],
    settings: LogSettings,
) -> Result<CommitLog, GixError> {
    let cancelled = Arc::new(AtomicBool::new(false));

    let walk = Walk::new(
        &opened.repo(),
        start,
        paths,
        settings,
        Arc::clone(&cancelled),
    )?;

    let (sender, receiver) = mpsc::sync_channel(CHANNEL_CAPACITY);

    let walker = thread::Builder::new()
        .name(WALKER_THREAD_NAME.to_owned())
        .spawn(move || walk.run(&opened.repo(), &sender))
        .map_err(GixError::Spawn)?;

    Ok(CommitLog {
        receiver,
        walker: Some(walker),
        cancelled,
        interrupted: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fixtures::open_path;
    use crate::log::commit::CommitInfo;
    use crate::log::{FileStat, LogStart, Mailmap, pathspec};

    use gix::index::write::Options;
    use nu_plugin_gix_testing_support::{
        ADD_DIR_MESSAGE, ADD_FILE_MESSAGE, COMMITTER_EMAIL, COMMITTER_NAME, COMMITTER_TIME,
        DISCARDED_FIRST_PARENT_MESSAGE, DISCARDED_MERGE_MESSAGE, DISCARDED_ROOT_MESSAGE,
        DISCARDED_SIDE_MESSAGE, DiscardedSideTestRepo, DivergedTestRepo, EDIT_FILE_MESSAGE,
        FIRST_COMMIT_MESSAGE, FORK_FIRST_PARENT_MESSAGE, FORK_MERGE_MESSAGE, FORK_ROOT_MESSAGE,
        FORK_SECOND_PARENT_MESSAGE, FixtureError, FixtureRepo as _, ForkedTestRepo, HISTORY,
        MAILMAP_EMAIL, MAILMAP_NAME, OTHER_COMMIT_MESSAGE, PATHS_FILE, PATHS_NESTED_FILE,
        PathsTestRepo, SECOND_COMMIT_MESSAGE, SECOND_WINS_FIRST_PARENT_MESSAGE,
        SECOND_WINS_MERGE_MESSAGE, SECOND_WINS_ROOT_MESSAGE, SECOND_WINS_SECOND_PARENT_MESSAGE,
        SKEW_PARENT_MESSAGE, SKEW_ROOT_MESSAGE, SecondParentWinsTestRepo, SkewedTestRepo, TestRepo,
        dir, discarded_side_repo, diverged_repo, forked_repo, init_with_history, mailmap_repo,
        paths_repo, remove_loose_object, second_parent_wins_repo, skewed_repo,
    };
    use rstest::rstest;
    use tempfile::TempDir;

    use std::{assert_matches, io, iter, path::Path, sync::atomic::Ordering, time::Duration};

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

    /// The registered `OpenRepository` for the repository at `path`.
    fn opened_at(path: &Path) -> Result<Arc<OpenRepository>, TestError> {
        Ok(Arc::new(open_path(path)?))
    }

    /// A start that walks the given revisions and nothing else.
    fn from_revisions(expressions: &[&str]) -> LogStart {
        LogStart {
            revisions: expressions
                .iter()
                .map(|&expression| expression.to_owned())
                .collect(),
            all: false,
        }
    }

    /// The commits `log` streams for the repository at `path`, with the given start, paths and settings.
    fn collect(
        path: &Path,
        start: &LogStart,
        paths: &[Pattern],
        settings: LogSettings,
    ) -> Result<Vec<CommitInfo>, TestError> {
        Ok(log(opened_at(path)?, start, paths, settings)?.collect::<Result<_, _>>()?)
    }

    /// The commits of the repository at `path` walked from HEAD with `settings`, without a path filter.
    fn collect_settings(path: &Path, settings: LogSettings) -> Result<Vec<CommitInfo>, TestError> {
        collect(path, &LogStart::default(), &[], settings)
    }

    /// The commits of the repository at `path` walked from `start`, with the default settings.
    fn collect_start(path: &Path, start: &LogStart) -> Result<Vec<CommitInfo>, TestError> {
        collect(path, start, &[], LogSettings::default())
    }

    /// The commits of the repository at `path` walked from HEAD, keeping only the commits that change `paths`.
    fn collect_paths(path: &Path, paths: &[Pattern]) -> Result<Vec<CommitInfo>, TestError> {
        collect(path, &LogStart::default(), paths, LogSettings::default())
    }

    /// The pathspecs `paths` written at the root of the repository at `repo`.
    fn specs(repo: &Path, paths: &[&str]) -> Result<Vec<Pattern>, TestError> {
        Ok(paths
            .iter()
            .map(|spec| pathspec(Some(repo), repo, spec))
            .collect::<Result<_, _>>()?)
    }

    /// The paths of the changed files each commit lists, `None` where it lists none.
    fn changed_paths(commits: &[CommitInfo]) -> Vec<Option<Vec<String>>> {
        commits
            .iter()
            .map(|commit| {
                commit
                    .files
                    .as_ref()
                    .map(|files| files.iter().map(|file| file.path.clone()).collect())
            })
            .collect()
    }

    mod log {
        use super::*;

        #[rstest]
        #[case::zero(0, 0)]
        #[case::one(1, 1)]
        #[case::exact(3, 3)]
        #[case::more_than_history(10, 3)]
        #[case::usize_max(usize::MAX, 3)]
        fn respects_limit(
            dir: Result<TempDir, FixtureError>,
            #[case] limit: usize,
            #[case] expected: usize,
        ) -> Result<(), TestError> {
            let dir = dir?;

            init_with_history(dir.path(), &HISTORY)?;

            assert_eq!(
                collect_settings(
                    dir.path(),
                    LogSettings {
                        limit: Some(limit),
                        ..LogSettings::default()
                    }
                )?
                .len(),
                expected
            );

            Ok(())
        }

        /// The limit that keeps the two newest commits of `forked_repo`.
        const CAPPED_FORK_LIMIT: usize = 2;

        #[rstest]
        fn lists_forked_history_by_commit_time_not_first_parent(
            forked_repo: Result<ForkedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = forked_repo?;

            let summaries: Vec<String> = collect_settings(repo.path(), LogSettings::default())?
                .into_iter()
                .map(|commit| commit.subject)
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

            let summaries: Vec<String> = collect_settings(
                repo.path(),
                LogSettings {
                    limit: Some(CAPPED_FORK_LIMIT),
                    ..LogSettings::default()
                },
            )?
            .into_iter()
            .map(|commit| commit.subject)
            .collect();

            assert_eq!(summaries, [FORK_MERGE_MESSAGE, FORK_SECOND_PARENT_MESSAGE]);

            Ok(())
        }

        #[rstest]
        #[case::none(0, &["third", "second", "first"])]
        #[case::one(1, &["second", "first"])]
        #[case::all(3, &[])]
        #[case::usize_max(usize::MAX, &[])]
        fn skips_the_newest_commits(
            dir: Result<TempDir, FixtureError>,
            #[case] skip: usize,
            #[case] expected: &[&str],
        ) -> Result<(), TestError> {
            let dir = dir?;

            init_with_history(dir.path(), &HISTORY)?;

            let summaries: Vec<String> = collect_settings(
                dir.path(),
                LogSettings {
                    skip,
                    ..LogSettings::default()
                },
            )?
            .into_iter()
            .map(|commit| commit.subject)
            .collect();

            assert_eq!(summaries, expected);

            Ok(())
        }

        #[rstest]
        fn applies_skip_before_limit(dir: Result<TempDir, FixtureError>) -> Result<(), TestError> {
            let dir = dir?;

            init_with_history(dir.path(), &HISTORY)?;

            let summaries: Vec<String> = collect_settings(
                dir.path(),
                LogSettings {
                    skip: 1,
                    limit: Some(1),
                    ..LogSettings::default()
                },
            )?
            .into_iter()
            .map(|commit| commit.subject)
            .collect();

            assert_eq!(summaries, ["second"]);

            Ok(())
        }

        #[rstest]
        #[case::skip(1, None, &[ADD_FILE_MESSAGE])]
        #[case::limit(0, Some(1), &[EDIT_FILE_MESSAGE])]
        fn trims_only_the_commits_that_pass_the_path_filter(
            paths_repo: Result<PathsTestRepo, FixtureError>,
            #[case] skip: usize,
            #[case] limit: Option<usize>,
            #[case] expected: &[&str],
        ) -> Result<(), TestError> {
            let repo = paths_repo?;

            let summaries: Vec<String> = collect(
                repo.path(),
                &LogStart::default(),
                &specs(repo.path(), &[PATHS_FILE])?,
                LogSettings {
                    skip,
                    limit,
                    ..LogSettings::default()
                },
            )?
            .into_iter()
            .map(|commit| commit.subject)
            .collect();

            assert_eq!(summaries, expected);

            Ok(())
        }

        #[rstest]
        fn reverse_lists_oldest_first(dir: Result<TempDir, FixtureError>) -> Result<(), TestError> {
            let dir = dir?;

            init_with_history(dir.path(), &HISTORY)?;

            let summaries: Vec<String> = collect_settings(
                dir.path(),
                LogSettings {
                    reverse: true,
                    ..LogSettings::default()
                },
            )?
            .into_iter()
            .map(|commit| commit.subject)
            .collect();

            assert_eq!(summaries, ["first", "second", "third"]);

            Ok(())
        }

        #[rstest]
        fn reverse_keeps_the_newest_window(
            dir: Result<TempDir, FixtureError>,
        ) -> Result<(), TestError> {
            let dir = dir?;

            init_with_history(dir.path(), &HISTORY)?;

            let summaries: Vec<String> = collect_settings(
                dir.path(),
                LogSettings {
                    reverse: true,
                    limit: Some(2),
                    ..LogSettings::default()
                },
            )?
            .into_iter()
            .map(|commit| commit.subject)
            .collect();

            assert_eq!(summaries, ["second", "third"]);

            Ok(())
        }

        #[rstest]
        fn reverse_yields_the_good_commits_before_a_failure(
            dir: Result<TempDir, FixtureError>,
        ) -> Result<(), TestError> {
            let dir = dir?;

            init_with_history(
                dir.path(),
                &[
                    (FIRST_COMMIT_MESSAGE, COMMITTER_TIME),
                    ("second", "1700000060 +0000"),
                    ("third", "1700000120 +0000"),
                    ("fourth", "1700000180 +0000"),
                ],
            )?;

            let second = collect_settings(dir.path(), LogSettings::default())?
                .into_iter()
                .find(|commit| commit.subject == "second")
                .ok_or_else(|| TestError::Failure("no second commit".to_owned()))?
                .oid;

            // The middle commit is unreadable, so the walk fails after `fourth` and `third`.
            remove_loose_object(dir.path(), second)?;

            let items: Vec<Result<CommitInfo, GixError>> = log(
                opened_at(dir.path())?,
                &LogStart::default(),
                &[],
                LogSettings {
                    reverse: true,
                    ..LogSettings::default()
                },
            )?
            .collect();

            let (failure, good) = items
                .split_last()
                .ok_or_else(|| TestError::Failure("the walk yielded nothing".to_owned()))?;

            let summaries: Vec<String> = good
                .iter()
                .filter_map(|item| item.as_ref().ok())
                .map(|commit| commit.subject.clone())
                .collect();

            assert!(good.iter().all(Result::is_ok));
            assert_eq!(summaries, ["third", "fourth"]);
            assert_matches!(failure, Err(_));

            Ok(())
        }

        #[rstest]
        fn first_parent_follows_only_first_parents(
            forked_repo: Result<ForkedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = forked_repo?;

            let summaries: Vec<String> = collect_settings(
                repo.path(),
                LogSettings {
                    first_parent: true,
                    ..LogSettings::default()
                },
            )?
            .into_iter()
            .map(|commit| commit.subject)
            .collect();

            assert_eq!(
                summaries,
                [
                    FORK_MERGE_MESSAGE,
                    FORK_FIRST_PARENT_MESSAGE,
                    FORK_ROOT_MESSAGE,
                ]
            );

            Ok(())
        }

        #[rstest]
        fn first_parent_compares_only_the_first_parent_at_the_paths(
            second_parent_wins_repo: Result<SecondParentWinsTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = second_parent_wins_repo?;

            let summaries: Vec<String> = collect(
                repo.path(),
                &LogStart::default(),
                &specs(repo.path(), &[PATHS_FILE])?,
                LogSettings {
                    first_parent: true,
                    ..LogSettings::default()
                },
            )?
            .into_iter()
            .map(|commit| commit.subject)
            .collect();

            assert_eq!(
                summaries,
                [
                    SECOND_WINS_MERGE_MESSAGE,
                    SECOND_WINS_FIRST_PARENT_MESSAGE,
                    SECOND_WINS_ROOT_MESSAGE,
                ]
            );

            Ok(())
        }

        #[rstest]
        fn default_walk_drops_a_merge_identical_to_its_second_parent(
            second_parent_wins_repo: Result<SecondParentWinsTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = second_parent_wins_repo?;

            let summaries: Vec<String> =
                collect_paths(repo.path(), &specs(repo.path(), &[PATHS_FILE])?)?
                    .into_iter()
                    .map(|commit| commit.subject)
                    .collect();

            assert_eq!(
                summaries,
                [SECOND_WINS_SECOND_PARENT_MESSAGE, SECOND_WINS_ROOT_MESSAGE]
            );

            Ok(())
        }

        #[rstest]
        fn starts_from_a_revision(dir: Result<TempDir, FixtureError>) -> Result<(), TestError> {
            let dir = dir?;

            init_with_history(dir.path(), &HISTORY)?;

            let summaries: Vec<String> = collect_start(dir.path(), &from_revisions(&["HEAD~1"]))?
                .into_iter()
                .map(|commit| commit.subject)
                .collect();

            assert_eq!(summaries, ["second", "first"]);

            Ok(())
        }

        #[rstest]
        fn hides_the_excluded_side_of_a_range(
            dir: Result<TempDir, FixtureError>,
        ) -> Result<(), TestError> {
            let dir = dir?;

            init_with_history(dir.path(), &HISTORY)?;

            let summaries: Vec<String> =
                collect_start(dir.path(), &from_revisions(&["HEAD~2..HEAD"]))?
                    .into_iter()
                    .map(|commit| commit.subject)
                    .collect();

            assert_eq!(summaries, ["third", "second"]);

            Ok(())
        }

        #[rstest]
        fn lists_branches_head_cannot_reach_with_all(
            diverged_repo: Result<DivergedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = diverged_repo?;

            let summaries: Vec<String> = collect_start(
                repo.path(),
                &LogStart {
                    revisions: Vec::new(),
                    all: true,
                },
            )?
            .into_iter()
            .map(|commit| commit.subject)
            .collect();

            assert_eq!(
                summaries,
                [
                    OTHER_COMMIT_MESSAGE,
                    SECOND_COMMIT_MESSAGE,
                    FIRST_COMMIT_MESSAGE,
                ]
            );

            Ok(())
        }

        #[rstest]
        fn filters_by_paths(
            paths_repo: Result<PathsTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = paths_repo?;

            let summaries: Vec<String> =
                collect_paths(repo.path(), &specs(repo.path(), &[PATHS_FILE])?)?
                    .into_iter()
                    .map(|commit| commit.subject)
                    .collect();

            assert_eq!(summaries, [EDIT_FILE_MESSAGE, ADD_FILE_MESSAGE]);

            Ok(())
        }

        #[rstest]
        fn drops_a_side_change_that_a_merge_discards(
            discarded_side_repo: Result<DiscardedSideTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = discarded_side_repo?;

            let summaries: Vec<String> =
                collect_paths(repo.path(), &specs(repo.path(), &[PATHS_FILE])?)?
                    .into_iter()
                    .map(|commit| commit.subject)
                    .collect();

            assert_eq!(summaries, [DISCARDED_ROOT_MESSAGE]);

            Ok(())
        }

        #[rstest]
        fn lists_the_whole_history_without_a_path_filter(
            discarded_side_repo: Result<DiscardedSideTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = discarded_side_repo?;

            let summaries: Vec<String> = collect_settings(repo.path(), LogSettings::default())?
                .into_iter()
                .map(|commit| commit.subject)
                .collect();

            assert_eq!(
                summaries,
                [
                    DISCARDED_MERGE_MESSAGE,
                    DISCARDED_SIDE_MESSAGE,
                    DISCARDED_FIRST_PARENT_MESSAGE,
                    DISCARDED_ROOT_MESSAGE,
                ]
            );

            Ok(())
        }

        #[rstest]
        fn lists_a_commit_walked_before_the_child_that_follows_it(
            skewed_repo: Result<SkewedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = skewed_repo?;

            let summaries: Vec<String> =
                collect_paths(repo.path(), &specs(repo.path(), &[PATHS_FILE])?)?
                    .into_iter()
                    .map(|commit| commit.subject)
                    .collect();

            assert_eq!(summaries, [SKEW_PARENT_MESSAGE, SKEW_ROOT_MESSAGE]);

            Ok(())
        }

        #[rstest]
        fn yields_nothing_without_start_points(
            dir: Result<TempDir, FixtureError>,
        ) -> Result<(), TestError> {
            let dir = dir?;

            init_with_history(dir.path(), &HISTORY)?;

            let summaries: Vec<String> = collect_start(dir.path(), &from_revisions(&["^HEAD"]))?
                .into_iter()
                .map(|commit| commit.subject)
                .collect();

            assert_eq!(summaries, Vec::<String>::new());

            Ok(())
        }

        #[rstest]
        #[case::apply(Mailmap::Apply, (MAILMAP_NAME, MAILMAP_EMAIL))]
        #[case::ignore(Mailmap::Ignore, (COMMITTER_NAME, COMMITTER_EMAIL))]
        fn shows_the_head_author_as_the_mailmap_asks(
            mailmap_repo: Result<TestRepo, FixtureError>,
            #[case] mailmap: Mailmap,
            #[case] expected: (&str, &str),
        ) -> Result<(), TestError> {
            let repo = mailmap_repo?;

            let commits = collect_settings(
                repo.path(),
                LogSettings {
                    mailmap,
                    ..LogSettings::default()
                },
            )?;

            let head = commits
                .first()
                .ok_or_else(|| TestError::Failure("the walk listed no commits".to_owned()))?;

            assert_eq!(
                (head.author.name.as_str(), head.author.email.as_str()),
                expected
            );

            Ok(())
        }

        #[rstest]
        #[case::neither(false, false)]
        #[case::files(true, false)]
        #[case::stat_alone_lists_files(false, true)]
        #[case::files_and_stat(true, true)]
        fn lists_the_files_each_commit_changes_as_asked(
            paths_repo: Result<PathsTestRepo, FixtureError>,
            #[case] files: bool,
            #[case] stat: bool,
        ) -> Result<(), TestError> {
            let repo = paths_repo?;

            let commits = collect_settings(
                repo.path(),
                LogSettings {
                    files,
                    stat,
                    ..LogSettings::default()
                },
            )?;

            let expected_paths: Vec<Option<Vec<String>>> = if files || stat {
                vec![
                    Some(Vec::new()),
                    Some(vec![PATHS_NESTED_FILE.to_owned()]),
                    Some(vec![PATHS_FILE.to_owned()]),
                    Some(vec![PATHS_NESTED_FILE.to_owned()]),
                    Some(vec![PATHS_FILE.to_owned()]),
                ]
            } else {
                vec![None; 5]
            };

            assert_eq!(changed_paths(&commits), expected_paths);

            assert!(
                commits
                    .iter()
                    .flat_map(|commit| commit.files.iter().flatten())
                    .all(|file| matches!(file.stat, Some(FileStat::Text { .. })) == stat)
            );

            Ok(())
        }
    }

    mod skip {
        use super::*;

        #[rstest]
        #[case::files(true, false)]
        #[case::stat(false, true)]
        fn does_not_read_the_trees_of_skipped_commits(
            paths_repo: Result<PathsTestRepo, FixtureError>,
            #[case] files: bool,
            #[case] stat: bool,
        ) -> Result<(), TestError> {
            let repo = paths_repo?;
            let opened = open_path(repo.path())?.repo();
            let head = opened.find_commit(repo.unchanged)?;

            // Diffs fall back to an index built from HEAD's tree when none is stored, so store one first.
            let head_tree = head.tree_id()?.detach();

            opened
                .index_from_tree(&head_tree)?
                .write(Options::default())?;

            // `unchanged` and `edit_nested` share this tree; the commits below them never read it.
            remove_loose_object(repo.path(), head_tree)?;

            let commits = collect_settings(
                repo.path(),
                LogSettings {
                    skip: 2,
                    files,
                    stat,
                    ..LogSettings::default()
                },
            )?;

            assert_eq!(
                commits
                    .iter()
                    .map(|commit| commit.subject.as_str())
                    .collect::<Vec<_>>(),
                vec![EDIT_FILE_MESSAGE, ADD_DIR_MESSAGE, ADD_FILE_MESSAGE]
            );

            Ok(())
        }
    }

    mod early_stop {
        use super::*;

        /// Commits in the long history; just above the channel window.
        const HISTORY_LEN: usize = 300;
        /// Commit time of the oldest commit of the long history, in seconds since the epoch.
        const HISTORY_START_SECONDS: usize = 1_700_000_000;
        /// How long the walker is given to fill the channel and block before the buffer is counted.
        const SETTLE: Duration = Duration::from_millis(100);
        /// Most commits that may be buffered after one is taken: a full channel plus one blocked in `send`.
        const MAX_BUFFERED: usize = CHANNEL_CAPACITY.saturating_add(1);
        /// Buffered commits must stay well short of the full history.
        const MAX_BUFFERED_BELOW_HISTORY: usize = HISTORY_LEN.saturating_sub(1);

        /// `HISTORY_LEN` `(message, raw git timestamp)` pairs, oldest first, with increasing times.
        fn long_history() -> Vec<(String, String)> {
            (0..HISTORY_LEN)
                .map(|index| {
                    (
                        format!("commit {index}"),
                        format!("{} +0000", HISTORY_START_SECONDS.saturating_add(index)),
                    )
                })
                .collect()
        }

        #[rstest]
        fn taking_one_commit_stops_the_walker_within_the_channel_window(
            dir: Result<TempDir, FixtureError>,
        ) -> Result<(), TestError> {
            let dir = dir?;
            let owned = long_history();

            let history: Vec<(&str, &str)> = owned
                .iter()
                .map(|(message, time)| (message.as_str(), time.as_str()))
                .collect();

            init_with_history(dir.path(), &history)?;

            let mut stream = log(
                opened_at(dir.path())?,
                &LogStart::default(),
                &[],
                LogSettings::default(),
            )?;

            let newest: CommitInfo = stream
                .next()
                .ok_or_else(|| TestError::Failure("stream ended early".to_owned()))??;

            assert_eq!(
                newest.subject,
                format!("commit {}", HISTORY_LEN.saturating_sub(1))
            );

            thread::sleep(SETTLE);

            // Stop the walker from scanning further, so the count holds only what the channel kept
            // at the settle point plus at most the one commit blocked in `send`.
            stream.cancelled.store(true, Ordering::Relaxed);

            let buffered = iter::from_fn(|| stream.receiver.try_recv().ok())
                .flatten()
                .count();

            assert!(
                buffered <= MAX_BUFFERED,
                "buffered {buffered} commits after taking one"
            );

            assert!(
                buffered < MAX_BUFFERED_BELOW_HISTORY,
                "buffered {buffered} of {HISTORY_LEN} commits"
            );

            let walker = stream
                .walker
                .take()
                .ok_or_else(|| TestError::Failure("no walker thread".to_owned()))?;

            drop(stream);

            walker
                .join()
                .map_err(|panic| TestError::Failure(format!("walker panicked: {panic:?}")))?;

            Ok(())
        }

        /// A path no commit of the fixture history changes, so the walk scans everything and keeps nothing.
        const UNMATCHED_PATH: &str = "no-such-file.txt";

        #[rstest]
        fn dropping_a_walk_that_matches_nothing_lets_the_walker_finish(
            dir: Result<TempDir, FixtureError>,
        ) -> Result<(), TestError> {
            let dir = dir?;

            init_with_history(dir.path(), &HISTORY)?;

            let mut stream = log(
                opened_at(dir.path())?,
                &LogStart::default(),
                &specs(dir.path(), &[UNMATCHED_PATH])?,
                LogSettings::default(),
            )?;

            let walker = stream
                .walker
                .take()
                .ok_or_else(|| TestError::Failure("no walker thread".to_owned()))?;

            drop(stream);

            walker
                .join()
                .map_err(|panic| TestError::Failure(format!("walker panicked: {panic:?}")))?;

            Ok(())
        }
    }
}
