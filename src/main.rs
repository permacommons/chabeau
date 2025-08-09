// Declare all modules
mod api;
mod app;
mod auth;
mod commands;
mod config;
mod logging;
mod message;
mod models;
mod scroll;
mod ui;

// Declare the new modules we created
mod chat_loop;
mod editor;
mod model_list;
mod provider_list;
mod set_default_model;

// Re-export the main function from cli module
mod cli;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    cli::main()
}
