//! The commit itself as `gix log` shows it: ids, message parts, signatures and changed files.

use super::{
    decoder::Decoder,
    signature::{SignatureInfo, mapped, signature_info},
};

use crate::{GixError, log::changes::FileChange};

use gix::{bstr::ByteSlice as _, mailmap::Snapshot, objs::commit};

/// One commit as `gix log` shows it.
///
/// Text is decoded in the encoding the commit's `encoding` header names; without one, or with one
/// not known, as UTF-8, with malformed bytes replaced by U+FFFD.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitInfo {
    /// Object id of the commit.
    pub oid: gix::ObjectId,
    /// Ids of the parents in the order the commit stores them; empty for a root commit.
    pub parents: Vec<gix::ObjectId>,
    /// Id of the commit's tree.
    pub tree: gix::ObjectId,
    /// The title paragraph folded onto one line, as `git log --format=%s` shows it.
    pub subject: String,
    /// The message after the title paragraph with trailing whitespace trimmed; `None` when there is
    /// none or it is blank.
    pub body: Option<String>,
    /// The message exactly as stored in the commit.
    pub message: String,
    /// Who wrote the change, and when.
    pub author: SignatureInfo,
    /// Who made the commit, and when.
    pub committer: SignatureInfo,
    /// The files the commit changes compared with its first parent (the empty tree for a root commit), limited to
    /// the walk's paths; `None` unless `LogSettings::files` is set.
    pub files: Option<Vec<FileChange>>,
}

