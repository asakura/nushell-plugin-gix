//! Plugin commands.

mod error;
mod gix;
mod log;
mod open;

pub use self::{error::CommandError, gix::Gix, log::Log, open::Open};
