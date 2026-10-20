//! A walk whose start points are resolved, and which runs on the walker thread.
//!
//! [`Walk::new`] runs on the caller's thread, so resolution and pathspec errors are returned from
//! `log` itself. [`Walk::run`] then owns the walker thread: it builds the revision walk, wraps it in
//! a `Selected`, which leaves out the `skip` commits without reading them, applies `limit`, and feeds
//! the result into the channel.

use super::feed::feed;
use super::reach::Reach;
use super::selected::Selected;
use crate::GixError;
use crate::log::commit::CommitInfo;
use crate::log::options::{LogSettings, LogStart, Mailmap};
use crate::log::paths::PathFilter;
use crate::log::tips::Tips;

use gix::{pathspec::Pattern, revision::walk::Sorting, traverse::commit::simple::CommitTimeOrder};

use std::sync::{Arc, atomic::AtomicBool, mpsc::SyncSender};

/// A walk whose start points are resolved, ready to run on the walker thread.
#[derive(Debug)]
pub(super) struct Walk {
    tips: Tips,
    filter: PathFilter,
    settings: LogSettings,
    /// Shared with the [`CommitLog`](super::commit_log::CommitLog); once set, the scan of the history stops.
    cancelled: Arc<AtomicBool>,
}

impl Walk {
    /// Resolves the start points `start` in `repo` and compiles the path filter `paths`.
    pub(super) fn new(
        repo: &gix::Repository,
        start: &LogStart,
        paths: &[Pattern],
        settings: LogSettings,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self, GixError> {
        Ok(Self {
            tips: Tips::resolve(repo, start)?,
            filter: PathFilter::new(paths)?,
            settings,
            cancelled,
        })
    }

    /// Sends the walk's commits into `sender` until they run out, one fails, the receiver is dropped,
    /// or the stream is cancelled.
    ///
    /// A reversed walk must see its whole selection before it can yield the last commit first, so it
    /// collects the commits up to the first failure through the same cancellable iterator, then feeds
    /// them backwards followed by that failure, so the consumer sees the good rows and then the error.
    pub(super) fn run(
        self,
        repo: &gix::Repository,
        sender: &SyncSender<Result<CommitInfo, GixError>>,
    ) {
        let reverse = self.settings.reverse;

        match self.commits(repo) {
            Err(err) => drop(sender.send(Err(err))),
            Ok(items) if reverse => {
                let mut commits = Vec::new();
                let mut failure = None;

                for item in items {
                    match item {
                        Ok(commit) => commits.push(commit),
                        Err(err) => {
                            failure = Some(err);
                            break;
                        }
                    }
                }

                feed(
                    commits.into_iter().rev().map(Ok).chain(failure.map(Err)),
                    sender,
                );
            }
            Ok(items) => feed(items, sender),
        }
    }

    /// The selected commits, newest first by commit time, after the first `skip` kept commits are dropped
    /// unread and before `limit` cuts the rest. The walk ends once the stream is cancelled, even while the
    /// path filter is rejecting a long run of commits.
    fn commits(
        self,
        repo: &gix::Repository,
    ) -> Result<impl Iterator<Item = Result<CommitInfo, GixError>> + '_, GixError> {
        let platform = repo
            .rev_walk(self.tips.include.iter().copied())
            .sorting(Sorting::ByCommitTime(CommitTimeOrder::NewestFirst))
            .with_hidden(self.tips.hide.iter().copied());

        let platform = if self.settings.first_parent {
            platform.first_parent_only()
        } else {
            platform
        };

        let steps = platform.all().map_err(GixError::Walk)?;

        let mailmap = match self.settings.mailmap {
            Mailmap::Apply => Some(repo.open_mailmap()),
            Mailmap::Ignore => None,
        };

        // History is simplified with a non-empty filter and without `first_parent`; only then is a
        // commit considered when a followed edge reaches it.
        let simplifies_history = !self.filter.is_empty() && !self.settings.first_parent;
        let settings = self.settings;
        let limit = settings.limit.unwrap_or(usize::MAX);

        Ok(Selected {
            repo,
            steps,
            reach: Reach::new(self.tips.include.iter().copied()),
            mailmap,
            filter: self.filter,
            settings,
            skip: settings.skip,
            cancelled: self.cancelled,
            simplifies_history,
            pending: Vec::new(),
            done: false,
        }
        .take(limit))
    }
}
