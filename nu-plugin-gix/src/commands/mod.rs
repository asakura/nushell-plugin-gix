//! Plugin commands.

mod error;
mod gix;
mod input;
mod log;
mod open;

pub use self::{
    error::CommandError,
    gix::{CATEGORY, Gix},
    input::{acquire_handle, flag_arg, handle_from, input_value},
    log::Log,
    open::Open,
};
