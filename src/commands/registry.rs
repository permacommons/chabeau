use super::CommandResult;
use crate::core::app::App;

pub type CommandHandler = fn(&mut App, CommandInvocation<'_>) -> CommandResult;

pub struct CommandUsage {
    pub syntax: &'static str,
    pub description: &'static str,
}

pub struct Command {
    pub name: &'static str,
    pub usages: &'static [CommandUsage],
    pub extra_help: &'static [&'static str],
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

pub fn matching_commands(prefix: &str) -> Vec<&'static Command> {
    let lower_prefix = prefix.to_ascii_lowercase();
    all_commands()
        .iter()
        .filter(|command| {
            if lower_prefix.is_empty() {
                true
            } else {
                command.name.to_ascii_lowercase().starts_with(&lower_prefix)
            }
        })
        .collect()
}

const COMMANDS: &[Command] = &[
    Command {
        name: "help",
        usages: &[CommandUsage {
            syntax: "/help",
            description: "Show available commands and usage information.",
        }],
        extra_help: &[],
        handler: super::handle_help,
    },
    Command {
        name: "log",
        usages: &[CommandUsage {
            syntax: "/log [filename]",
            description:
                "Enable logging to a file, or toggle pause/resume when no filename is provided.",
        }],
        extra_help: &[],
        handler: super::handle_log,
    },
    Command {
        name: "dump",
        usages: &[CommandUsage {
            syntax: "/dump [filename]",
            description:
                "Dump the full conversation to a file (default: `chabeau-log-YYYY-MM-DD.txt`).",
        }],
        extra_help: &[],
        handler: super::handle_dump,
    },
    Command {
        name: "theme",
        usages: &[
            CommandUsage {
                syntax: "/theme",
                description:
                    "Pick a theme (built-in or custom) with filtering and sorting options.",
            },
            CommandUsage {
                syntax: "/theme <id>",
                description: "Apply a theme by id and persist the selection to config.",
            },
        ],
        extra_help: &[],
        handler: super::handle_theme,
    },
    Command {
        name: "model",
        usages: &[
            CommandUsage {
                syntax: "/model",
                description:
                    "Pick a model from the current provider with filtering, sorting, and metadata.",
            },
            CommandUsage {
                syntax: "/model <id>",
                description: "Switch to the specified model for this session only.",
            },
        ],
        extra_help: &[],
        handler: super::handle_model,
    },
    Command {
        name: "provider",
        usages: &[
            CommandUsage {
                syntax: "/provider",
                description: "Pick a provider with filtering and sorting.",
            },
            CommandUsage {
                syntax: "/provider <id>",
                description: "Switch to the specified provider for this session only.",
            },
        ],
        extra_help: &[],
        handler: super::handle_provider,
    },
    Command {
        name: "markdown",
        usages: &[CommandUsage {
            syntax: "/markdown [on|off|toggle]",
            description: "Toggle Markdown rendering and persist the preference to config.",
        }],
        extra_help: &[],
        handler: super::handle_markdown,
    },
    Command {
        name: "syntax",
        usages: &[CommandUsage {
            syntax: "/syntax [on|off|toggle]",
            description: "Toggle code syntax highlighting and persist the preference to config.",
        }],
        extra_help: &[],
        handler: super::handle_syntax,
    },
    Command {
        name: "character",
        usages: &[
            CommandUsage {
                syntax: "/character",
                description:
                    "Pick a character card from available cards with filtering and sorting.",
            },
            CommandUsage {
                syntax: "/character <name>",
                description: "Load the specified character card for this session.",
            },
        ],
        extra_help: &[],
        handler: super::handle_character,
    },
    Command {
        name: "persona",
        usages: &[
            CommandUsage {
                syntax: "/persona",
                description: "Pick a persona from available personas with filtering and sorting.",
            },
            CommandUsage {
                syntax: "/persona <id>",
                description: "Activate the specified persona for this session.",
            },
        ],
        extra_help: &[],
        handler: super::handle_persona,
    },
    Command {
        name: "preset",
        usages: &[
            CommandUsage {
                syntax: "/preset",
                description: "Pick a preset from available presets with filtering and sorting.",
            },
            CommandUsage {
                syntax: "/preset <id>",
                description: "Activate the specified preset for this session.",
            },
        ],
        extra_help: &[],
        handler: super::handle_preset,
    },
];
