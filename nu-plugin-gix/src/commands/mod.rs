//! Plugin commands.

mod error;
mod gix;
mod input;
mod log;
mod oid;
mod open;
mod refs;
mod rev;

pub use self::{
    error::CommandError,
    gix::{CATEGORY, Gix, help_call},
    input::{acquire_handle, argument_span, flag_arg, handle_from, input_value, positional_arg},
    log::Log,
    oid::require_full_oid,
    open::Open,
    refs::{Ref, RefHead, RefList, RefSet},
    rev::Rev,
};
