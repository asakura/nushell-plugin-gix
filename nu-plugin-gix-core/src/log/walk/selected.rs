//! The commits a walk selects, in the order the walk reaches them.
//!
//! `Selected` is a small state machine driven by `advance`:
//!
//! ```text
//!  pending not empty ──▶ visit the last pending commit ──▶ maybe list it (skipped ones are not read); release waiting parents it follows
//!  pending empty ───────▶ cancelled? ──▶ done
//!                         next walk step ──▶ none left? ──▶ done
//!                                          ├─ not reached by a followed edge ──▶ dropped (waits in `Reach`)
//!                                          └─ reached ──▶ pushed onto pending
//! ```
//!
//! Waiting is how clock skew is handled. The revision walk orders by commit time, so a parent with a
//! newer time than its child comes out first. Under simplification it is held back until the child
//! follows it, then visited right after that child.

use super::reach::Reach;
use crate::GixError;
use crate::log::changes::changed_files;
use crate::log::commit::CommitInfo;
use crate::log::options::LogSettings;
use crate::log::paths::PathFilter;

use gix::{mailmap::Snapshot, revision::Walk as RevWalk};

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// The commits a walk selects, in the order the walk reaches them; see [`Walk::commits`](super::plan::Walk::commits).
///
/// A walk step brings in its commit first. When history is simplified, a step whose commit no followed edge
/// reached is dropped without being read, so the commits behind a pruned parent are not listed. Otherwise a
/// reached commit is visited, and each commit it follows is visited too, so a parent that waited for an
/// earlier-walked child is listed right after that child. Parents that no followed edge reaches yet wait in `reach`.
/// The first `skip` commits the path filter keeps are left out here, before any read of their contents.
#[expect(
    clippy::field_scoped_visibility_modifiers,
    reason = "the walker builds the selection from its fields"
)]
pub(super) struct Selected<'repo> {
    pub(super) repo: &'repo gix::Repository,
    pub(super) steps: RevWalk<'repo>,
    pub(super) reach: Reach,
    pub(super) mailmap: Option<Snapshot>,
    pub(super) filter: PathFilter,
    pub(super) settings: LogSettings,
    /// Commits the path filter keeps that are still to be left out; each one is counted down without being read.
    pub(super) skip: usize,
    /// Shared with the [`CommitLog`](super::commit_log::CommitLog); once set, the scan of the history stops.
    pub(super) cancelled: Arc<AtomicBool>,
    /// Whether history is simplified against the path filter: only with a non-empty filter and without
    /// `first_parent`.
    pub(super) simplifies_history: bool,
    /// Commits reached but not yet visited, with their parents; popped last-in first-out.
    pub(super) pending: Vec<(gix::ObjectId, Vec<gix::ObjectId>)>,
    /// Set once the walk has ended or failed, so the stream stays ended.
    pub(super) done: bool,
}

impl Selected<'_> {
    /// Reads the commit `id` and decides whether the path filter keeps it, given its `parents`. Returns the
    /// kept commit, if any, and the parents the walk follows from it: the one identical parent, or every
    /// parent when it is listed. A listed commit's signatures are mapped through the mailmap when it is given,
    /// and it lists its changed files when `settings.files` or `settings.stat` is set. A kept commit that falls
    /// within `skip` is not read beyond the commit itself: it is returned as not listed, and `skip` counts down.
    fn visit(
        &mut self,
        id: gix::ObjectId,
        parents: &[gix::ObjectId],
    ) -> Result<(Option<CommitInfo>, Vec<gix::ObjectId>), GixError> {
        let unreadable = |source| GixError::Commit {
            id: id.to_string(),
            source,
        };

        let commit = self.repo.find_commit(id).map_err(unreadable)?;

        let wants_files = self.settings.files || self.settings.stat;

        // A commit that is listed and not skipped lists its files from the same tree diff that decides it.
        let (simplified, files) = if wants_files && self.skip == 0 {
            self.filter
                .simplify_with_files(self.repo, &commit, parents, self.settings.stat)
        } else {
            self.filter
                .simplify(self.repo, &commit, parents)
                .map(|simplified| (simplified, None))
        }
        .map_err(unreadable)?;

        let follow = simplified
            .follow
            .map_or_else(|| parents.to_vec(), |parent| vec![parent]);

        let listed = if !simplified.listed {
            None
        } else if self.skip > 0 {
            self.skip = self.skip.saturating_sub(1);

            None
        } else {
            let mut info = CommitInfo::read(&commit, self.mailmap.as_ref())?;

            if wants_files {
                info.files = Some(match files {
                    Some(files) => files,
                    None => changed_files(self.repo, &commit, &mut self.filter, self.settings.stat)
                        .map_err(unreadable)?,
                });
            }

            Some(info)
        };

        Ok((listed, follow))
    }

    /// Makes one step of progress; see the module docs for the cases.
    ///
    /// Returns the commit when the step lists one; `None` means it moved the walk on without listing
    /// anything, so the caller calls again until `done` is set.
    fn advance(&mut self) -> Result<Option<CommitInfo>, GixError> {
        if let Some((id, parents)) = self.pending.pop() {
            let (listed, follow) = self.visit(id, &parents)?;

            if self.simplifies_history {
                for parent in follow {
                    if let Some(waiting) = self.reach.follow(parent) {
                        self.pending.push(waiting);
                    }
                }
            }

            return Ok(listed);
        }

        if self.cancelled.load(Ordering::Relaxed) {
            self.done = true;
            return Ok(None);
        }

        let Some(step) = self.steps.next() else {
            self.done = true;
            return Ok(None);
        };

        let info = step.map_err(GixError::Walk)?;

        if self.simplifies_history && !self.reach.admit(info.id, &info.parent_ids) {
            return Ok(None);
        }

        self.pending.push((info.id, info.parent_ids.to_vec()));

        Ok(None)
    }
}

impl Iterator for Selected<'_> {
    type Item = Result<CommitInfo, GixError>;

    fn next(&mut self) -> Option<Self::Item> {
        while !self.done {
            match self.advance() {
                Ok(Some(commit)) => return Some(Ok(commit)),
                Ok(None) => {}
                Err(err) => {
                    self.done = true;

                    return Some(Err(err));
                }
            }
        }

        None
    }
}
