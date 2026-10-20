//! Sending the items of a walk into the bounded channel that feeds a `CommitLog`.
//!
//! The channel is the backpressure: `send` blocks when the consumer is behind, so the walker stays
//! within the channel capacity (plus the item being sent) ahead of it. Guarded by
//! `stops_pulling_once_the_receiver_is_dropped`, which counts the items pulled after the receiver is
//! dropped.

use crate::{GixError, log::commit::CommitInfo};

use std::sync::mpsc::SyncSender;

/// Sends `items` into `sender` until they run out, one fails, or the receiver is dropped. Over a
/// bounded channel the producer therefore runs at most the channel's capacity ahead of the consumer.
pub(super) fn feed<I>(items: I, sender: &SyncSender<Result<CommitInfo, GixError>>)
where
    I: IntoIterator<Item = Result<CommitInfo, GixError>>,
{
    for item in items {
        let failed = item.is_err();

        if sender.send(item).is_err() || failed {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::log::SignatureInfo;

    use chrono::DateTime;
    use gix::hash::Kind;
    use nu_plugin_gix_testing_support::{COMMITTER_EMAIL, COMMITTER_NAME};
    use rstest::rstest;

    use std::{
        iter,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
    };

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Gix(#[from] GixError),
        #[error("Test failure: {0}")]
        Failure(String),
    }

    mod feed {
        use super::*;

        /// Capacity of the test channel; small, so the bound is cheap to exceed.
        const WINDOW: usize = 4;
        /// Most items `feed` may pull after one is received and the receiver dropped: the received one,
        /// a full window, and one blocked in `send`.
        const MAX_PULLED: usize = 6;
        /// Items offered to `feed`, far more than it may pull.
        const OFFERED: usize = 1000;
        /// Any representable commit time works for the sample.
        const SAMPLE_SECONDS: i64 = 1_700_000_000;

        /// A commit with placeholder fields, for feeding the channel.
        fn sample_commit() -> Result<CommitInfo, TestError> {
            let time = DateTime::from_timestamp(SAMPLE_SECONDS, 0)
                .ok_or_else(|| TestError::Failure("timestamp out of range".to_owned()))?
                .fixed_offset();

            let signature = SignatureInfo {
                name: COMMITTER_NAME.to_owned(),
                email: COMMITTER_EMAIL.to_owned(),
                time,
            };

            Ok(CommitInfo {
                oid: gix::ObjectId::null(Kind::Sha1),
                parents: Vec::new(),
                tree: gix::ObjectId::null(Kind::Sha1),
                subject: String::new(),
                body: None,
                message: String::new(),
                author: signature.clone(),
                committer: signature,
                files: None,
            })
        }

        #[rstest]
        fn stops_pulling_once_the_receiver_is_dropped() -> Result<(), TestError> {
            let sample = sample_commit()?;
            let pulled = Arc::new(AtomicUsize::new(0));
            let counter = Arc::clone(&pulled);
            let (sender, receiver) = mpsc::sync_channel(WINDOW);

            let items = iter::repeat_with(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(sample.clone())
            })
            .take(OFFERED);

            let walker = thread::spawn(move || feed(items, &sender));

            let first: CommitInfo = receiver
                .recv()
                .map_err(|error| TestError::Failure(format!("no first commit: {error}")))??;

            assert_eq!(first, sample_commit()?);

            drop(receiver);

            walker
                .join()
                .map_err(|panic| TestError::Failure(format!("walker panicked: {panic:?}")))?;

            let count = pulled.load(Ordering::SeqCst);

            assert!(
                count <= MAX_PULLED,
                "pulled {count} commits for one received"
            );

            Ok(())
        }
    }
}
