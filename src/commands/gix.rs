//! `gix`: the parent command, which prints the plugin's help.

use nu_plugin::{EngineInterface, EvaluatedCall, SimplePluginCommand};
use nu_protocol::{Category, LabeledError, Signature, Type, Value};

use super::super::GixPlugin;

const COMMAND_NAME: &str = "gix";

/// Parent of all `gix` subcommands; prints the help listing them.
#[derive(Clone, Copy, Debug)]
pub struct Gix;

impl SimplePluginCommand for Gix {
    type Plugin = GixPlugin;

    fn name(&self) -> &'static str {
        COMMAND_NAME
    }

    fn description(&self) -> &'static str {
        "Git commands powered by gix (gitoxide)."
    }

    fn extra_description(&self) -> &'static str {
        "You must use one of the following subcommands. Using this command as-is will only produce this help message."
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .input_output_type(Type::Nothing, Type::String)
            .category(Category::Custom("gix".into()))
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["git", "gitoxide"]
    }

    fn run(
        &self,
        _plugin: &GixPlugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: &Value,
    ) -> Result<Value, LabeledError> {
        Ok(Value::string(engine.get_help()?, call.head))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use rstest::rstest;

    mod signature {
        use super::*;

        #[rstest]
        fn is_named_gix() {
            assert_eq!(Gix.signature().name, COMMAND_NAME);
        }
    }
}
