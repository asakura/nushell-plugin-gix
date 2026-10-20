//! The `paths_repo` fixture, whose history touches files inside and outside a directory.

use crate::error::FixtureError;
use crate::repo::{
    EDITED_CONTENT, FixtureRepo, MAIN_BRANCH, ORIGINAL_CONTENT, PATHS_FILE, commit_on_head,
    init_isolated, point_head_at, tree_with,
};

use gix::create;
use gix::objs::Blob;
use rstest::fixture;
use tempfile::TempDir;

use std::fs;

/// Directory that [`paths_repo`] adds; it also exists on disk in the worktree, so tests can run from it.
pub const PATHS_DIR: &str = "dir";

/// File inside [`PATHS_DIR`] that [`paths_repo`] adds and later edits.
pub const PATHS_NESTED_FILE: &str = "dir/b.txt";

/// Name of [`PATHS_NESTED_FILE`] relative to [`PATHS_DIR`], as written from inside that directory.
pub const PATHS_NESTED_NAME: &str = "b.txt";

/// Subject of [`paths_repo`]'s root commit, which adds [`PATHS_FILE`].
pub const ADD_FILE_MESSAGE: &str = "add file";

/// Subject of the commit that adds [`PATHS_NESTED_FILE`].
pub const ADD_DIR_MESSAGE: &str = "add dir";

/// Subject of the commit that edits [`PATHS_FILE`].
pub const EDIT_FILE_MESSAGE: &str = "edit file";

/// Subject of the commit that edits [`PATHS_NESTED_FILE`].
pub const EDIT_NESTED_MESSAGE: &str = "edit nested";

/// Subject of [`paths_repo`]'s `HEAD`, whose tree equals its parent's.
pub const UNCHANGED_TREE_MESSAGE: &str = "unchanged tree";

/// Timestamp of the commit that adds [`PATHS_FILE`], the oldest commit of [`paths_repo`].
const PATHS_ADD_FILE_TIME: &str = "1700000000 +0000";

/// Timestamp of the commit that adds [`PATHS_NESTED_FILE`], one minute after [`PATHS_ADD_FILE_TIME`].
const PATHS_ADD_DIR_TIME: &str = "1700000060 +0000";

/// Timestamp of the commit that edits [`PATHS_FILE`], one minute after [`PATHS_ADD_DIR_TIME`].
const PATHS_EDIT_FILE_TIME: &str = "1700000120 +0000";

/// Timestamp of the commit that edits [`PATHS_NESTED_FILE`], one minute after [`PATHS_EDIT_FILE_TIME`].
const PATHS_EDIT_NESTED_TIME: &str = "1700000180 +0000";

/// Timestamp of the unchanged-tree commit of [`paths_repo`], the newest commit.
const PATHS_UNCHANGED_TIME: &str = "1700000240 +0000";

/// A throwaway repository whose `HEAD` history touches a top-level file and a file inside [`PATHS_DIR`].
///
/// Returned by [`paths_repo`]. Each id names one commit of the history: the two adds, the two edits, and the commit
/// whose tree equals its parent's.
#[derive(Debug)]
pub struct PathsTestRepo {
    /// The temporary directory backing the repository (dropped last).
    pub dir: TempDir,
    /// Id of the root commit, which adds [`PATHS_FILE`].
    pub add_file: gix::ObjectId,
    /// Id of the commit that adds [`PATHS_NESTED_FILE`].
    pub add_dir: gix::ObjectId,
    /// Id of the commit that edits [`PATHS_FILE`].
    pub edit_file: gix::ObjectId,
    /// Id of the commit that edits [`PATHS_NESTED_FILE`].
    pub edit_nested: gix::ObjectId,
    /// Id of `HEAD`, whose tree equals the tree of its parent.
    pub unchanged: gix::ObjectId,
}

impl FixtureRepo for PathsTestRepo {
    fn dir(&self) -> &TempDir {
        &self.dir
    }
}

