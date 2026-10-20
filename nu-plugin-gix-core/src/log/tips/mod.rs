//! Start points of a commit walk: the commits it walks from and the commits whose history it hides.
//!
//! A revision expression becomes include and hide commits as follows:
//!
//! | Expression | Includes | Hides |
//! |---|---|---|
//! | `x` | `x` | |
//! | `^x` | | `x` |
//! | `a..b` | `b` | `a` |
//! | `a...b` | `a`, `b` | their merge bases |
//! | `x^@` | the parents of `x` | |
//! | `x^!` | `x` | the parents of `x` |
//!
//! With no revision and no `all`, the start is `HEAD`. With `all`, every reference that peels to a
//! commit is included. An explicit revision that does not resolve is an error, but a reference met
//! while expanding `all` that cannot be read or peeled is skipped: one broken reference should not
//! stop a listing of everything else.

mod lookup;
mod resolve;

pub(super) use self::resolve::Tips;
