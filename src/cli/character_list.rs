use crate::character::loader::get_cards_dir;
use crate::character::CharacterService;
use crate::core::config::path_display;
use std::error::Error;

pub async fn list_characters(service: &mut CharacterService) -> Result<(), Box<dyn Error>> {
    let cards_dir = get_cards_dir();
    let cards_dir_display = path_display(&cards_dir);

    println!("Available character cards (from {}):\n", cards_dir_display);

    match service.list_metadata_with_paths() {
        Ok(cards) => {
            if cards.is_empty() {
                println!("  No character cards found.");
                println!("\n💡 Import character cards with:");
                println!("   chabeau import <file.json|file.png>");
            } else {
                for (metadata, path) in cards {
                    let filename = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown");
                    println!("  • {} ({})", metadata.name, filename);
                    let description = metadata.description.trim();
                    if let Some(first_line) = description.lines().next() {
                        let summary = first_line.trim();
                        if !summary.is_empty() {
                            println!("    {}", summary);
                        }
                    }
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
