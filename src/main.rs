// Declare all modules
mod api;
mod auth;
mod character;
mod cli;
mod commands;
mod core;
mod ui;
mod utils;

fn main() {
    if let Err(err) = cli::main() {
        if let Some(config_err) = err.downcast_ref::<crate::core::config::ConfigError>() {
            eprintln!("❌ Failed to load configuration: {config_err}");
        } else {
            eprintln!("Error: {err}");
        }
        std::process::exit(1);
    }
}
