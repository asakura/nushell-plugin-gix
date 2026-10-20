//! The stream of commits a walk yields, which stops its walker thread when dropped or interrupted.
//!
//! | Event | What the stream does |
//! |---|---|
//! | the walker sends an item | `next` checks the interrupt; if set, it cancels the walker, drops the item and ends; otherwise it returns the item |
//! | no item for `INTERRUPT_POLL` | `next` checks the interrupt; if set, it cancels the walker and ends |
//! | the stream is dropped | sets `cancelled`; the walker is not joined, so dropping never blocks |
//! | the channel closes | `next` joins the walker; a panic becomes one final `WalkerPanicked` item |
//!
//! Guarded by `reports_a_walker_panic_as_one_error`, `ends_quietly_when_the_walker_finishes`,
//! `dropping_the_log_sets_the_flag` and `an_interrupt_ends_the_wait_and_sets_the_flag`.

use crate::GixError;
use crate::log::commit::CommitInfo;

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError},
    },
    thread,
    time::Duration,
};

/// Commits buffered between the walker thread and the consumer; bounds memory and applies backpressure.
/// The walk runs at most this many commits ahead of the consumer and stops when the stream is dropped
/// or interrupted.
pub(super) const CHANNEL_CAPACITY: usize = 256;
/// How long `next` waits for a commit before it checks whether the stream was interrupted.
const INTERRUPT_POLL: Duration = Duration::from_millis(50);
pub(super) const WALKER_THREAD_NAME: &str = "gix-log-walker";

/// A lazily produced sequence of commits, newest first unless the walk is reversed.
///
/// The walk runs on a background thread feeding a bounded channel, which bounds the commits buffered
/// between walker and consumer. A walk simplified against a path filter also keeps every commit that no
/// followed edge reaches, with its parents, until the walk ends, so its memory grows with the size of the
/// pruned history. Dropping the stream stops the walk, including while it scans commits
/// that the path filter rejects. An interrupt set with [`CommitLog::with_interrupt`] ends the stream
/// the same way; it is checked on every call to `next`, and again every `INTERRUPT_POLL` while `next`
/// waits for a commit. After the
/// first error the stream ends. A panic of the walker thread is reported as a final
/// [`GixError::WalkerPanicked`] item, so a truncated history never looks complete. A reversed walk
/// buffers its whole selection before yielding the first commit. Under clock skew a commit reached
/// late is listed right after the child that follows it.
#[expect(
    clippy::field_scoped_visibility_modifiers,
    reason = "the walk tests and `log` build the stream from its fields"
)]
pub struct CommitLog {
    pub(super) receiver: Receiver<Result<CommitInfo, GixError>>,
    /// The walker thread, kept until it is joined after the channel closes.
    pub(super) walker: Option<thread::JoinHandle<()>>,
    /// Set when the stream is dropped or interrupted, so the walker stops scanning.
    pub(super) cancelled: Arc<AtomicBool>,
    /// Checked on every call to `next` and while it waits for a commit; `true` ends the stream.
    pub(super) interrupted: Option<Box<dyn Fn() -> bool + Send>>,
}

impl CommitLog {
    /// Ends the stream as soon as `interrupted` returns `true`: checked on every call to `next` and while
    /// it waits for a commit, so a caller's cancellation (such as Ctrl-C) is seen even when no commit
    /// matches for a long time, and even when the walker never pauses.
    #[must_use = "the stream only gains the interrupt when the returned value is kept"]
    pub fn with_interrupt<F>(mut self, interrupted: F) -> Self
    where
        F: Fn() -> bool + Send + 'static,
    {
        self.interrupted = Some(Box::new(interrupted));

        self
    }

    /// Whether the interrupt installed with [`CommitLog::with_interrupt`] reports a request to stop.
    fn is_interrupted(&self) -> bool {
        self.interrupted
            .as_ref()
            .is_some_and(|interrupted| interrupted())
    }
}

impl fmt::Debug for CommitLog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommitLog")
            .field("receiver", &self.receiver)
            .field("walker", &self.walker)
            .field("cancelled", &self.cancelled)
            .finish_non_exhaustive()
    }
}

