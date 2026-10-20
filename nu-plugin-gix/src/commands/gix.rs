//! `gix`: the parent command, which prints the plugin's help.

use crate::GixPlugin;
use crate::commands::CommandError;

use nu_plugin::{EngineInterface, EvaluatedCall, SimplePluginCommand};
use nu_protocol::{Category, LabeledError, Signature, Span, Type, Value};

const COMMAND_NAME: &str = "gix";

/// Nushell command category shared by every `gix` command.
pub const CATEGORY: &str = "gix";

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
        "You must use one of the following subcommands. Using this command as-is will only produce this help message.\n\
        Error codes: gix::help-unavailable"
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .input_output_type(Type::Nothing, Type::String)
            .category(Category::Custom(CATEGORY.into()))
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
        help_call(engine, call.head).map_err(|e| e.into_labeled(call.head))
    }
}

/// Reads the help text the engine renders for the parent command; a failure points at the call.
pub fn help_call(engine: &EngineInterface, span: Span) -> Result<Value, CommandError> {
    let help = engine
        .get_help()
        .map_err(|source| CommandError::HelpUnavailable {
            source: Box::new(source),
            span,
        })?;

    Ok(Value::string(help, span))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fixtures::plugin_test;

    use nu_plugin::Plugin as _;
    use nu_plugin_gix_testing_support::FixtureError;
    use nu_plugin_test_support::PluginTest;
    use nu_protocol::{ShellError, Span};
    use rstest::rstest;

    const EVAL_HELP: &str = "gix";
    /// Each `Subcommands:` entry reads `  <name> (plugin) - <description>`: the name ends at either marker.
    const PLUGIN_MARKER: &str = " (";
    const DESCRIPTION_MARKER: &str = " - ";

    /// Errors raised by the `gix` command end-to-end tests.
    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Fixture(#[from] FixtureError),

        #[error(transparent)]
        Shell(#[from] ShellError),

        #[error("Test failure: {0}")]
        Failure(String),
    }

    /// Metadata checks that `eval` cannot express: they inspect the `Signature`
    /// directly rather than the command surface.
    mod signature {
        use super::*;

        #[rstest]
        fn is_named_gix() {
            assert_eq!(Gix.signature().name, COMMAND_NAME);
        }
    }

    mod help {
        use super::*;

        /// The command name at the start of a help line, or the whole line when it has no entry marker.
        fn listed_name(line: &str) -> &str {
            let entry = line.trim_start();

            let head = entry
                .split_once(PLUGIN_MARKER)
                .map_or(entry, |(name, _)| name);

            head.split_once(DESCRIPTION_MARKER)
                .map_or(head, |(name, _)| name)
        }

        #[rstest]
        fn lists_the_registered_subcommands(
            plugin_test: Result<PluginTest, FixtureError>,
        ) -> Result<(), TestError> {
            let commands = GixPlugin::default().commands();

            let subcommands: Vec<&str> = commands
                .iter()
                .map(|command| command.name())
                .filter(|name| *name != COMMAND_NAME)
                .collect();

            if subcommands.is_empty() {
                return Err(TestError::Failure(
                    "the plugin registers no subcommands of `gix`".to_owned(),
                ));
            }

            let mut test = plugin_test?;
            let help = test.eval(EVAL_HELP)?.into_value(Span::test_data())?;
            let text = help.as_str()?;
            let listed: Vec<&str> = text.lines().map(listed_name).collect();

            for name in subcommands {
                assert!(listed.contains(&name), "help is missing `{name}`:\n{text}");
            }

            Ok(())
        }
    }

    /// Registration checked against a hardcoded list, so a command dropped from `commands()` fails here.
    mod registration {
        use super::*;

        /// Every command the plugin registers, spelled as Nushell sees it.
        const EXPECTED_COMMANDS: [&str; 10] = [
            "gix",
            "gix config",
            "gix config get",
            "gix log",
            "gix open",
            "gix ref",
            "gix ref head",
            "gix ref list",
            "gix ref set",
            "gix rev",
        ];

        #[rstest]
        fn matches_the_expected_command_names() {
            let commands = GixPlugin::default().commands();

            let mut registered: Vec<&str> = commands.iter().map(|command| command.name()).collect();
            registered.sort_unstable();

            let mut expected = EXPECTED_COMMANDS.to_vec();
            expected.sort_unstable();

            assert_eq!(registered, expected);
        }
    }
}
