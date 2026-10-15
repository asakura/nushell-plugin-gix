//! `nu_plugin_gix`: a Nushell plugin exposing `gix` (gitoxide).

#![cfg_attr(
    test,
    expect(
        clippy::panic_in_result_fn,
        reason = "rstest cases return Result for ?-based setup but use assertions for checks"
    ),
    expect(
        clippy::rest_pattern_accessible_field,
        reason = "tests match on a variant without caring about its fields"
    )
)]

mod commands;
mod error;
mod git;
mod repository;

use nu_plugin::{MsgPackSerializer, Plugin, PluginCommand, serve_plugin};

/// The plugin entry point registered with Nushell.
#[derive(Clone, Copy, Debug)]
pub struct GixPlugin;

impl Plugin for GixPlugin {
    fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_owned()
    }

    fn commands(&self) -> Vec<Box<dyn PluginCommand<Plugin = Self>>> {
        vec![Box::new(commands::Open), Box::new(commands::Log)]
    }
}

fn main() {
    serve_plugin(&GixPlugin, MsgPackSerializer);
}
