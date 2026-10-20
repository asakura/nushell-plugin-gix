//! The name, e-mail address and time of one signature of a commit, mapped through a mailmap.

use super::decoder::Decoder;

use crate::GixError;

use chrono::{DateTime, FixedOffset};
use gix::{actor::SignatureRef, mailmap::Snapshot};

/// Who signed a commit, and when: its author or its committer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureInfo {
    /// Name, with surrounding whitespace trimmed.
    pub name: String,
    /// E-mail address, with surrounding whitespace trimmed.
    pub email: String,
    /// When it was signed, in the signer's own UTC offset.
    pub time: DateTime<FixedOffset>,
}

/// `signature` with the name and e-mail address `mailmap` maps it to.
pub(super) fn mapped<'a>(
    signature: SignatureRef<'a>,
    mailmap: Option<&'a Snapshot>,
) -> SignatureRef<'a> {
    mailmap
        .and_then(|map| map.try_resolve_ref(signature))
        .map_or(signature, |resolved| SignatureRef {
            name: resolved.name.unwrap_or(signature.name),
            email: resolved.email.unwrap_or(signature.email),
            time: signature.time,
        })
}

/// Converts one signature of the commit with hex id `oid`, decoding its name and e-mail address
/// with `decoder` and reporting failures against that commit.
///
/// # Errors
///
/// Returns [`GixError::Commit`] when the signature's time cannot be decoded, and
/// [`GixError::Timestamp`] when it cannot be represented as a date.
pub(super) fn signature_info(
    signature: SignatureRef<'_>,
    decoder: Decoder,
    oid: &str,
) -> Result<SignatureInfo, GixError> {
    let when = signature.time().map_err(|source| GixError::Commit {
        id: oid.to_owned(),
        source,
    })?;

    let out_of_range = || GixError::Timestamp { id: oid.to_owned() };
    let offset = FixedOffset::east_opt(when.offset).ok_or_else(out_of_range)?;

    let time = DateTime::from_timestamp(when.seconds, 0)
        .ok_or_else(out_of_range)?
        .with_timezone(&offset);

    Ok(SignatureInfo {
        name: decoder.decode(signature.name),
        email: decoder.decode(signature.email),
        time,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::log::commit::CommitInfo;

    use gix::objs::Tree;
    use nu_plugin_gix_testing_support::{
        COMMITTER_EMAIL, COMMITTER_NAME, COMMITTER_TIME, FixtureError, MAILMAP_EMAIL, MAILMAP_NAME,
        committer, dir,
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
    }

    mod mailmap {
        use super::*;

        /// One line of a mailmap, each mapping the fixture committer somehow.
        #[derive(Clone, Copy, Debug)]
        enum Entry {
            NameAndEmail,
            NameOnly,
            Unmatched,
        }

        impl Entry {
            /// The mailmap line this entry stands for.
            fn line(self) -> String {
                match self {
                    Self::NameAndEmail => format!(
                        "{MAILMAP_NAME} <{MAILMAP_EMAIL}> {COMMITTER_NAME} <{COMMITTER_EMAIL}>\n"
                    ),
                    Self::NameOnly => format!("{MAILMAP_NAME} <{COMMITTER_EMAIL}>\n"),
                    Self::Unmatched => "Grace <grace@example.com>\n".to_owned(),
                }
            }
        }

        /// Writes a commit by the fixture committer on a new repository in `dir` and reads it back
        /// through the mailmap `mailmap`.
        fn mailmapped(dir: &Path, mailmap: &str) -> Result<CommitInfo, TestError> {
            let repo = gix::init(dir)?;
            let tree = repo.write_object(Tree::empty())?.detach();

            let id = repo.commit_as(
                committer(COMMITTER_TIME),
                committer(COMMITTER_TIME),
                "HEAD",
                "title",
                tree,
                Vec::<gix::ObjectId>::new(),
            )?;

            let commit = id.object()?.try_into_commit()?;
            let snapshot = Snapshot::from_bytes(mailmap.as_bytes());

            Ok(CommitInfo::read(&commit, Some(&snapshot))?)
        }

        #[rstest]
        #[case::name_and_email(Entry::NameAndEmail, MAILMAP_NAME, MAILMAP_EMAIL)]
        #[case::name_only(Entry::NameOnly, MAILMAP_NAME, COMMITTER_EMAIL)]
        #[case::unmatched(Entry::Unmatched, COMMITTER_NAME, COMMITTER_EMAIL)]
        fn maps_both_signatures_through_the_mailmap(
            dir: Result<TempDir, FixtureError>,
            #[case] entry: Entry,
            #[case] name: &str,
            #[case] email: &str,
        ) -> Result<(), TestError> {
            let dir = dir?;
            let info = mailmapped(dir.path(), &entry.line())?;

            assert_eq!(
                (info.author.name.as_str(), info.author.email.as_str()),
                (name, email)
            );
            assert_eq!(
                (info.committer.name.as_str(), info.committer.email.as_str()),
                (name, email)
            );

            Ok(())
        }
    }
}
