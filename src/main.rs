// Declare all modules
mod api;
mod auth;
mod builtin_providers;
mod cli;
mod commands;
mod core;
mod ui;
mod utils;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    cli::main()
}
