use chabeau::{cli, core};

fn main() {
    if let Err(err) = cli::main() {
        if let Some(config_err) = err.downcast_ref::<core::config::io::ConfigError>() {
            eprintln!("❌ Failed to load configuration: {config_err}");
        } else {
            eprintln!("Error: {err}");
        }
        std::process::exit(1);
    }
}
