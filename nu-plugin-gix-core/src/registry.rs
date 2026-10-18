//! Plugin-side cache of opened repositories, keyed by handle id.
//!
//! The registry is only a cache: an unknown id (after a plugin restart or an eviction) is
//! rebuilt from the handle's on-disk location, never reported as an error.

use super::{GixError, RepositoryLocation, repository::OpenRepository};

use std::{
    collections::VecDeque,
    path::Path,
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

#[derive(Debug)]
struct Entry {
    id: u64,
    opened: Arc<OpenRepository>,
}

impl Entry {
    /// Whether this entry caches the repository at `git_dir` for handle `id`.
    fn names(&self, id: u64, git_dir: &Path) -> bool {
        self.id == id && self.opened.git_dir() == git_dir
    }
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
        Ok(self.cache(OpenRepository::open(location)?))
    }

    /// Discovers the repository containing `path`, caches the repository that discovery opened,
    /// and returns the id of its new handle together with its location.
    ///
    /// The repository is opened once: the cached entry is the one discovery produced, not a second open.
    ///
    /// # Errors
    ///
    /// Returns [`GixError::NotFound`] when no repository contains `path`, and [`GixError::Open`]
    /// when one is found but cannot be opened.
    pub fn discover_and_register(
        &self,
        path: &Path,
    ) -> Result<(u64, RepositoryLocation), GixError> {
        let (opened, location) = OpenRepository::discover(path)?;

        Ok((self.cache(opened), location))
    }

    /// Returns the cached repository for `id`, rebuilding it from `location` on a cache miss.
    ///
    /// The repository is opened outside the cache lock, so concurrent misses for the same id may
    /// each open it; the last one cached wins, and every caller still gets a working handle.
    ///
    /// # Errors
    ///
    /// Returns [`GixError::RepositoryGone`] when the repository or its worktree no longer exists,
    /// or the path no longer holds the repository `location` names, and [`GixError::Open`] when a
    /// rebuild fails.
    pub fn acquire(
        &self,
        id: u64,
        location: &RepositoryLocation,
    ) -> Result<Arc<OpenRepository>, GixError> {
        if !location.exists() {
            self.release(id, &location.git_dir);

            return Err(GixError::RepositoryGone {
                git_dir: location.git_dir.clone(),
            });
        }

        if let Some(opened) = self.lookup(id, location) {
            return Ok(opened);
        }

        let opened = Arc::new(OpenRepository::open(location)?);

        self.insert(id, Arc::clone(&opened));

        Ok(opened)
    }

    /// Drops the entry for `id` if it still caches the repository at `git_dir`.
    ///
    /// Repeated calls and calls for a handle whose id now belongs to another repository
    /// (e.g. a stale handle from an earlier plugin process) leave other entries alone.
    pub fn release(&self, id: u64, git_dir: &Path) {
        self.entries().retain(|entry| !entry.names(id, git_dir));
    }

    /// Stores `opened` under a fresh id and returns that id.
    fn cache(&self, opened: OpenRepository) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);

        self.insert(id, Arc::new(opened));

        id
    }

    fn entries(&self) -> MutexGuard<'_, VecDeque<Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Finds the entry for `id` that still names `location`'s git directory and marks it most recently used.
    fn lookup(&self, id: u64, location: &RepositoryLocation) -> Option<Arc<OpenRepository>> {
        let mut entries = self.entries();

        let index = entries
            .iter()
            .position(|entry| entry.names(id, &location.git_dir))?;

        let entry = entries.remove(index)?;
        let opened = Arc::clone(&entry.opened);

        entries.push_back(entry);
        drop(entries);

        Some(opened)
    }

    /// Stores `opened` under `id` as the most recently used entry. Any existing entry with the same
    /// id is replaced, even when it names another repository; then the least recently used entries
    /// are evicted until no more than `CAPACITY` remain.
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

#[cfg(test)]
mod tests {
    use super::*;

    use nu_plugin_gix_testing_support::{
        CorruptTestRepo, FixtureError, GIT_DIR, HEAD_FILE, MAIN_BRANCH, UnbornTestRepo,
        corrupt_repo, plain_dir, unborn_repo,
    };
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use std::{assert_matches, fs, io, iter, sync::Barrier, thread};

    const THREADS: usize = 8;

    /// Directory inside the main git directory that holds one private git directory per linked worktree.
    const WORKTREES_DIR: &str = "worktrees";

