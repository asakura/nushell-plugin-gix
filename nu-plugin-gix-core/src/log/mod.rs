//! Streaming commit history, like `git log`, as plain Rust data.
//!
//! [`log`] resolves where the history starts, walks it on a background thread, and returns
//! a [`CommitLog`], a stream of [`CommitInfo`] that stops the walk when dropped or interrupted.
//! [`pathspec`] prepares the path filter that [`log`] takes.
//!
//! # Pipeline
//!
//! ```text
//!      caller thread                       walker thread                       caller thread
//! ┌─────────────────────┐   ┌──────────────────────────────────────────┐   ┌───────────────────┐
//! │ log()               │   │ 3. rev-walk, newest first by time        │   │ 6. next() reads   │
//! │ 1. resolve tips     │   │ 4. Selected: path filter, simplification,│   │    the channel    │
//! │    (LogStart →      │──▶│    files / stat, mailmap                 │──▶│                   │
//! │    include / hide)  │   │ 5. skip, limit, then reverse; feed into  │   │ Drop or interrupt │
//! │ 2. compile filter   │   │    a bounded channel                     │   │ sets `cancelled`  │
//! └─────────────────────┘   └──────────────────────────────────────────┘   └─────────┬─────────┘
//!                                          ▲                                         │
//!                                          └──────────── cancelled ──────────────────┘
//! ```
//!
//! Steps 1 and 2 run before the thread starts, so an unknown revision or an invalid pathspec is
//! returned by [`log`] itself. Everything after that arrives as stream items.
//!
//! # Vocabulary
//!
//! - **Tip**: a commit the walk starts from (*include*) or whose ancestry it leaves out (*hide*).
//! - **Selection**: the commits that pass the path filter, before `skip`, `limit` and `reverse`.
//! - **Simplified history**: with a path filter, a commit identical to a parent at those paths is not
//!   listed and only that parent is followed, as in `git log -- <paths>`.
//! - **Followed edge**: a child-to-parent link the walk takes. Under simplification a commit is
//!   considered only once a followed edge reaches it.
//!
//! # Guarantees
//!
//! - Order is newest first by commit time, not by parentage.
//! - `skip` and `limit` count commits after path filtering; `reverse` is applied to the result and
//!   buffers the whole selection before the first item.
//! - Memory is bounded: the walker runs a fixed number of commits ahead of the consumer.
//! - Dropping the stream, or an interrupt, stops the walker even while it scans commits the filter
//!   rejects; dropping never blocks.
//! - After the first error the stream ends. A panic of the walker is a final `WalkerPanicked` item.
//!
//! # Worked example: why a commit can wait
//!
//! ```text
//!   M   merge; its tree equals A's at the filtered path
//!   ├─ A   first parent, changes src/lib.rs
//!   └─ B   second parent, changes docs/x.md
//! ```
//!
//! For `-- src/`, `M` is not listed and only `A` is followed, so `B` is never reached. When a parent
//! has a newer commit time than its child (clock skew), the walk reaches it first; it waits in
//! `Reach` and is listed right after the child that follows it.
//!
//! The walk tests guard these cases: `drops_a_side_change_that_a_merge_discards` is the merge above,
//! `default_walk_drops_a_merge_identical_to_its_second_parent` covers a merge that equals its second
//! parent, and `lists_a_commit_walked_before_the_child_that_follows_it` is the clock-skew case.
//!
//! # Where things live
//!
//! | Stage | Module |
//! |---|---|
//! | Start points | `tips` |
//! | Path filter and pathspec | `paths` |
//! | Walk loop, thread, channel | `walk` |
//! | Per-commit data | `commit` |
//! | Changed files and line counts | `changes` |
//! | Public options | `options` |

mod changes;
mod commit;
mod options;
mod paths;
mod tips;
mod walk;

pub use self::{
    changes::{FileChange, FileChangeKind, FileStat},
    commit::{CommitInfo, SignatureInfo},
    options::{LogSettings, LogStart, Mailmap},
    paths::pathspec,
    walk::{CommitLog, log},
};
