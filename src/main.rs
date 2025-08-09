// Declare all modules
mod api;
mod core;
mod auth;
mod commands;
mod ui;
mod utils;
mod cli;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    cli::main()
}