    /// Name of the directory a linked worktree is created in, next to the main worktree.
    const LINKED_WORKTREE_NAME: &str = "linked";

    /// File in a linked worktree's git directory that points back at the main git directory.
    const COMMON_DIR_FILE: &str = "commondir";

    /// Contents of [`COMMON_DIR_FILE`]: the main git directory, relative to the linked git directory.
    const COMMON_DIR_TARGET: &str = "../..";

    /// File in a linked worktree's git directory that records the path of the worktree's `.git` file.
    const GITDIR_FILE: &str = "gitdir";

    /// Prefix of a `.git` file that points at the git directory of a linked worktree.
    const GITDIR_POINTER_PREFIX: &str = "gitdir: ";

    /// Prefix of a `HEAD` file that names the branch it points at.
    const SYMBOLIC_REF_PREFIX: &str = "ref: ";

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Io(#[from] io::Error),

        #[error(transparent)]
        Git(#[from] GixError),

        #[error(transparent)]
        Fixture(#[from] FixtureError),

        #[error("Test failure: {0}")]
        Failure(String),
    }

    /// A registry and the location of a fresh repository, with the directory that keeps it alive.
    #[fixture]
    fn registry_with_repo(
        unborn_repo: Result<UnbornTestRepo, FixtureError>,
    ) -> Result<(TempDir, RepositoryLocation, Registry), TestError> {
        let dir = unborn_repo?.dir;
        let location = RepositoryLocation::discover(dir.path())?;

        Ok((dir, location, Registry::new()))
    }

    /// A linked worktree of a fresh repository, laid out as `git worktree add` lays it out.
    ///
    /// Its directory can be deleted while its private git directory under the main repository
    /// remains, and the returned `TempDir` keeps both alive.
    #[fixture]
    fn linked_worktree(
        unborn_repo: Result<UnbornTestRepo, FixtureError>,
    ) -> Result<(TempDir, RepositoryLocation), TestError> {
        let dir = unborn_repo?.dir;
        let worktree = dir.path().join(LINKED_WORKTREE_NAME);
        let git_dir = dir
            .path()
            .join(GIT_DIR)
            .join(WORKTREES_DIR)
            .join(LINKED_WORKTREE_NAME);

        fs::create_dir_all(&worktree)?;
        fs::create_dir_all(&git_dir)?;
        fs::write(
            git_dir.join(COMMON_DIR_FILE),
            format!("{COMMON_DIR_TARGET}\n"),
        )?;
        fs::write(
            git_dir.join(GITDIR_FILE),
            format!("{}\n", worktree.join(GIT_DIR).display()),
        )?;
        fs::write(
            git_dir.join(HEAD_FILE),
            format!("{SYMBOLIC_REF_PREFIX}{MAIN_BRANCH}\n"),
        )?;
        fs::write(
            worktree.join(GIT_DIR),
            format!("{GITDIR_POINTER_PREFIX}{}\n", git_dir.display()),
        )?;

        let location = RepositoryLocation::discover(&worktree)?;

        Ok((dir, location))
    }

    fn entry_count(registry: &Registry) -> usize {
        registry.entries().len()
    }

    mod discover_and_register {
        use super::*;

        #[rstest]
        fn returns_the_location_of_the_repository(
            unborn_repo: Result<UnbornTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let dir = unborn_repo?.dir;
            let (_, location) = Registry::new().discover_and_register(dir.path())?;

            assert_eq!(location, RepositoryLocation::discover(dir.path())?);

            Ok(())
        }

        #[rstest]
        fn caches_the_repository_discovery_opened(
            unborn_repo: Result<UnbornTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let dir = unborn_repo?.dir;
            let registry = Registry::new();
            let (id, location) = registry.discover_and_register(dir.path())?;

            assert_eq!(entry_count(&registry), 1);

            // A rebuild would reopen the repository, which fails once its HEAD file is gone.
            fs::remove_file(location.git_dir.join(HEAD_FILE))?;

            assert!(
                Registry::new().acquire(id, &location).is_err(),
                "a fresh registry must not be able to rebuild the repository"
            );

            let first = registry.acquire(id, &location)?;
            let second = registry.acquire(id, &location)?;

            assert!(Arc::ptr_eq(&first, &second));

            Ok(())
        }

        #[rstest]
        fn reports_a_missing_path_as_not_found(
            plain_dir: Result<TempDir, FixtureError>,
        ) -> Result<(), TestError> {
            let dir = plain_dir?;
            let registry = Registry::new();

            // Missing, so discovery fails before it searches any parent directory.
            let err = registry
                .discover_and_register(&dir.path().join("missing"))
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert_matches!(err, GixError::NotFound { .. }, "got {err:?}");
            assert_eq!(entry_count(&registry), 0);

            Ok(())
        }

        #[rstest]
        fn reports_a_corrupt_repository_as_open_failure(
            corrupt_repo: Result<CorruptTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let dir = corrupt_repo?.dir;
            let registry = Registry::new();

            let err = registry
                .discover_and_register(dir.path())
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert_matches!(err, GixError::Open { .. }, "got {err:?}");
            assert_eq!(entry_count(&registry), 0);

            Ok(())
        }
    }

    mod register {
        use super::*;

        #[rstest]
        fn caches_the_opened_repository(
            registry_with_repo: Result<(TempDir, RepositoryLocation, Registry), TestError>,
        ) -> Result<(), TestError> {
            let (_dir, location, registry) = registry_with_repo?;

            let id = registry.register(&location)?;

            assert_eq!(entry_count(&registry), 1);

            // A rebuild would reopen the repository, which fails once its HEAD file is gone.
            fs::remove_file(location.git_dir.join(HEAD_FILE))?;

            assert!(
                Registry::new().acquire(id, &location).is_err(),
                "a fresh registry must not be able to rebuild the repository"
            );

            assert!(Arc::ptr_eq(
                &registry.acquire(id, &location)?,
                &registry.acquire(id, &location)?
            ));

            assert_eq!(entry_count(&registry), 1);

            Ok(())
        }

        #[rstest]
        fn hands_out_distinct_ids(
            registry_with_repo: Result<(TempDir, RepositoryLocation, Registry), TestError>,
        ) -> Result<(), TestError> {
            let (_dir, location, registry) = registry_with_repo?;

            let first_id = registry.register(&location)?;
            let second_id = registry.register(&location)?;

            assert_ne!(first_id, second_id);
            assert!(!Arc::ptr_eq(
                &registry.acquire(first_id, &location)?,
                &registry.acquire(second_id, &location)?
            ));

            Ok(())
        }

        #[rstest]
        fn evicts_the_least_recently_used_entry(
            registry_with_repo: Result<(TempDir, RepositoryLocation, Registry), TestError>,
        ) -> Result<(), TestError> {
            let (_dir, location, registry) = registry_with_repo?;

            let ids = iter::repeat_with(|| registry.register(&location))
                .take(CAPACITY)
                .collect::<Result<Vec<u64>, GixError>>()?;

            let first = ids
                .first()
                .copied()
                .ok_or_else(|| TestError::Failure("no ids registered".to_owned()))?;

            let second = ids
                .get(1)
                .copied()
                .ok_or_else(|| TestError::Failure("only one id registered".to_owned()))?;

            // Using the oldest entry makes the second oldest the least recently used one.
            assert!(registry.lookup(first, &location).is_some());

            registry.register(&location)?;

            assert_eq!(entry_count(&registry), CAPACITY);

            assert!(
                registry.lookup(first, &location).is_some(),
                "the recently used entry must survive the overflow"
            );

            assert!(
                registry.lookup(second, &location).is_none(),
                "the least recently used entry must be evicted"
            );

            Ok(())
        }
    }

    mod acquire {
        use super::*;

        #[rstest]
        fn rebuilds_when_the_id_is_unknown(
            unborn_repo: Result<UnbornTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let dir = unborn_repo?.dir;
            let location = RepositoryLocation::discover(dir.path())?;
            let id = Registry::new().register(&location)?;
            let restarted = Registry::new();

            restarted.acquire(id, &location)?;

            assert_eq!(entry_count(&restarted), 1);

            Ok(())
        }

        #[rstest]
        fn replaces_the_entry_when_the_id_names_another_repository() -> Result<(), TestError> {
            let (one, other) = (unborn_repo()?.dir, unborn_repo()?.dir);
            let registry = Registry::new();
            let one = RepositoryLocation::discover(one.path())?;
            let other = RepositoryLocation::discover(other.path())?;
            let id = registry.register(&one)?;
            let rebuilt = registry.acquire(id, &other)?;

            assert_eq!(rebuilt.git_dir(), other.git_dir);
            assert!(registry.lookup(id, &one).is_none());
            assert_eq!(entry_count(&registry), 1);

            Ok(())
        }

        #[rstest]
        fn reports_a_deleted_repository(
            registry_with_repo: Result<(TempDir, RepositoryLocation, Registry), TestError>,
        ) -> Result<(), TestError> {
            let (_dir, location, registry) = registry_with_repo?;
            let id = registry.register(&location)?;

            fs::remove_dir_all(&location.git_dir)?;

            let err = registry
                .acquire(id, &location)
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert_matches!(err, GixError::RepositoryGone { .. }, "got {err:?}");

            assert_eq!(entry_count(&registry), 0);

            Ok(())
        }

        #[rstest]
        fn reports_a_deleted_worktree_on_a_cache_hit(
            linked_worktree: Result<(TempDir, RepositoryLocation), TestError>,
        ) -> Result<(), TestError> {
            let (_dir, location) = linked_worktree?;
            let registry = Registry::new();
            let id = registry.register(&location)?;

            fs::remove_dir_all(location.root())?;

            let err = registry
                .acquire(id, &location)
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert_matches!(err, GixError::RepositoryGone { .. }, "got {err:?}");
            assert_eq!(entry_count(&registry), 0);

            Ok(())
        }

        #[rstest]
        fn reports_a_deleted_worktree_on_a_cache_miss(
            linked_worktree: Result<(TempDir, RepositoryLocation), TestError>,
        ) -> Result<(), TestError> {
            let (_dir, location) = linked_worktree?;
            let id = Registry::new().register(&location)?;
            let restarted = Registry::new();

            fs::remove_dir_all(location.root())?;

            let err = restarted
                .acquire(id, &location)
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert_matches!(err, GixError::RepositoryGone { .. }, "got {err:?}");
            assert_eq!(entry_count(&restarted), 0);

            Ok(())
        }

        #[rstest]
        fn shares_the_cached_repository_between_threads(
            unborn_repo: Result<UnbornTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let dir = unborn_repo?.dir;
            let location = RepositoryLocation::discover(dir.path())?;
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

        #[rstest]
        fn concurrent_misses_name_the_same_repository(
            unborn_repo: Result<UnbornTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let dir = unborn_repo?.dir;
            let location = RepositoryLocation::discover(dir.path())?;
            let id = Registry::new().register(&location)?;
            let restarted = Registry::new();
            let barrier = Barrier::new(THREADS);

            let results = thread::scope(|scope| {
                let workers: Vec<_> = iter::repeat_with(|| {
                    scope.spawn(|| {
                        barrier.wait();

                        restarted.acquire(id, &location)
                    })
                })
                .take(THREADS)
                .collect();

                workers
                    .into_iter()
                    .map(thread::ScopedJoinHandle::join)
                    .collect::<Vec<_>>()
            });

            for result in results {
                let opened =
                    result.map_err(|_panic| TestError::Failure("worker panicked".to_owned()))??;

                assert_eq!(opened.git_dir(), location.git_dir);
            }

            assert_eq!(entry_count(&restarted), 1);

            Ok(())
        }
    }

    mod release {
        use super::*;

        #[rstest]
        fn removes_the_entry(
            registry_with_repo: Result<(TempDir, RepositoryLocation, Registry), TestError>,
        ) -> Result<(), TestError> {
            let (_dir, location, registry) = registry_with_repo?;
            let id = registry.register(&location)?;

            registry.release(id, &location.git_dir);

            assert_eq!(entry_count(&registry), 0);

            Ok(())
        }

        #[rstest]
        fn is_idempotent(
            registry_with_repo: Result<(TempDir, RepositoryLocation, Registry), TestError>,
        ) -> Result<(), TestError> {
            let (_dir, location, registry) = registry_with_repo?;
            let id = registry.register(&location)?;

            registry.release(id, &location.git_dir);
            registry.release(id, &location.git_dir);

            assert_eq!(entry_count(&registry), 0);

            Ok(())
        }

        #[rstest]
        fn keeps_an_entry_that_names_another_repository() -> Result<(), TestError> {
            let (dir, other) = (unborn_repo()?.dir, unborn_repo()?.dir);
            let (location, stale) = (
                RepositoryLocation::discover(dir.path())?,
                RepositoryLocation::discover(other.path())?,
            );
            let registry = Registry::new();
            let id = registry.register(&location)?;

            registry.release(id, &stale.git_dir);

            assert_eq!(entry_count(&registry), 1);

            Ok(())
        }
    }
}
