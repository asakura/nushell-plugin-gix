//! Linear histories of empty-tree commits, with chosen messages and committer times, for walk and ordering tests.

use crate::basic::FIRST_COMMIT_MESSAGE;
use crate::error::FixtureError;
use crate::repo::{COMMITTER_TIME, HEAD_REF, committer};

use gix::objs::Tree;

use std::path::Path;

/// `(message, raw git timestamp)` pairs of [`init_with_history`], oldest first.
pub const HISTORY: [(&str, &str); 3] = [
    (FIRST_COMMIT_MESSAGE, COMMITTER_TIME),
    ("second\n\nbody text", "1700000060 +0200"),
    ("third", "1700000120 -0500"),
];

/// Initialises a repository in `dir` with one empty-tree commit on `HEAD` per entry of `history`, oldest first.
///
/// Each commit is authored and committed by the fixture committer, dated at the entry's raw git timestamp.
///
/// # Errors
///
/// Returns [`FixtureError::Gix`] when the repository or one of its commits cannot be written.
pub fn init_with_history(dir: &Path, history: &[(&str, &str)]) -> Result<(), FixtureError> {
    let repo = gix::init(dir)?;
    // `empty_tree()` is virtual and never reaches the object database, so write it once.
    let tree = repo.write_object(Tree::empty())?.detach();
    let mut parents = Vec::new();

    for (message, time) in history {
        let signature = committer(time);

        let id = repo.commit_as(
            signature,
            signature,
            HEAD_REF,
            *message,
            tree,
            parents.clone(),
        )?;

        parents = vec![id.detach()];
    }

    Ok(())
}
