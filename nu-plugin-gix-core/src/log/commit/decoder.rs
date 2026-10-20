//! Decodes the text of a commit in the encoding its `encoding` header names, or as UTF-8.

use gix::{bstr::BStr, filter::plumbing::encoding::Encoding};

/// How the text of a commit is decoded: the encoding its `encoding` header names, or UTF-8.
#[derive(Clone, Copy, Debug)]
pub(super) struct Decoder {
    /// `None` decodes as UTF-8.
    encoding: Option<&'static Encoding>,
}

impl Decoder {
    /// Takes the encoding `label` names; an unknown label decodes as UTF-8.
    pub(super) fn new(label: Option<&BStr>) -> Self {
        Self {
            encoding: label.and_then(|label| Encoding::for_label_no_replacement(label)),
        }
    }

    /// `bytes` as text; malformed sequences become U+FFFD.
    pub(super) fn decode(self, bytes: &[u8]) -> String {
        self.encoding.map_or_else(
            || String::from_utf8_lossy(bytes).into_owned(),
            |encoding| encoding.decode_without_bom_handling(bytes).0.into_owned(),
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::{GixError, log::commit::CommitInfo};

    use gix::{
        actor::Signature,
        objs::{Commit, Tree},
    };
    use nu_plugin_gix_testing_support::{
        COMMITTER_NAME, COMMITTER_TIME, FixtureError, committer, dir,
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

    /// Header value naming the ISO-8859-1 (Latin-1) encoding.
    const LATIN1_LABEL: &str = "ISO-8859-1";
    /// Header value naming an encoding that does not exist.
    const UNKNOWN_LABEL: &str = "x-no-such-encoding";
    /// `café` and `naïve` in ISO-8859-1.
    const LATIN1_MESSAGE: &[u8] = b"caf\xe9\n\nna\xefve\n";
    /// `café` and `naïve` in UTF-8.
    const UTF8_MESSAGE: &[u8] = "caf\u{e9}\n\nna\u{ef}ve\n".as_bytes();
    /// `José` in ISO-8859-1.
    const LATIN1_NAME: &[u8] = b"Jos\xe9";
    /// `José` as UTF-8 text.
    const DECODED_NAME: &str = "Jos\u{e9}";

    /// Writes a commit with `encoding`, `name` as both signature names and `message` as stored
    /// bytes, on a new repository in `dir`, and reads it back.
    fn encoded_commit(
        dir: &Path,
        encoding: Option<&str>,
        name: &[u8],
        message: &[u8],
    ) -> Result<CommitInfo, TestError> {
        let repo = gix::init(dir)?;
        let tree = repo.write_object(Tree::empty())?.detach();
        let mut signature: Signature = committer(COMMITTER_TIME).into();

        signature.name = name.into();

        let commit = Commit {
            tree,
            parents: Vec::new().into(),
            author: signature.clone(),
            committer: signature,
            encoding: encoding.map(Into::into),
            message: message.into(),
            extra_headers: Vec::new(),
        };

        let id = repo.write_object(&commit)?;

        Ok(CommitInfo::read(&repo.find_commit(id)?, None)?)
    }

    mod encoding {
        use super::*;

        #[rstest]
        #[case::latin1(
            Some(LATIN1_LABEL),
            LATIN1_MESSAGE,
            "caf\u{e9}",
            Some("na\u{ef}ve"),
            "caf\u{e9}\n\nna\u{ef}ve\n"
        )]
        #[case::unknown_label_is_utf8(
            Some(UNKNOWN_LABEL),
            UTF8_MESSAGE,
            "caf\u{e9}",
            Some("na\u{ef}ve"),
            "caf\u{e9}\n\nna\u{ef}ve\n"
        )]
        #[case::no_header_is_utf8(
            None,
            UTF8_MESSAGE,
            "caf\u{e9}",
            Some("na\u{ef}ve"),
            "caf\u{e9}\n\nna\u{ef}ve\n"
        )]
        #[case::malformed_utf8_is_replaced(
            None,
            LATIN1_MESSAGE,
            "caf\u{FFFD}",
            Some("na\u{FFFD}ve"),
            "caf\u{FFFD}\n\nna\u{FFFD}ve\n"
        )]
        fn decodes_the_message_in_the_encoding_its_header_names(
            dir: Result<TempDir, FixtureError>,
            #[case] encoding: Option<&str>,
            #[case] message: &[u8],
            #[case] subject: &str,
            #[case] body: Option<&str>,
            #[case] text: &str,
        ) -> Result<(), TestError> {
            let dir = dir?;
            let info = encoded_commit(dir.path(), encoding, COMMITTER_NAME.as_bytes(), message)?;

            assert_eq!(
                (
                    info.subject.as_str(),
                    info.body.as_deref(),
                    info.message.as_str()
                ),
                (subject, body, text)
            );

            Ok(())
        }

        #[rstest]
        fn decodes_signature_names_in_the_header_encoding(
            dir: Result<TempDir, FixtureError>,
        ) -> Result<(), TestError> {
            let dir = dir?;
            let info = encoded_commit(dir.path(), Some(LATIN1_LABEL), LATIN1_NAME, UTF8_MESSAGE)?;

            assert_eq!(info.author.name, DECODED_NAME);
            assert_eq!(info.committer.name, DECODED_NAME);

            Ok(())
        }
    }
}
