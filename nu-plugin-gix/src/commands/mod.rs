//! Plugin commands.

mod error;
mod gix;
mod input;
mod log;
mod open;
mod rev;

pub use self::{
    error::CommandError,
    gix::{CATEGORY, Gix},
    input::{acquire_handle, flag_arg, handle_from, input_value, positional_arg},
    log::Log,
    open::Open,
    rev::Rev,
};
