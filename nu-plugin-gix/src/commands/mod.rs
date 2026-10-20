//! Plugin commands.

mod config;
mod config_known;
mod config_read;
mod error;
mod gix;
mod input;
mod log;
mod oid;
mod open;
mod refs;
mod rev;

pub use self::{
    config::{Config, ConfigGet, ConfigList},
    error::CommandError,
    gix::{CATEGORY, Gix, help_call},
    input::{
        acquire_handle, argument_span, flag_arg, handle_from, input_value, non_blank_revision,
        optional_arg, positional_arg, switch_arg,
    },
    log::Log,
    oid::{is_full_oid, is_hexadecimal, require_full_oid},
    open::Open,
    refs::{Ref, RefHead, RefList, RefSet},
    rev::Rev,
};
