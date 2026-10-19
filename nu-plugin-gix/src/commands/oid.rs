//! Shape and validation helpers for full object ids, shared by argument-parsing commands.

use crate::commands::CommandError;

use nu_protocol::Span;

/// Number of hexadecimal characters in a full SHA-1 object id.
pub const SHA1_HEX_LEN: usize = 40;

/// Number of hexadecimal characters in a full SHA-256 object id.
pub const SHA256_HEX_LEN: usize = 64;

/// Returns whether `value` has the length of a full SHA-1 or SHA-256 hexadecimal digest.
#[must_use = "returns the shape verdict; discarding it skips the check"]
pub const fn is_full_oid(value: &str) -> bool {
    value.len() == SHA1_HEX_LEN || value.len() == SHA256_HEX_LEN
}

/// Returns whether `byte` is an ASCII hexadecimal digit, in either case.
#[must_use = "returns the digit verdict; discarding it skips the check"]
pub const fn is_hexadecimal(byte: u8) -> bool {
    byte.is_ascii_hexdigit()
}

/// Requires `value` to be a full object id: a SHA-1 or SHA-256 digest length, made only of
/// hexadecimal digits. Both failures point at `span`, the argument or flag that carried `value`.
///
/// # Errors
///
/// Returns [`CommandError::NotAFullObjectId`] when `value` has the wrong length, and
/// [`CommandError::InvalidObjectId`] when it has the right length but a character that is not a
/// hexadecimal digit.
pub fn require_full_oid(value: &str, span: Span) -> Result<(), CommandError> {
    if !is_full_oid(value) {
        return Err(CommandError::NotAFullObjectId {
            value: value.to_owned(),
            span,
        });
    }

    if !value.bytes().all(is_hexadecimal) {
        return Err(CommandError::InvalidObjectId {
            expression: value.to_owned(),
            span,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use rstest::rstest;

    use std::assert_matches;

    const SHA1_LOWER: &str = "0123456789abcdef0123456789abcdef01234567";
    const SHA1_UPPER: &str = "0123456789ABCDEF0123456789ABCDEF01234567";
    const SHA256_LOWER: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const SHORT_OID: &str = "0123456";
    const SHA1_NOT_HEX: &str = "0123456789abcdef0123456789abcdef0123456z";
    const VALUE_SPAN: Span = Span { start: 7, end: 12 };

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Command(#[from] CommandError),

        #[error("Test failure: {0}")]
        Failure(String),
    }

    mod is_full_oid {
        use super::*;

        #[rstest]
        #[case::sha1(SHA1_LOWER, true)]
        #[case::sha256(SHA256_LOWER, true)]
        #[case::short(SHORT_OID, false)]
        fn accepts_only_full_digest_lengths(#[case] value: &str, #[case] expected: bool) {
            assert_eq!(is_full_oid(value), expected);
        }
    }

    mod require_full_oid {
        use super::*;

        #[rstest]
        #[case::sha1_upper(SHA1_UPPER)]
        #[case::sha256_lower(SHA256_LOWER)]
        fn accepts_full_length_hexadecimal_values(#[case] value: &str) -> Result<(), TestError> {
            require_full_oid(value, VALUE_SPAN)?;

            Ok(())
        }

        #[rstest]
        fn rejects_a_wrong_length_value_at_its_span() -> Result<(), TestError> {
            let err = require_full_oid(SHORT_OID, VALUE_SPAN)
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert_matches!(
                &err,
                CommandError::NotAFullObjectId { span, .. } if *span == VALUE_SPAN,
                "got {err:?}"
            );

            Ok(())
        }

        #[rstest]
        fn rejects_a_full_length_non_hexadecimal_value_at_its_span() -> Result<(), TestError> {
            let err = require_full_oid(SHA1_NOT_HEX, VALUE_SPAN)
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert_matches!(
                &err,
                CommandError::InvalidObjectId { span, .. } if *span == VALUE_SPAN,
                "got {err:?}"
            );

            Ok(())
        }
    }
}
