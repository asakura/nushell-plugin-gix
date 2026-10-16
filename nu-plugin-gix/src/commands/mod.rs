//! Plugin commands.

mod error;
mod gix;
mod input;
mod log;
mod open;

pub use self::{error::CommandError, gix::Gix, input::handle_from, log::Log, open::Open};