impl Drop for CommitLog {
    /// Tells the walker to stop scanning; it is not joined, so dropping a stream never blocks.
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl Iterator for CommitLog {
    type Item = Result<CommitInfo, GixError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.receiver.recv_timeout(INTERRUPT_POLL) {
                Ok(item) => {
                    if self.is_interrupted() {
                        self.cancelled.store(true, Ordering::Relaxed);
                        return None;
                    }

                    return Some(item);
                }
                Err(RecvTimeoutError::Timeout) => {
                    if self.is_interrupted() {
                        self.cancelled.store(true, Ordering::Relaxed);
                        return None;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return self
                        .walker
                        .take()
                        .and_then(|handle| handle.join().err())
                        .map(|_| Err(GixError::WalkerPanicked));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use rstest::rstest;

    use std::{
        assert_matches,
        sync::mpsc::{self, SyncSender},
    };

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error("Test failure: {0}")]
        Failure(String),
    }

    mod commit_log {
        use super::*;

        /// Starts `walk` on a thread with the sender of a fresh channel, as `log` does, so the sender is
        /// dropped when the thread ends, whether it returns or panics.
        fn log_with_walker<W>(walk: W) -> CommitLog
        where
            W: FnOnce(SyncSender<Result<CommitInfo, GixError>>) + Send + 'static,
        {
            let (sender, receiver) = mpsc::sync_channel(CHANNEL_CAPACITY);
            let walker = thread::spawn(move || walk(sender));

            CommitLog {
                receiver,
                walker: Some(walker),
                cancelled: Arc::new(AtomicBool::new(false)),
                interrupted: None,
            }
        }

        /// A walker that dies before it sends anything.
        #[expect(
            clippy::panic,
            reason = "the walker thread must panic to exercise the join that reports it"
        )]
        fn panicking_walk(_sender: SyncSender<Result<CommitInfo, GixError>>) {
            panic!("walker failed");
        }

        /// A walker that finishes without sending anything.
        fn finishing_walk(sender: SyncSender<Result<CommitInfo, GixError>>) {
            drop(sender);
        }

        #[rstest]
        fn reports_a_walker_panic_as_one_error() -> Result<(), TestError> {
            let mut stream = log_with_walker(panicking_walk);

            let first = stream
                .next()
                .ok_or_else(|| TestError::Failure("stream ended without an error".to_owned()))?;

            assert_matches!(first, Err(GixError::WalkerPanicked));
            assert!(stream.next().is_none());

            Ok(())
        }

        #[rstest]
        fn ends_quietly_when_the_walker_finishes() -> Result<(), TestError> {
            let mut stream = log_with_walker(finishing_walk);

            if let Some(item) = stream.next() {
                return Err(TestError::Failure(format!(
                    "expected no item, got {item:?}"
                )));
            }

            Ok(())
        }
    }

    mod cancellation {
        use super::*;

        /// A stream with no walker thread, and the sender that keeps its channel open.
        fn silent_log() -> (SyncSender<Result<CommitInfo, GixError>>, CommitLog) {
            let (sender, receiver) = mpsc::sync_channel(CHANNEL_CAPACITY);

            let stream = CommitLog {
                receiver,
                walker: None,
                cancelled: Arc::new(AtomicBool::new(false)),
                interrupted: None,
            };

            (sender, stream)
        }

        /// A stream fed by a walker that never pauses, so `next` always has an item to return.
        fn busy_log() -> CommitLog {
            let (sender, receiver) = mpsc::sync_channel(CHANNEL_CAPACITY);

            drop(thread::spawn(move || {
                while sender.send(Err(GixError::WalkerPanicked)).is_ok() {}
            }));

            CommitLog {
                receiver,
                walker: None,
                cancelled: Arc::new(AtomicBool::new(false)),
                interrupted: None,
            }
        }

        #[rstest]
        fn dropping_the_log_sets_the_flag() {
            let (_sender, stream) = silent_log();
            let flag = Arc::clone(&stream.cancelled);

            drop(stream);

            assert!(flag.load(Ordering::Relaxed));
        }

        #[rstest]
        fn an_interrupt_ends_the_wait_and_sets_the_flag() {
            let (_sender, stream) = silent_log();
            let mut stream = stream.with_interrupt(|| true);
            let flag = Arc::clone(&stream.cancelled);

            assert!(stream.next().is_none());
            assert!(flag.load(Ordering::Relaxed));
        }

        #[rstest]
        fn an_interrupt_ends_a_stream_whose_walker_never_pauses() {
            let mut stream = busy_log().with_interrupt(|| true);
            let flag = Arc::clone(&stream.cancelled);

            assert!(stream.next().is_none());
            assert!(flag.load(Ordering::Relaxed));
        }
    }
}
