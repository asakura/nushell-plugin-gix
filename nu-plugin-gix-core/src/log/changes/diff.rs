//! Lists the files a commit changes against its first parent, with the kind and line counts of each.

use super::types::{FileChange, FileChangeKind, FileStat};
use crate::log::paths::PathFilter;

use gix::{
    bstr::{BStr, ByteSlice as _},
    object::tree::diff::Change,
};

use std::ops::ControlFlow;

/// The kind of a non-tree `change`; rewrites are off, so a rewrite is never reported and counts as a modification.
const fn change_kind(change: &Change<'_, '_, '_>) -> FileChangeKind {
    match change {
        Change::Addition {
            location: _location,
            entry_mode: _entry_mode,
            relation: _relation,
            id: _id,
        } => FileChangeKind::Addition,
        Change::Deletion {
            location: _location,
            entry_mode: _entry_mode,
            relation: _relation,
            id: _id,
        } => FileChangeKind::Deletion,
        _ => FileChangeKind::Modification,
    }
}

/// Calls `visit` with the path and kind of each non-directory entry that differs between `old` and `new` until it
/// breaks; renames are not tracked, so a move is a deletion and an addition.
pub(in crate::log) fn for_each_change<F>(
    old: &gix::Tree<'_>,
    new: &gix::Tree<'_>,
    mut visit: F,
) -> Result<(), gix::Error>
where
    F: FnMut(&BStr, FileChangeKind) -> ControlFlow<()>,
{
    let mut platform = old.changes()?;

    platform.options(|options| {
        options.track_path().track_rewrites(None);
    });

    let mut stopped = false;

    let outcome = platform.for_each_to_obtain_tree(new, |change| {
        if change.entry_mode().is_tree() {
            return Ok(ControlFlow::Continue(()));
        }

        let flow = visit(change.location(), change_kind(&change));

        stopped = flow.is_break();

        Ok(flow)
    });

    match outcome {
        Ok(_) => Ok(()),
        // Only the cancellation caused by the visitor's own `ControlFlow::Break` counts as success; any other error is
        // a real failure even when the visitor had stopped.
        Err(err) if stopped && err.is_cancelled() => Ok(()),
        Err(err) => Err(err),
    }
}

/// The files that the trees `old` and `new` differ in, keeping those that `filter` matches, each with its line counts.
///
/// Every non-tree change is diffed through one resource cache that is cleared after each change, as gix's
/// `Platform::stats` does. A blob that cannot be read or diffed does not end the listing: its file is returned
/// with [`FileStat::Unavailable`]. Failures of the tree diff itself and of the cache are returned.
fn changed_files_with_stats(
    repo: &gix::Repository,
    old: &gix::Tree<'_>,
    new: &gix::Tree<'_>,
    filter: &mut PathFilter,
) -> Result<Vec<FileChange>, gix::Error> {
    let mut platform = old.changes()?;

    platform.options(|options| {
        options.track_path().track_rewrites(None);
    });

    let mut cache = repo.diff_resource_cache_for_tree_diff()?;
    let mut files = Vec::new();

    platform.for_each_to_obtain_tree(new, |change| {
        if change.entry_mode().is_tree() || !filter.matches(change.location()) {
            return Ok(ControlFlow::Continue(()));
        }

        let stat = change
            .diff(&mut cache)
            .map_or(FileStat::Unavailable, |mut blob_diff| {
                blob_diff
                    .line_counts()
                    .map_or(FileStat::Unavailable, |counts| {
                        counts.map_or(FileStat::Binary, |counts| FileStat::Text {
                            insertions: counts.insertions,
                            deletions: counts.removals,
                        })
                    })
            });

        cache.clear_resource_cache_keep_allocation();

        files.push(FileChange {
            path: change.location().to_str_lossy().into_owned(),
            kind: change_kind(&change),
            stat: Some(stat),
        });

        Ok(ControlFlow::Continue(()))
    })?;

    Ok(files)
}

/// The files `commit` changes compared with its first parent, or with the empty tree for a root commit, keeping
/// those that `filter` matches; in the order the tree diff reports them. With `stat` each file carries its line
/// counts, otherwise `stat` is `None`.
pub(in crate::log) fn changed_files(
    repo: &gix::Repository,
    commit: &gix::Commit<'_>,
    filter: &mut PathFilter,
    stat: bool,
) -> Result<Vec<FileChange>, gix::Error> {
    let ours = commit.tree()?;

    let theirs = match commit.parent_ids().next() {
        Some(parent) => repo.find_commit(parent.detach())?.tree()?,
        None => repo.empty_tree(),
    };

    changed_files_between(repo, &theirs, &ours, filter, stat)
}

