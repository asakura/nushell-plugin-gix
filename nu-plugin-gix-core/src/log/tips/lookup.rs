//! Reads the commit a revision names and the parents of a commit, for the start points of a walk.

use crate::GixError;

use gix::object::Kind as ObjectKind;

/// Peels `id`, which `expression` resolved to, through tags to the commit it names.
pub(super) fn commit_id(
    repo: &gix::Repository,
    id: gix::ObjectId,
    expression: &str,
) -> Result<gix::ObjectId, GixError> {
    let unresolved = |source| GixError::Revision {
        expression: expression.to_owned(),
        source,
    };

    let object = repo
        .find_object(id)
        .map_err(unresolved)?
        .peel_tags_to_end()
        .map_err(unresolved)?;

    let kind = object.kind;

    if kind == ObjectKind::Commit {
        Ok(object.id)
    } else {
        Err(GixError::NotACommit {
            expression: expression.to_owned(),
            kind: kind.to_string(),
        })
    }
}

/// `id` when it names a commit; `None` when it names something else or cannot be read.
pub(super) fn commit_or_none(repo: &gix::Repository, id: gix::ObjectId) -> Option<gix::ObjectId> {
    (repo.find_object(id).ok()?.kind == ObjectKind::Commit).then_some(id)
}

/// The parents of the commit `id`.
pub(super) fn parent_ids(
    repo: &gix::Repository,
    id: gix::ObjectId,
) -> Result<Vec<gix::ObjectId>, gix::Error> {
    Ok(repo
        .find_commit(id)?
        .parent_ids()
        .map(gix::Id::detach)
        .collect())
}
