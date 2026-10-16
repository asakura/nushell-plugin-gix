//! Plugin-side cache of opened repositories, keyed by handle id.
//!
//! The registry is only a cache: an unknown id (after a plugin restart or an eviction) is
//! rebuilt from the handle's on-disk location, never reported as an error.

use super::{GixError, RepositoryLocation, open::open};

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    process,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
};

/// Entries kept before the least recently used one is evicted.
const CAPACITY: usize = 16;

/// Ids start at `pid * ID_STRIDE`, so ids from a previous plugin process rarely collide with new ones.
const ID_STRIDE: u64 = 1 << 32;

/// An opened repository shared between commands; `gix` types never leave the git side.
#[derive(Debug)]
pub struct OpenRepository {
    git_dir: PathBuf,
    repo: gix::ThreadSafeRepository,
}

impl OpenRepository {
    /// A repository handle for the calling thread.
    pub(super) fn repo(&self) -> gix::Repository {
        self.repo.to_thread_local()
    }
}

#[derive(Debug)]
struct Entry {
    id: u64,
    opened: Arc<OpenRepository>,
}

/// Bounded LRU map from handle id to [`OpenRepository`].
#[derive(Debug)]
pub struct Registry {
    next_id: AtomicU64,
    entries: Mutex<VecDeque<Entry>>,
}