/// The files that the trees `old` and `new` differ in, keeping those that `filter` matches; see [`changed_files`]
/// for the order and for `stat`.
pub(in crate::log) fn changed_files_between(
    repo: &gix::Repository,
    old: &gix::Tree<'_>,
    new: &gix::Tree<'_>,
    filter: &mut PathFilter,
    stat: bool,
) -> Result<Vec<FileChange>, gix::Error> {
    if stat {
        return changed_files_with_stats(repo, old, new, filter);
    }

    let mut files = Vec::new();

    for_each_change(old, new, |path, kind| {
        if filter.matches(path) {
            files.push(FileChange {
                path: path.to_str_lossy().into_owned(),
                kind,
                stat: None,
            });
        }

        ControlFlow::Continue(())
    })?;

    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{GixError, fixtures::open_path, log::paths::pathspec};

    use nu_plugin_gix_testing_support::{
        BINARY_FILE, BinaryTestRepo, DiscardedSideTestRepo, EDITED_CONTENT, FixtureError,
        FixtureRepo as _, PATHS_FILE, PATHS_NESTED_FILE, PathsTestRepo, SecondParentWinsTestRepo,
        binary_repo, discarded_side_repo, paths_repo, remove_loose_object, second_parent_wins_repo,
    };
    use rstest::rstest;

    use std::{io, path::Path};

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Gix(#[from] gix::Error),

        #[error(transparent)]
        Git(#[from] GixError),

        #[error(transparent)]
        Fixture(#[from] FixtureError),

        #[error(transparent)]
        Io(#[from] io::Error),
    }

    mod for_each_change {
        use super::*;

        #[derive(Clone, Copy, Debug)]
        enum Commit {
            AddFile,
            AddDir,
            EditFile,
            EditNested,
            Unchanged,
        }

        impl Commit {
            fn id(self, fixture: &PathsTestRepo) -> gix::ObjectId {
                match self {
                    Self::AddFile => fixture.add_file,
                    Self::AddDir => fixture.add_dir,
                    Self::EditFile => fixture.edit_file,
                    Self::EditNested => fixture.edit_nested,
                    Self::Unchanged => fixture.unchanged,
                }
            }
        }

        /// The changed files between the trees of `old` and `new`, in the order they are visited.
        fn changes(
            fixture: &PathsTestRepo,
            old: Commit,
            new: Commit,
        ) -> Result<Vec<(String, FileChangeKind)>, TestError> {
            let repo = open_path(fixture.path())?.repo();
            let old_tree = repo.find_commit(old.id(fixture))?.tree()?;
            let new_tree = repo.find_commit(new.id(fixture))?.tree()?;
            let mut found = Vec::new();

            for_each_change(&old_tree, &new_tree, |path, kind| {
                found.push((path.to_str_lossy().into_owned(), kind));
                ControlFlow::Continue(())
            })?;

            Ok(found)
        }

        #[rstest]
        #[case::added_below_a_directory(Commit::AddFile, Commit::AddDir, &[(PATHS_NESTED_FILE, FileChangeKind::Addition)])]
        #[case::modified(Commit::AddDir, Commit::EditFile, &[(PATHS_FILE, FileChangeKind::Modification)])]
        #[case::deleted(Commit::AddDir, Commit::AddFile, &[(PATHS_NESTED_FILE, FileChangeKind::Deletion)])]
        #[case::unchanged(Commit::EditNested, Commit::Unchanged, &[])]
        fn reports_each_changed_file_with_its_kind(
            paths_repo: Result<PathsTestRepo, FixtureError>,
            #[case] old: Commit,
            #[case] new: Commit,
            #[case] expected: &[(&str, FileChangeKind)],
        ) -> Result<(), TestError> {
            let fixture = paths_repo?;

            let expected: Vec<(String, FileChangeKind)> = expected
                .iter()
                .map(|&(path, kind)| (path.to_owned(), kind))
                .collect();

            assert_eq!(changes(&fixture, old, new)?, expected);

            Ok(())
        }
    }

    mod stat {
        use super::*;

        /// The commit of `paths_repo` that a case lists the changes of.
        #[derive(Clone, Copy, Debug)]
        enum Commit {
            AddFile,
            EditFile,
        }

        impl Commit {
            fn id(self, fixture: &PathsTestRepo) -> gix::ObjectId {
                match self {
                    Self::AddFile => fixture.add_file,
                    Self::EditFile => fixture.edit_file,
                }
            }
        }

        /// The changes that the commit `commit` makes against its first parent, with a stat when `stat` is set.
        fn changed_in(
            path: &Path,
            commit: gix::ObjectId,
            stat: bool,
        ) -> Result<Vec<FileChange>, TestError> {
            let repo = open_path(path)?.repo();
            let commit = repo.find_commit(commit)?;

            Ok(changed_files(
                &repo,
                &commit,
                &mut PathFilter::default(),
                stat,
            )?)
        }

        /// The changes between the trees of the commits `old` and `new`, with a stat for each.
        fn changed_between(
            fixture: &PathsTestRepo,
            old: gix::ObjectId,
            new: gix::ObjectId,
        ) -> Result<Vec<FileChange>, TestError> {
            let repo = open_path(fixture.path())?.repo();
            let old_tree = repo.find_commit(old)?.tree()?;
            let new_tree = repo.find_commit(new)?.tree()?;

            Ok(changed_files_with_stats(
                &repo,
                &old_tree,
                &new_tree,
                &mut PathFilter::default(),
            )?)
        }

        #[rstest]
        #[case::added_file(Commit::AddFile, FileChangeKind::Addition, FileStat::Text { insertions: 1, deletions: 0 })]
        #[case::modified_file(Commit::EditFile, FileChangeKind::Modification, FileStat::Text { insertions: 1, deletions: 1 })]
        fn counts_the_lines_of_a_changed_file(
            paths_repo: Result<PathsTestRepo, FixtureError>,
            #[case] commit: Commit,
            #[case] kind: FileChangeKind,
            #[case] stat: FileStat,
        ) -> Result<(), TestError> {
            let fixture = paths_repo?;

            let expected = vec![FileChange {
                path: PATHS_FILE.to_owned(),
                kind,
                stat: Some(stat),
            }];

            assert_eq!(
                changed_in(fixture.path(), commit.id(&fixture), true)?,
                expected
            );

            Ok(())
        }

        #[rstest]
        fn counts_every_line_of_a_deleted_file(
            paths_repo: Result<PathsTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = paths_repo?;

            let expected = vec![FileChange {
                path: PATHS_NESTED_FILE.to_owned(),
                kind: FileChangeKind::Deletion,
                stat: Some(FileStat::Text {
                    insertions: 0,
                    deletions: 1,
                }),
            }];

            assert_eq!(
                changed_between(&fixture, fixture.add_dir, fixture.add_file)?,
                expected
            );

            Ok(())
        }

        #[rstest]
        fn leaves_the_stat_unset_when_not_requested(
            paths_repo: Result<PathsTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = paths_repo?;

            let expected = vec![FileChange {
                path: PATHS_FILE.to_owned(),
                kind: FileChangeKind::Addition,
                stat: None,
            }];

            assert_eq!(
                changed_in(fixture.path(), fixture.add_file, false)?,
                expected
            );

            Ok(())
        }

        #[rstest]
        fn reports_a_file_with_a_nul_byte_as_binary(
            binary_repo: Result<BinaryTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = binary_repo?;

            let expected = vec![FileChange {
                path: BINARY_FILE.to_owned(),
                kind: FileChangeKind::Addition,
                stat: Some(FileStat::Binary),
            }];

            assert_eq!(changed_in(fixture.path(), fixture.commit, true)?, expected);

            Ok(())
        }

        #[rstest]
        fn compares_a_merge_with_its_first_parent(
            second_parent_wins_repo: Result<SecondParentWinsTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = second_parent_wins_repo?;
            let repo = open_path(fixture.path())?.repo();
            let merge = repo.find_commit(fixture.merge)?;

            let expected = vec![FileChange {
                path: PATHS_FILE.to_owned(),
                kind: FileChangeKind::Modification,
                stat: None,
            }];

            assert_eq!(
                changed_files(&repo, &merge, &mut PathFilter::default(), false)?,
                expected
            );

            Ok(())
        }

        #[rstest]
        fn limits_the_files_to_the_paths(
            discarded_side_repo: Result<DiscardedSideTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = discarded_side_repo?;
            let repo = open_path(fixture.path())?.repo();
            let root = repo.find_commit(fixture.root)?;
            let spec = pathspec(Some(fixture.path()), fixture.path(), PATHS_FILE)?;
            let mut filter = PathFilter::new(&[spec])?;

            let expected = vec![FileChange {
                path: PATHS_FILE.to_owned(),
                kind: FileChangeKind::Addition,
                stat: None,
            }];

            assert_eq!(changed_files(&repo, &root, &mut filter, false)?, expected);

            Ok(())
        }

        #[rstest]
        fn reports_a_file_whose_blob_is_unreadable_as_unavailable(
            paths_repo: Result<PathsTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let fixture = paths_repo?;
            let repo = open_path(fixture.path())?.repo();

            // `edit_file` writes `EDITED_CONTENT` to `PATHS_FILE`; deleting that blob makes the diff unreadable.
            let edited = repo.write_blob(EDITED_CONTENT)?.detach();
            remove_loose_object(fixture.path(), edited)?;

            let expected = vec![FileChange {
                path: PATHS_FILE.to_owned(),
                kind: FileChangeKind::Modification,
                stat: Some(FileStat::Unavailable),
            }];

            assert_eq!(
                changed_between(&fixture, fixture.add_dir, fixture.edit_file)?,
                expected
            );

            Ok(())
        }
    }
}
