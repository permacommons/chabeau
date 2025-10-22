use super::CommandResult;
use crate::core::app::App;
use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

/// Function pointer used by the registry to invoke a command implementation.
///
/// Command handlers receive the shared [`App`] state plus the parsed
/// [`CommandInvocation`].
pub type CommandHandler = fn(&mut App, CommandInvocation<'_>) -> CommandResult;

/// One usage line that can be surfaced in command help.
pub struct CommandUsage {
    pub syntax: &'static str,
    pub description: &'static str,
}

/// Metadata describing a single slash command.
pub struct Command {
    pub name: &'static str,
    pub usages: &'static [CommandUsage],
    pub extra_help: &'static [&'static str],
    pub handler: CommandHandler,
}

/// Parsed view of a command input string, produced by [`CommandRegistry::dispatch`].
///
/// An invocation carries the original input (sans leading slash), the arguments as
/// contiguous text, and a cached token list for handlers that prefer positional
/// access.
pub struct CommandInvocation<'a> {
    pub command: &'static Command,
    pub input: &'a str,
    args: &'a str,
    tokens: Vec<&'a str>,
}

impl<'a> fmt::Debug for CommandInvocation<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandInvocation")
            .field("command", &self.command.name)
            .field("input", &self.input)
            .field("args", &self.args)
            .field("tokens", &self.tokens)
            .finish()
    }
}

impl<'a> CommandInvocation<'a> {
    /// Returns the raw argument substring after the command name.
    pub fn args_text(&self) -> &'a str {
        self.args
    }

    #[cfg(test)]
    /// Returns an iterator over whitespace-delimited argument tokens.
    pub fn args_iter(&'a self) -> impl Iterator<Item = &'a str> + 'a {
        self.tokens.iter().copied()
    }

    /// Returns the number of whitespace-delimited tokens in the invocation.
    pub fn args_len(&self) -> usize {
        self.tokens.len()
    }

    /// Returns the `index`th argument token if it exists.
    pub fn arg(&self, index: usize) -> Option<&'a str> {
        self.tokens.get(index).copied()
    }

    /// Maps the first argument onto a toggle intent. If no argument is provided
    /// the command toggles its state; otherwise the handler must pass a known
    /// literal such as `on`, `off`, or `toggle`.
    pub fn toggle_action(&self) -> Result<ToggleAction, ToggleError<'a>> {
        match self.arg(0) {
            None => Ok(ToggleAction::Toggle),
            Some(arg) if arg.eq_ignore_ascii_case("toggle") => Ok(ToggleAction::Toggle),
            Some(arg) if arg.eq_ignore_ascii_case("on") => Ok(ToggleAction::Enable),
            Some(arg) if arg.eq_ignore_ascii_case("off") => Ok(ToggleAction::Disable),
            Some(arg) => Err(ToggleError::InvalidValue(arg)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToggleAction {
    Enable,
    Disable,
    Toggle,
}

impl ToggleAction {
    pub fn apply(self, current: bool) -> bool {
        match self {
            ToggleAction::Enable => true,
            ToggleAction::Disable => false,
            ToggleAction::Toggle => !current,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ToggleError<'a> {
    InvalidValue(&'a str),
}

#[derive(Debug)]
/// Result of attempting to dispatch an input string through the registry.
pub enum DispatchOutcome<'a> {
    Invocation(CommandInvocation<'a>),
    NotACommand,
    UnknownCommand,
}

/// Central registry that owns the statically-defined command table and handles
/// parsing/dispatch.
pub struct CommandRegistry {
    commands: &'static [Command],
    lookup: HashMap<String, usize>,
}

impl CommandRegistry {
    /// Builds a registry for the statically-declared command table.
    pub fn new() -> Self {
        let mut lookup = HashMap::new();
        for (index, command) in COMMANDS.iter().enumerate() {
            lookup.insert(command.name.to_ascii_lowercase(), index);
        }
        Self {
            commands: COMMANDS,
            lookup,
        }
    }

    /// Returns every command known to the registry.
    pub fn all(&self) -> &'static [Command] {
        self.commands
    }

    /// Looks up a command by name using case-insensitive matching.
    pub fn find(&self, name: &str) -> Option<&'static Command> {
        let key = name.to_ascii_lowercase();
        self.lookup
            .get(&key)
            .and_then(|index| self.commands.get(*index))
    }

    /// Returns commands whose names share the provided prefix (case-insensitive).
    pub fn matching(&self, prefix: &str) -> Vec<&'static Command> {
        let lower_prefix = prefix.to_ascii_lowercase();
        self.commands
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

    /// Parses an input line once, splitting the command name from its arguments.
    ///
    /// Handlers receive a [`CommandInvocation`] that exposes cached argument
    /// tokens, allowing them to focus on business logic instead of parsing.
    pub fn dispatch<'a>(&'static self, input: &'a str) -> DispatchOutcome<'a> {
        let trimmed = input.trim();
        if !trimmed.starts_with('/') {
            return DispatchOutcome::NotACommand;
        }

        let body = trimmed[1..].trim();
        if body.is_empty() {
            return DispatchOutcome::UnknownCommand;
        }

        let (name, args) = match body.split_once(char::is_whitespace) {
            Some((name, rest)) => (name, rest.trim()),
            None => (body, ""),
        };

        let command = match self.find(name) {
            Some(cmd) => cmd,
            None => return DispatchOutcome::UnknownCommand,
        };

        let tokens: Vec<&'a str> = if args.is_empty() {
            Vec::new()
        } else {
            args.split_whitespace().collect()
        };

        DispatchOutcome::Invocation(CommandInvocation {
            command,
            input: trimmed,
            args,
            tokens,
        })
    }
}

static REGISTRY: LazyLock<CommandRegistry> = LazyLock::new(CommandRegistry::new);

/// Provides read-only access to the registered command metadata.
pub fn all_commands() -> &'static [Command] {
    REGISTRY.all()
}

#[cfg(test)]
pub fn find_command(name: &str) -> Option<&'static Command> {
    REGISTRY.find(name)
}

/// Returns commands whose names share the provided prefix.
pub fn matching_commands(prefix: &str) -> Vec<&'static Command> {
    REGISTRY.matching(prefix)
}

/// Exposes the lazily-initialised registry singleton for direct queries.
pub fn registry() -> &'static CommandRegistry {
    &REGISTRY
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
        name: "clear",
        usages: &[CommandUsage {
            syntax: "/clear",
            description: "Clear the conversation transcript.",
        }],
        extra_help: &[],
        handler: super::handle_clear,
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