impl Registry {
    /// Creates an empty registry.
    #[must_use = "an unused registry caches nothing"]
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(u64::from(process::id()).saturating_mul(ID_STRIDE)),
            entries: Mutex::new(VecDeque::new()),
        }
    }

    /// Opens the repository at `location`, caches it and returns the id of its new handle.
    ///
    /// # Errors
    ///
    /// Returns [`GixError::Open`] when the repository cannot be opened.
    pub fn register(&self, location: &RepositoryLocation) -> Result<u64, GixError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);

        self.insert(id, Arc::new(load(location)?));

        Ok(id)
    }

    /// Returns the cached repository for `id`, rebuilding it from `location` on a cache miss.
    ///
    /// # Errors
    ///
    /// Returns [`GixError::RepositoryGone`] when the git directory no longer exists, and
    /// [`GixError::Open`] when a rebuild fails.
    pub fn acquire(
        &self,
        id: u64,
        location: &RepositoryLocation,
    ) -> Result<Arc<OpenRepository>, GixError> {
        if !location.git_dir.is_dir() {
            self.release(id, &location.git_dir);

            return Err(GixError::RepositoryGone {
                git_dir: location.git_dir.clone(),
            });
        }

        if let Some(opened) = self.lookup(id, location) {
            return Ok(opened);
        }

        let opened = Arc::new(load(location)?);

        self.insert(id, Arc::clone(&opened));

        Ok(opened)
    }

    /// Drops the entry for `id` if it still caches the repository at `git_dir`.
    ///
    /// Repeated calls and calls for a handle whose id now belongs to another repository
    /// (e.g. a stale handle from an earlier plugin process) leave other entries alone.
    pub fn release(&self, id: u64, git_dir: &Path) {
        self.entries()
            .retain(|entry| !(entry.id == id && entry.opened.git_dir == git_dir));
    }

    fn entries(&self) -> MutexGuard<'_, VecDeque<Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Finds the entry for `id` that still names `location`'s git directory and marks it most recently used.
    fn lookup(&self, id: u64, location: &RepositoryLocation) -> Option<Arc<OpenRepository>> {
        let mut entries = self.entries();

        let index = entries
            .iter()
            .position(|entry| entry.id == id && entry.opened.git_dir == location.git_dir)?;
        let entry = entries.remove(index)?;
        let opened = Arc::clone(&entry.opened);

        entries.push_back(entry);
        drop(entries);

        Some(opened)
    }

    fn insert(&self, id: u64, opened: Arc<OpenRepository>) {
        let mut entries = self.entries();

        entries.retain(|entry| entry.id != id);
        entries.push_back(Entry { id, opened });

        while entries.len() > CAPACITY {
            entries.pop_front();
        }

        drop(entries);
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

fn load(location: &RepositoryLocation) -> Result<OpenRepository, GixError> {
    Ok(OpenRepository {
        git_dir: location.git_dir.clone(),
        repo: open(location.root())?.into_sync(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::discover;

    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use std::{fs, io, iter, thread};

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

    /// A temporary directory holding a freshly initialised git repository.
    #[fixture]
    fn repo_dir() -> Result<TempDir, TestError> {
        let dir = TempDir::new()?;

        gix::init(dir.path())?;

        Ok(dir)
    }

    /// A second, independent repository.
    #[fixture]
    fn other_repo_dir() -> Result<TempDir, TestError> {
        repo_dir()
    }

    fn cached(registry: &Registry) -> usize {
        registry.entries().len()
    }

    mod register {
        use super::*;

        #[rstest]
        fn caches_the_opened_repository(
            repo_dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            let dir = repo_dir?;
            let location = discover(dir.path())?;
            let registry = Registry::new();

            let id = registry.register(&location)?;

            assert!(Arc::ptr_eq(
                &registry.acquire(id, &location)?,
                &registry.acquire(id, &location)?
            ));
            assert_eq!(cached(&registry), 1);

            Ok(())
        }

        #[rstest]
        fn hands_out_distinct_ids(repo_dir: Result<TempDir, TestError>) -> Result<(), TestError> {
            let dir = repo_dir?;
            let location = discover(dir.path())?;
            let registry = Registry::new();

            assert_ne!(registry.register(&location)?, registry.register(&location)?);

            Ok(())
        }

        #[rstest]
        fn evicts_the_least_recently_used_entry(
            repo_dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            let dir = repo_dir?;
            let location = discover(dir.path())?;
            let registry = Registry::new();

            let first = registry.register(&location)?;

            for _ in 0..CAPACITY {
                registry.register(&location)?;
            }

            assert_eq!(cached(&registry), CAPACITY);
            assert!(registry.lookup(first, &location).is_none());

            Ok(())
        }
    }

    mod acquire {
        use super::*;

        #[rstest]
        fn rebuilds_when_the_id_is_unknown(
            repo_dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            let dir = repo_dir?;
            let location = discover(dir.path())?;
            let id = Registry::new().register(&location)?;
            let restarted = Registry::new();

            restarted.acquire(id, &location)?;

            assert_eq!(cached(&restarted), 1);

            Ok(())
        }

        #[rstest]
        fn rebuilds_when_the_id_names_another_repository(
            repo_dir: Result<TempDir, TestError>,
            other_repo_dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            let (one, other) = (repo_dir?, other_repo_dir?);
            let (one, other) = (discover(one.path())?, discover(other.path())?);
            let registry = Registry::new();

            let id = registry.register(&one)?;
            let rebuilt = registry.acquire(id, &other)?;

            assert_eq!(rebuilt.git_dir, other.git_dir);
            assert_eq!(cached(&registry), 1);

            Ok(())
        }

        #[rstest]
        fn reports_a_deleted_repository(
            repo_dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            let dir = repo_dir?;
            let location = discover(dir.path())?;
            let registry = Registry::new();
            let id = registry.register(&location)?;

            fs::remove_dir_all(&location.git_dir)?;

            let err = registry
                .acquire(id, &location)
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert!(
                matches!(err, GixError::RepositoryGone { .. }),
                "got {err:?}"
            );
            assert_eq!(cached(&registry), 0);

            Ok(())
        }

        #[rstest]
        fn shares_one_opened_repository_between_threads(
            repo_dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            const THREADS: usize = 8;

            let dir = repo_dir?;
            let location = discover(dir.path())?;
            let registry = Registry::new();
            let id = registry.register(&location)?;
            let expected = registry.acquire(id, &location)?;

            let shared = thread::scope(|scope| {
                let workers: Vec<_> =
                    iter::repeat_with(|| scope.spawn(|| registry.acquire(id, &location)))
                        .take(THREADS)
                        .collect();

                workers
                    .into_iter()
                    .map(thread::ScopedJoinHandle::join)
                    .collect::<Vec<_>>()
            });

            for result in shared {
                let opened =
                    result.map_err(|_panic| TestError::Failure("worker panicked".to_owned()))??;

                assert!(Arc::ptr_eq(&opened, &expected));
            }

            Ok(())
        }
    }

    mod release {
        use super::*;

        #[rstest]
        fn removes_the_entry(repo_dir: Result<TempDir, TestError>) -> Result<(), TestError> {
            let dir = repo_dir?;
            let location = discover(dir.path())?;
            let registry = Registry::new();
            let id = registry.register(&location)?;

            registry.release(id, &location.git_dir);

            assert_eq!(cached(&registry), 0);

            Ok(())
        }

        #[rstest]
        fn is_idempotent(repo_dir: Result<TempDir, TestError>) -> Result<(), TestError> {
            let dir = repo_dir?;
            let location = discover(dir.path())?;
            let registry = Registry::new();
            let id = registry.register(&location)?;

            registry.release(id, &location.git_dir);
            registry.release(id, &location.git_dir);

            assert_eq!(cached(&registry), 0);

            Ok(())
        }

        #[rstest]
        fn keeps_an_entry_that_names_another_repository(
            repo_dir: Result<TempDir, TestError>,
            other_repo_dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            let (dir, other) = (repo_dir?, other_repo_dir?);
            let (location, stale) = (discover(dir.path())?, discover(other.path())?);
            let registry = Registry::new();
            let id = registry.register(&location)?;

            registry.release(id, &stale.git_dir);

            assert_eq!(cached(&registry), 1);

            Ok(())
        }
    }
}
