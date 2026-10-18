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

pub mod cdt;
mod commands;
#[cfg(test)]
mod fixtures;
pub mod side;

use nu_plugin_gix_core::Registry;

use nu_plugin::{EngineInterface, MsgPackSerializer, Plugin, PluginCommand, serve_plugin};
use nu_protocol::{CustomValue, LabeledError};

pub use side::SideArg;

/// The plugin entry point registered with Nushell.
#[derive(Debug, Default)]
pub struct GixPlugin {
    registry: Registry,
}

impl GixPlugin {
    /// The cache of repositories opened by live handles.
    pub const fn registry(&self) -> &Registry {
        &self.registry
    }
}

impl Plugin for GixPlugin {
    fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_owned()
    }

    fn commands(&self) -> Vec<Box<dyn PluginCommand<Plugin = Self>>> {
        vec![
            Box::new(commands::Gix),
            Box::new(commands::Open),
            Box::new(commands::Log),
        ]
    }

    fn custom_value_dropped(
        &self,
        _engine: &EngineInterface,
        custom_value: Box<dyn CustomValue>,
    ) -> Result<(), LabeledError> {
        if let Some(handle) = custom_value.as_any().downcast_ref::<cdt::Repository>() {
            self.registry.release(handle.id, &handle.git_dir);
        }

        Ok(())
    }
}

fn main() {
    serve_plugin(&GixPlugin::default(), MsgPackSerializer);
}