impl CommitInfo {
    /// Reads what [`CommitInfo`] shows of `commit`, mapping its signatures through `mailmap` when
    /// given.
    ///
    /// # Errors
    ///
    /// Returns [`GixError::Commit`] when the commit cannot be decoded, and [`GixError::Timestamp`]
    /// when a signature's time cannot be represented as a date.
    pub(in crate::log) fn read(
        commit: &gix::Commit<'_>,
        mailmap: Option<&Snapshot>,
    ) -> Result<Self, GixError> {
        let hex = commit.id.to_string();

        let wrap = |source| GixError::Commit {
            id: hex.clone(),
            source,
        };

        let decoder = Decoder::new(commit.decode().map_err(wrap)?.encoding);
        let message = decoder.decode(commit.message_raw().map_err(wrap)?);
        let tree = commit.tree_id().map_err(wrap)?.detach();

        let author = signature_info(
            mapped(commit.author().map_err(wrap)?, mailmap),
            decoder,
            hex.as_str(),
        )?;

        let committer = signature_info(
            mapped(commit.committer().map_err(wrap)?, mailmap),
            decoder,
            hex.as_str(),
        )?;

        let parents = commit.parent_ids().map(gix::Id::detach).collect();
        let parsed = commit::MessageRef::from_bytes(message.as_bytes());
        let subject = parsed.summary().to_str_lossy().into_owned();

        let body = parsed
            .body
            .map(|body| body.to_str_lossy().trim_end().to_owned())
            .filter(|body| !body.is_empty());

        Ok(Self {
            oid: commit.id,
            parents,
            tree,
            subject,
            body,
            message,
            author,
            committer,
            files: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fixtures::open_path;

    use gix::objs::Tree;
    use nu_plugin_gix_testing_support::{
        COMMITTER_EMAIL, COMMITTER_NAME, COMMITTER_TIME, FORK_MERGE_TIME, FORK_SECOND_PARENT_TIME,
        FixtureError, FixtureRepo as _, ForkedTestRepo, TaggedTestRepo, committer, dir,
        forked_repo, tagged_repo,
    };
    use rstest::rstest;
    use tempfile::TempDir;

    use std::{io, path::Path};

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Io(#[from] io::Error),

        #[error(transparent)]
        Gix(#[from] gix::Error),

        #[error(transparent)]
        Git(#[from] GixError),

        #[error(transparent)]
        Fixture(#[from] FixtureError),

        #[error("Test failure: {0}")]
        Failure(String),
    }

    /// The `forked_repo` author time, 1700000200 s = 2023-11-14T22:16:40Z, shown at `+02:00`.
    const AUTHOR_RFC3339: &str = "2023-11-15T00:16:40+02:00";
    /// The `forked_repo` committer time, 1700000300 s = 2023-11-14T22:18:20Z, shown at `-05:00`.
    const COMMITTER_RFC3339: &str = "2023-11-14T17:18:20-05:00";

    /// The commit that `revision` resolves to in the repository at `path`.
    fn info_at(path: &Path, revision: &str) -> Result<CommitInfo, TestError> {
        let repo = open_path(path)?.repo();

        let commit = repo
            .rev_parse_single(revision)?
            .object()?
            .try_into_commit()?;

        Ok(CommitInfo::read(&commit, None)?)
    }

    /// Creates a commit with `message` on `HEAD` in a new repository in `dir`.
    fn commit_with(
        dir: &Path,
        message: &str,
        author: &str,
        committer_time: &str,
    ) -> Result<CommitInfo, TestError> {
        let repo = gix::init(dir)?;
        let tree = repo.write_object(Tree::empty())?.detach();

        let id = repo.commit_as(
            committer(committer_time),
            committer(author),
            "HEAD",
            message,
            tree,
            Vec::<gix::ObjectId>::new(),
        )?;

        Ok(CommitInfo::read(&id.object()?.try_into_commit()?, None)?)
    }

    mod try_from {
        use super::*;

        #[rstest]
        #[case::title_and_body("title\n\nbody text\n", "title", Some("body text"))]
        #[case::title_only("title\n", "title", None)]
        #[case::blank_body("title\n\n\n", "title", None)]
        #[case::trailing_blank_lines("title\n\nbody\n\n\n", "title", Some("body"))]
        fn splits_the_message_into_subject_body_and_stored_text(
            dir: Result<TempDir, FixtureError>,
            #[case] message: &str,
            #[case] subject: &str,
            #[case] body: Option<&str>,
        ) -> Result<(), TestError> {
            let dir = dir?;
            let info = commit_with(dir.path(), message, COMMITTER_TIME, COMMITTER_TIME)?;

            assert_eq!(
                (
                    info.subject.as_str(),
                    info.body.as_deref(),
                    info.message.as_str()
                ),
                (subject, body, message)
            );

            Ok(())
        }

        #[rstest]
        fn lists_a_merge_commits_parents_in_stored_order(
            forked_repo: Result<ForkedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = forked_repo?;
            let info = info_at(repo.path(), "HEAD")?;

            let [first, second] = info.parents.as_slice() else {
                return Err(TestError::Failure(
                    "a merge commit has exactly two parents".to_owned(),
                ));
            };

            assert_eq!(first.to_string(), repo.first_parent);
            assert_eq!(second.to_string(), repo.second_parent);

            Ok(())
        }

        #[rstest]
        fn gives_a_root_commit_no_parents(
            forked_repo: Result<ForkedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = forked_repo?;
            let info = info_at(repo.path(), "HEAD^1^1")?;

            assert_eq!(info.parents, Vec::<gix::ObjectId>::new());

            Ok(())
        }

        #[rstest]
        fn exposes_the_commit_and_tree_ids(
            tagged_repo: Result<TaggedTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = tagged_repo?;
            let info = info_at(repo.path(), "HEAD")?;

            assert_eq!(info.oid, repo.commit);
            assert_eq!(info.tree, repo.tree);

            Ok(())
        }

        #[rstest]
        fn keeps_author_and_committer_apart_each_in_its_own_offset(
            dir: Result<TempDir, FixtureError>,
        ) -> Result<(), TestError> {
            let dir = dir?;

            let info = commit_with(
                dir.path(),
                "title",
                FORK_SECOND_PARENT_TIME,
                FORK_MERGE_TIME,
            )?;

            assert_eq!(info.author.time.to_rfc3339(), AUTHOR_RFC3339);
            assert_eq!(info.committer.time.to_rfc3339(), COMMITTER_RFC3339);
            assert_eq!(info.author.name, COMMITTER_NAME);
            assert_eq!(info.author.email, COMMITTER_EMAIL);
            assert_eq!(info.committer.name, COMMITTER_NAME);
            assert_eq!(info.committer.email, COMMITTER_EMAIL);

            Ok(())
        }
    }
}
