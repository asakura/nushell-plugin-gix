//! Shared test fixtures for `nushell-plugin-gix`: throwaway git repositories built with `gix`,
//! including `paths_repo`, whose history touches files inside and outside a directory, and
//! `diverged_repo`, whose second branch `HEAD` cannot reach; `skewed_repo`, whose parent is newer
//! than the children it precedes; `mailmap_repo`, whose worktree `.mailmap` maps the fixture
//! committer; and the in-process plugin-engine
//! helpers (`plugin_test_for`, `custom_value_input`, `pin_pwd`) used by plugin tests, plus the
//! script assertions (`eval_value`, `eval_labeled_error`, ...) that inspect what a script produces.

#![cfg_attr(
    test,
    expect(
        clippy::panic_in_result_fn,
        reason = "rstest cases return Result for ?-based setup but use assertions for checks"
    )
)]

mod assertions;
mod basic;
mod engine;
mod error;
mod history;
mod merges;
mod paths;
mod repo;

pub use self::assertions::{
    AssertionError, eval_labeled_error, eval_value, first_row, record_column,
};
pub use self::basic::{
    ANNOTATED_TAG, ANNOTATED_TAG_MESSAGE, ANNOTATED_TAG_REF, BINARY_FILE, BLOB_CONTENT,
    BinaryTestRepo, CorruptTestRepo, FEATURE_BRANCH, FEATURE_BRANCH_REF, FIRST_COMMIT_MESSAGE,
    HEAD_FILE, INITIAL_MESSAGE, LIGHT_TAG_REF, LinkedTestRepo, MAILMAP_EMAIL, MAILMAP_NAME,
    SECOND_COMMIT_MESSAGE, TaggedTestRepo, TestRepo, UnbornTestRepo, bare_repo, binary_repo,
    corrupt_repo, linked_repo, mailmap_repo, tagged_repo, test_repo, unborn_repo,
};
pub use self::engine::{
    custom_value_input, default_plugin_test, discovered_input, gix_error, pin_pwd, plugin_test_for,
};
pub use self::error::FixtureError;
pub use self::history::{HISTORY, init_with_history};
pub use self::merges::{
    DISCARDED_FIRST_PARENT_MESSAGE, DISCARDED_MERGE_MESSAGE, DISCARDED_ROOT_MESSAGE,
    DISCARDED_SIDE_MESSAGE, DiscardedSideTestRepo, DivergedTestRepo, FORK_FIRST_PARENT_MESSAGE,
    FORK_FIRST_PARENT_TIME, FORK_MERGE_MESSAGE, FORK_MERGE_TIME, FORK_ROOT_MESSAGE,
    FORK_SECOND_PARENT_MESSAGE, FORK_SECOND_PARENT_TIME, ForkedTestRepo, OTHER_BRANCH_REF,
    OTHER_COMMIT_MESSAGE, OTHER_COMMIT_TIME, SECOND_WINS_FIRST_PARENT_MESSAGE,
    SECOND_WINS_MERGE_MESSAGE, SECOND_WINS_ROOT_MESSAGE, SECOND_WINS_SECOND_PARENT_MESSAGE,
    SKEW_CHILD_MESSAGE, SKEW_MERGE_MESSAGE, SKEW_PARENT_MESSAGE, SKEW_ROOT_MESSAGE,
    SKEW_SIDE_MESSAGE, SecondParentWinsTestRepo, SkewedTestRepo, THIRD_CONTENT, UNRELATED_FILE,
    discarded_side_repo, diverged_repo, forked_repo, second_parent_wins_repo, skewed_repo,
};
pub use self::paths::{
    ADD_DIR_MESSAGE, ADD_FILE_MESSAGE, EDIT_FILE_MESSAGE, EDIT_NESTED_MESSAGE, PATHS_DIR,
    PATHS_NESTED_FILE, PATHS_NESTED_NAME, PathsTestRepo, UNCHANGED_TREE_MESSAGE, paths_repo,
};
pub use self::repo::{
    COMMITTER_EMAIL, COMMITTER_NAME, COMMITTER_TIME, EDITED_CONTENT, FIXTURE_REFLOG_MESSAGE,
    FixtureRepo, GIT_DIR, MAIN_BRANCH, ORIGINAL_CONTENT, PATHS_FILE, append_config, committer, dir,
    plain_dir, remove_committer_identity, remove_loose_object,
};
