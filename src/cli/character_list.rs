use crate::character::loader::{get_cards_dir, list_available_cards};
use crate::core::config::path_display;
use std::error::Error;

pub async fn list_characters() -> Result<(), Box<dyn Error>> {
    let cards_dir = get_cards_dir();
    let cards_dir_display = path_display(&cards_dir);

    println!("Available character cards (from {}):\n", cards_dir_display);

    match list_available_cards() {
        Ok(cards) => {
            if cards.is_empty() {
                println!("  No character cards found.");
                println!("\n💡 Import character cards with:");
                println!("   chabeau import -c <file.json|file.png>");
            } else {
                for (name, path) in cards {
                    let filename = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown");
                    println!("  • {} ({})", name, filename);
                }
                println!("\n💡 Use a character with:");
                println!("   chabeau -c <character_name>");
            }
        }
        Err(e) => {
            eprintln!("❌ Error listing character cards: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}
