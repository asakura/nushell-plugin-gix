//! Running a commit walk on a background thread and streaming its commits through a bounded channel.
//!
//! ```text
//!  log()  ──▶  Walk::run  ──▶  rev-walk ─▶ Selected ─▶ skip/take ──▶ feed ──▶ channel ──▶ CommitLog
//!  caller      walker thread: from Walk::run to feed                                   caller
//! ```
//!
//! - `log` resolves the start points and the path filter on the caller's thread, so those errors come
//!   back from the call itself, then spawns the walker.
//! - `plan` holds the resolved walk and runs it. `selected` is the iterator that applies the path
//!   filter and history simplification, with `reach` remembering which commits followed edges reached.
//! - `feed` pushes items into a bounded channel; a full channel blocks the walker, which is what
//!   keeps memory bounded. `commit_log` is the receiving end the caller iterates.
//!
//! The walk lives on its own thread because the channel gives backpressure for free and a long scan
//! over commits that the path filter rejects never blocks the caller. Cancellation is cooperative: the
//! stream sets a shared flag when it is dropped or interrupted, and `selected` checks it between
//! walk steps.

mod commit_log;
mod feed;
mod log;
mod plan;
mod reach;
mod selected;

pub use self::{commit_log::CommitLog, log::log};