/// Builds a five-commit history on `HEAD` that touches files: add [`PATHS_FILE`], add [`PATHS_NESTED_FILE`],
/// edit [`PATHS_FILE`], edit [`PATHS_NESTED_FILE`], then a commit whose tree equals its parent's.
///
/// The trees are written directly, not checked out; only the empty directory [`PATHS_DIR`] is created
/// in the worktree, so a test can make it the engine's current directory.
#[fixture]
pub fn paths_repo() -> Result<PathsTestRepo, FixtureError> {
    let dir = TempDir::new()?;
    let repo = init_isolated(dir.path(), create::Kind::WithWorktree)?;

    point_head_at(&repo, MAIN_BRANCH)?;

    fs::create_dir_all(dir.path().join(PATHS_DIR))?;

    let original = repo.write_object(&Blob {
        data: ORIGINAL_CONTENT.to_vec(),
    })?;

    let edited = repo.write_object(&Blob {
        data: EDITED_CONTENT.to_vec(),
    })?;

    let original = original.detach();
    let edited = edited.detach();
    let file_only = tree_with(&repo, &[(PATHS_FILE, original)])?;
    let both_edited = tree_with(&repo, &[(PATHS_FILE, edited), (PATHS_NESTED_FILE, edited)])?;

    let with_dir = tree_with(
        &repo,
        &[(PATHS_FILE, original), (PATHS_NESTED_FILE, original)],
    )?;

    let file_edited = tree_with(
        &repo,
        &[(PATHS_FILE, edited), (PATHS_NESTED_FILE, original)],
    )?;

    let add_file = commit_on_head(
        &repo,
        ADD_FILE_MESSAGE,
        file_only,
        PATHS_ADD_FILE_TIME,
        None,
    )?;

    let add_dir = commit_on_head(
        &repo,
        ADD_DIR_MESSAGE,
        with_dir,
        PATHS_ADD_DIR_TIME,
        Some(add_file),
    )?;

    let edit_file = commit_on_head(
        &repo,
        EDIT_FILE_MESSAGE,
        file_edited,
        PATHS_EDIT_FILE_TIME,
        Some(add_dir),
    )?;

    let edit_nested = commit_on_head(
        &repo,
        EDIT_NESTED_MESSAGE,
        both_edited,
        PATHS_EDIT_NESTED_TIME,
        Some(edit_file),
    )?;

    let unchanged = commit_on_head(
        &repo,
        UNCHANGED_TREE_MESSAGE,
        both_edited,
        PATHS_UNCHANGED_TIME,
        Some(edit_nested),
    )?;

    Ok(PathsTestRepo {
        dir,
        add_file,
        add_dir,
        edit_file,
        edit_nested,
        unchanged,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use gix::open;
    use rstest::rstest;

    use std::assert_matches;
    use std::{io, path::Path};

    #[derive(thiserror::Error, Debug)]
    enum TestError {
        #[error(transparent)]
        Fixture(#[from] FixtureError),

        #[error(transparent)]
        Gix(#[from] gix::Error),

        #[error(transparent)]
        Io(#[from] io::Error),

        #[error("Test failure: {0}")]
        #[expect(
            dead_code,
            reason = "every test error carries a Failure variant, even where no test builds one"
        )]
        Failure(String),
    }

    /// Opens `dir` with the isolated open options that the fixtures use.
    fn open_isolated(dir: &Path) -> Result<gix::Repository, TestError> {
        let repo = gix::open_opts(dir, open::Options::isolated())?;

        Ok(repo)
    }

    mod paths_repo_fixture {
        use super::*;

        #[rstest]
        fn head_is_the_unchanged_commit(
            paths_repo: Result<PathsTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = paths_repo?;
            let opened = open_isolated(repo.path())?;

            assert_matches!(opened.head_id()?.detach() == repo.unchanged, true);

            Ok(())
        }

        #[rstest]
        fn head_tree_holds_the_nested_file(
            paths_repo: Result<PathsTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = paths_repo?;
            let opened = open_isolated(repo.path())?;
            let tree = opened.head_commit()?.tree()?;

            assert_matches!(tree.lookup_entry_by_path(PATHS_NESTED_FILE)?, Some(_));

            Ok(())
        }

        #[rstest]
        fn creates_the_directory_on_disk(
            paths_repo: Result<PathsTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = paths_repo?;

            assert_matches!(repo.path().join(PATHS_DIR).is_dir(), true);

            Ok(())
        }
    }
}
