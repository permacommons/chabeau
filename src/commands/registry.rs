use super::CommandResult;
use crate::core::app::App;

pub type CommandHandler = fn(&mut App, CommandInvocation<'_>) -> CommandResult;

pub struct Command {
    pub name: &'static str,
    pub help: &'static str,
    pub handler: CommandHandler,
}

#[derive(Clone, Copy)]
pub struct CommandInvocation<'a> {
    pub input: &'a str,
    pub args: &'a str,
}

pub fn all_commands() -> &'static [Command] {
    COMMANDS
}

pub fn find_command(name: &str) -> Option<&'static Command> {
    all_commands()
        .iter()
        .find(|command| command.name.eq_ignore_ascii_case(name))
}

const COMMANDS: &[Command] = &[
    Command {
        name: "help",
        help: "Show available commands and usage information.",
        handler: super::handle_help,
    },
    Command {
        name: "log",
        help: "Toggle logging or set the log file path.",
        handler: super::handle_log,
    },
    Command {
        name: "dump",
        help: "Export the current conversation to a file.",
        handler: super::handle_dump,
    },
    Command {
        name: "theme",
        help: "Open the theme picker or apply a theme directly.",
        handler: super::handle_theme,
    },
    Command {
        name: "model",
        help: "Open the model picker or switch models immediately.",
        handler: super::handle_model,
    },
    Command {
        name: "provider",
        help: "Open the provider picker or switch providers immediately.",
        handler: super::handle_provider,
    },
    Command {
        name: "markdown",
        help: "Toggle markdown rendering for assistant responses.",
        handler: super::handle_markdown,
    },
    Command {
        name: "syntax",
        help: "Toggle syntax highlighting for code blocks.",
        handler: super::handle_syntax,
    },
];
