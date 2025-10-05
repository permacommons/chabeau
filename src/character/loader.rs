use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;
use crate::character::CharacterCard;

/// Errors that can occur when loading character cards
#[derive(Debug)]
pub enum CardLoadError {
    /// File could not be found or read
    FileNotFound(String),
    /// JSON parsing failed
    InvalidJson(String),
    /// PNG parsing failed
    InvalidPng(String),
    /// PNG metadata missing
    MissingMetadata(String),
    /// Card validation failed
    ValidationFailed(Vec<String>),
}

impl fmt::Display for CardLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CardLoadError::FileNotFound(msg) => {
                write!(f, "File not found: {}", msg)
            }
            CardLoadError::InvalidJson(msg) => {
                write!(f, "Invalid JSON: {}", msg)
            }
            CardLoadError::InvalidPng(msg) => {
                write!(f, "Invalid PNG: {}", msg)
            }
            CardLoadError::MissingMetadata(msg) => {
                write!(f, "Missing metadata: {}", msg)
            }
            CardLoadError::ValidationFailed(errors) => {
                write!(f, "Card validation failed:\n")?;
                for error in errors {
                    write!(f, "  • {}\n", error)?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for CardLoadError {}

/// Get the cards directory path
/// Returns the path to the cards directory in the config directory
pub fn get_cards_dir() -> PathBuf {
    let proj_dirs = directories::ProjectDirs::from("org", "permacommons", "chabeau")
        .expect("Failed to determine config directory");
    proj_dirs.config_dir().join("cards")
}

/// List all available character cards in the cards directory
/// Returns a vector of tuples containing (character_name, file_path)
pub fn list_available_cards() -> Result<Vec<(String, PathBuf)>, Box<dyn std::error::Error>> {
    let cards_dir = get_cards_dir();
    
    // If the cards directory doesn't exist, return an empty list
    if !cards_dir.exists() {
        return Ok(Vec::new());
    }
    
    let mut cards = Vec::new();
    
    // Scan the directory for card files
    for entry in fs::read_dir(cards_dir)? {
        let entry = entry?;
        let path = entry.path();
        
        // Only process files (not directories)
        if path.is_file() {
            let extension = path.extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_lowercase());
            
            // Check if it's a JSON or PNG file
            if matches!(extension.as_deref(), Some("json") | Some("png")) {
                // Try to load the card to get its name
                // If loading fails, skip this file (it will be logged elsewhere)
                match load_card(&path) {
                    Ok(card) => {
                        cards.push((card.data.name.clone(), path));
                    }
                    Err(_) => {
                        // Skip invalid cards silently during listing
                        // The error will be shown when the user tries to use the card
                    }
                }
            }
        }
    }
    
    // Sort by character name for consistent ordering
    cards.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(cards)
}

/// Find a character card by name with fallback logic
/// 
/// Search order:
/// 1. Check for {name}.json in cards directory
/// 2. Check for {name}.png in cards directory
/// 3. Search all cards for a matching character name (case-insensitive)
/// 
/// Returns the loaded card and its file path
pub fn find_card_by_name(name: &str) -> Result<(CharacterCard, PathBuf), Box<dyn std::error::Error>> {
    let cards_dir = get_cards_dir();
    
    // Try as direct filename (without extension)
    for ext in &["json", "png"] {
        let path = cards_dir.join(format!("{}.{}", name, ext));
        if path.exists() {
            let card = load_card(&path)?;
            return Ok((card, path));
        }
    }
    
    // Try as character name (search all cards)
    let cards = list_available_cards()?;
    for (card_name, path) in cards {
        if card_name.eq_ignore_ascii_case(name) {
            let card = load_card(&path)?;
            return Ok((card, path));
        }
    }
    
    Err(format!("Character '{}' not found in cards directory", name).into())
}

/// Load a character card from a file (JSON or PNG)
/// Automatically detects the file type based on extension
fn load_card<P: AsRef<Path>>(path: P) -> Result<CharacterCard, CardLoadError> {
    let path = path.as_ref();
    let extension = path.extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase());
    
    match extension.as_deref() {
        Some("json") => load_json_card(path),
        Some("png") => load_png_card(path),
        _ => Err(CardLoadError::InvalidJson(
            format!("{}: File must be .json or .png", path.display())
        )),
    }
}

/// Load a character card from a JSON file
pub fn load_json_card<P: AsRef<Path>>(path: P) -> Result<CharacterCard, CardLoadError> {
    let path = path.as_ref();
    
    // Read file contents
    let contents = fs::read_to_string(path).map_err(|e| {
        CardLoadError::FileNotFound(format!("{}: {}", path.display(), e))
    })?;
    
    // Parse JSON
    let card: CharacterCard = serde_json::from_str(&contents).map_err(|e| {
        CardLoadError::InvalidJson(format!("{}: {}", path.display(), e))
    })?;
    
    // Validate the card
    validate_card(&card)?;
    
    Ok(card)
}

/// Load a character card from a PNG file with embedded metadata
pub fn load_png_card<P: AsRef<Path>>(path: P) -> Result<CharacterCard, CardLoadError> {
    let path = path.as_ref();
    
    // Open the PNG file
    let file = fs::File::open(path).map_err(|e| {
        CardLoadError::FileNotFound(format!("{}: {}", path.display(), e))
    })?;
    
    // Create PNG decoder
    let decoder = png::Decoder::new(file);
    let reader = decoder.read_info().map_err(|e| {
        CardLoadError::InvalidPng(format!("{}: {}", path.display(), e))
    })?;
    
    // Extract tEXt chunk with key "chara"
    let info = reader.info();
    let chara_text = info
        .uncompressed_latin1_text
        .iter()
        .find(|chunk| chunk.keyword == "chara")
        .ok_or_else(|| {
            CardLoadError::MissingMetadata(format!(
                "{}: PNG does not contain 'chara' metadata in tEXt chunk",
                path.display()
            ))
        })?;
    
    // Base64 decode the chara data
    let decoded = base64::prelude::BASE64_STANDARD
        .decode(&chara_text.text)
        .map_err(|e| {
            CardLoadError::InvalidJson(format!("{}: Base64 decode failed: {}", path.display(), e))
        })?;
    
    // Convert to UTF-8 string
    let json_str = String::from_utf8(decoded).map_err(|e| {
        CardLoadError::InvalidJson(format!("{}: UTF-8 decode failed: {}", path.display(), e))
    })?;
    
    // Parse JSON
    let card: CharacterCard = serde_json::from_str(&json_str).map_err(|e| {
        CardLoadError::InvalidJson(format!("{}: {}", path.display(), e))
    })?;
    
    // Validate the card
    validate_card(&card)?;
    
    Ok(card)
}

/// Validate a character card against the v2 specification
pub fn validate_card(card: &CharacterCard) -> Result<(), CardLoadError> {
    let mut errors = Vec::new();
    
    // Check spec field
    if card.spec != "chara_card_v2" {
        errors.push(format!(
            "Invalid spec field: expected 'chara_card_v2', got '{}'",
            card.spec
        ));
    }
    
    // Check that name is not empty (this is the most critical field)
    if card.data.name.is_empty() {
        errors.push("Character name is required and cannot be empty".to_string());
    }
    
    // Note: Other fields (description, personality, scenario, first_mes, mes_example)
    // are required by the struct definition (serde will fail if they're missing),
    // but they can be empty strings in practice. Many real-world character cards
    // have empty values for some of these fields.
    
    if !errors.is_empty() {
        return Err(CardLoadError::ValidationFailed(errors));
    }
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;
    use crate::character::CharacterData;

    fn create_valid_card_json() -> String {
        serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "Test Character",
                "description": "A test character for unit tests",
                "personality": "Friendly and helpful",
                "scenario": "Testing environment",
                "first_mes": "Hello! I'm a test character.",
                "mes_example": "{{user}}: Hi\n{{char}}: Hello!"
            }
        })
        .to_string()
    }

    #[test]
    fn test_load_valid_json_card() {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(create_valid_card_json().as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let result = load_json_card(temp_file.path());
        assert!(result.is_ok());

        let card = result.unwrap();
        assert_eq!(card.spec, "chara_card_v2");
        assert_eq!(card.data.name, "Test Character");
        assert_eq!(card.data.description, "A test character for unit tests");
    }

    #[test]
    fn test_load_json_card_with_optional_fields() {
        let json = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "Test Character",
                "description": "A test character",
                "personality": "Friendly",
                "scenario": "Testing",
                "first_mes": "Hello!",
                "mes_example": "Example",
                "creator_notes": "Some notes",
                "system_prompt": "You are helpful",
                "post_history_instructions": "Be polite",
                "alternate_greetings": ["Hi!", "Hey!"],
                "tags": ["test", "friendly"],
                "creator": "Test Creator",
                "character_version": "1.0"
            }
        })
        .to_string();

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(json.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let result = load_json_card(temp_file.path());
        assert!(result.is_ok());

        let card = result.unwrap();
        assert_eq!(card.data.creator_notes, Some("Some notes".to_string()));
        assert_eq!(card.data.system_prompt, Some("You are helpful".to_string()));
        assert_eq!(
            card.data.post_history_instructions,
            Some("Be polite".to_string())
        );
        assert_eq!(card.data.alternate_greetings.as_ref().unwrap().len(), 2);
        assert_eq!(card.data.tags.as_ref().unwrap().len(), 2);
        assert_eq!(card.data.creator, Some("Test Creator".to_string()));
        assert_eq!(card.data.character_version, Some("1.0".to_string()));
    }

    #[test]
    fn test_load_json_card_file_not_found() {
        let result = load_json_card("/nonexistent/path/to/card.json");
        assert!(result.is_err());

        match result.unwrap_err() {
            CardLoadError::FileNotFound(msg) => {
                assert!(msg.contains("/nonexistent/path/to/card.json"));
            }
            _ => panic!("Expected FileNotFound error"),
        }
    }

    #[test]
    fn test_load_json_card_invalid_json() {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"{ invalid json }").unwrap();
        temp_file.flush().unwrap();

        let result = load_json_card(temp_file.path());
        assert!(result.is_err());

        match result.unwrap_err() {
            CardLoadError::InvalidJson(_) => {}
            _ => panic!("Expected InvalidJson error"),
        }
    }

    #[test]
    fn test_load_json_card_missing_required_field() {
        let json = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "Test Character",
                "description": "A test character",
                "personality": "Friendly",
                "scenario": "Testing",
                // Missing first_mes
                "mes_example": "Example"
            }
        })
        .to_string();

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(json.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let result = load_json_card(temp_file.path());
        assert!(result.is_err());

        match result.unwrap_err() {
            CardLoadError::InvalidJson(_) => {}
            _ => panic!("Expected InvalidJson error for missing required field"),
        }
    }

    #[test]
    fn test_validate_card_valid() {
        let card = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "Test".to_string(),
                description: "Test description".to_string(),
                personality: "Test personality".to_string(),
                scenario: "Test scenario".to_string(),
                first_mes: "Hello".to_string(),
                mes_example: "Example".to_string(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        let result = validate_card(&card);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_card_invalid_spec() {
        let card = CharacterCard {
            spec: "invalid_spec".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "Test".to_string(),
                description: "Test description".to_string(),
                personality: "Test personality".to_string(),
                scenario: "Test scenario".to_string(),
                first_mes: "Hello".to_string(),
                mes_example: "Example".to_string(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        let result = validate_card(&card);
        assert!(result.is_err());

        match result.unwrap_err() {
            CardLoadError::ValidationFailed(errors) => {
                assert_eq!(errors.len(), 1);
                assert!(errors[0].contains("Invalid spec field"));
            }
            _ => panic!("Expected ValidationFailed error"),
        }
    }

    #[test]
    fn test_validate_card_empty_name() {
        let card = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "".to_string(),
                description: "Test description".to_string(),
                personality: "Test personality".to_string(),
                scenario: "Test scenario".to_string(),
                first_mes: "Hello".to_string(),
                mes_example: "Example".to_string(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        let result = validate_card(&card);
        assert!(result.is_err());

        match result.unwrap_err() {
            CardLoadError::ValidationFailed(errors) => {
                assert!(errors.iter().any(|e| e.contains("name")));
            }
            _ => panic!("Expected ValidationFailed error"),
        }
    }

    #[test]
    fn test_validate_card_multiple_errors() {
        let card = CharacterCard {
            spec: "invalid_spec".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "".to_string(),
                description: "".to_string(),
                personality: "".to_string(),
                scenario: "".to_string(),
                first_mes: "".to_string(),
                mes_example: "Example".to_string(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        let result = validate_card(&card);
        assert!(result.is_err());

        match result.unwrap_err() {
            CardLoadError::ValidationFailed(errors) => {
                assert_eq!(errors.len(), 2); // invalid spec and empty name
            }
            _ => panic!("Expected ValidationFailed error"),
        }
    }

    #[test]
    fn test_validate_card_empty_fields_allowed() {
        // Test that empty strings are allowed for most fields (except name)
        let card = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "Test".to_string(),
                description: "".to_string(),
                personality: "".to_string(),
                scenario: "".to_string(),
                first_mes: "".to_string(),
                mes_example: "".to_string(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        let result = validate_card(&card);
        assert!(result.is_ok());
    }

    #[test]
    fn test_card_load_error_display() {
        let error = CardLoadError::FileNotFound("test.json: No such file".to_string());
        assert_eq!(
            format!("{}", error),
            "File not found: test.json: No such file"
        );

        let error = CardLoadError::InvalidJson("test.json: Parse error".to_string());
        assert_eq!(format!("{}", error), "Invalid JSON: test.json: Parse error");

        let error = CardLoadError::ValidationFailed(vec![
            "Error 1".to_string(),
            "Error 2".to_string(),
        ]);
        let display = format!("{}", error);
        assert!(display.contains("Card validation failed"));
        assert!(display.contains("Error 1"));
        assert!(display.contains("Error 2"));
    }

    #[test]
    fn test_load_picard_json() {
        // Test with the actual picard.json file if it exists
        let picard_path = "test-cards/picard.json";
        if std::path::Path::new(picard_path).exists() {
            let result = load_json_card(picard_path);
            assert!(result.is_ok(), "Failed to load picard.json: {:?}", result.err());

            let card = result.unwrap();
            assert_eq!(card.spec, "chara_card_v2");
            assert_eq!(card.data.name, "Jean Luc Picard");
            assert!(!card.data.description.is_empty());
            assert!(!card.data.first_mes.is_empty());
            assert_eq!(card.data.creator, Some("thekrautissour".to_string()));
            assert!(card.data.tags.is_some());
        }
    }

    #[test]
    fn test_load_simple_test_card() {
        // Test with the simple test card
        let test_path = "test-cards/test_simple.json";
        if std::path::Path::new(test_path).exists() {
            let result = load_json_card(test_path);
            assert!(result.is_ok(), "Failed to load test_simple.json: {:?}", result.err());

            let card = result.unwrap();
            assert_eq!(card.spec, "chara_card_v2");
            assert_eq!(card.data.name, "Simple Test Character");
            assert_eq!(card.data.description, "A simple test character for validation");
            assert_eq!(card.data.personality, "Friendly and helpful");
        }
    }

    #[test]
    fn test_load_invalid_test_card() {
        // Test with an invalid test card
        let test_path = "test-cards/test_invalid.json";
        if std::path::Path::new(test_path).exists() {
            let result = load_json_card(test_path);
            assert!(result.is_err(), "Expected error loading test_invalid.json");

            match result.unwrap_err() {
                CardLoadError::ValidationFailed(errors) => {
                    // Should have errors for wrong spec and empty name
                    assert!(errors.len() >= 1);
                    assert!(errors.iter().any(|e| e.contains("spec") || e.contains("name")));
                }
                _ => panic!("Expected ValidationFailed error"),
            }
        }
    }

    // PNG loading tests

    #[test]
    fn test_load_png_card_with_metadata() {
        // Create a test PNG with embedded character data
        use std::io::BufWriter;
        
        let temp_file = NamedTempFile::new().unwrap();
        let file = fs::File::create(temp_file.path()).unwrap();
        let w = BufWriter::new(file);

        let mut encoder = png::Encoder::new(w, 100, 100);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);

        // Create character card JSON
        let card_json = create_valid_card_json();
        let encoded = base64::prelude::BASE64_STANDARD.encode(card_json.as_bytes());

        // Add tEXt chunk with chara metadata
        encoder.add_text_chunk("chara".to_string(), encoded).unwrap();

        let mut writer = encoder.write_header().unwrap();
        
        // Write a simple RGB image (100x100 pixels, all black)
        let data = vec![0u8; 100 * 100 * 3];
        writer.write_image_data(&data).unwrap();
        writer.finish().unwrap();

        // Now try to load the PNG card
        let result = load_png_card(temp_file.path());
        assert!(result.is_ok(), "Failed to load PNG card: {:?}", result.err());

        let card = result.unwrap();
        assert_eq!(card.spec, "chara_card_v2");
        assert_eq!(card.data.name, "Test Character");
        assert_eq!(card.data.description, "A test character for unit tests");
    }

    #[test]
    fn test_load_png_card_without_metadata() {
        // Create a PNG without chara metadata
        use std::io::BufWriter;
        
        let temp_file = NamedTempFile::new().unwrap();
        let file = fs::File::create(temp_file.path()).unwrap();
        let w = BufWriter::new(file);

        let mut encoder = png::Encoder::new(w, 100, 100);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);

        let mut writer = encoder.write_header().unwrap();
        let data = vec![0u8; 100 * 100 * 3];
        writer.write_image_data(&data).unwrap();
        writer.finish().unwrap();

        // Try to load the PNG card - should fail with MissingMetadata
        let result = load_png_card(temp_file.path());
        assert!(result.is_err());

        match result.unwrap_err() {
            CardLoadError::MissingMetadata(msg) => {
                assert!(msg.contains("chara"));
            }
            _ => panic!("Expected MissingMetadata error"),
        }
    }

    #[test]
    fn test_load_png_card_invalid_base64() {
        // Create a PNG with invalid base64 in chara metadata
        use std::io::BufWriter;
        
        let temp_file = NamedTempFile::new().unwrap();
        let file = fs::File::create(temp_file.path()).unwrap();
        let w = BufWriter::new(file);

        let mut encoder = png::Encoder::new(w, 100, 100);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);

        // Add invalid base64
        encoder.add_text_chunk("chara".to_string(), "not-valid-base64!!!".to_string()).unwrap();

        let mut writer = encoder.write_header().unwrap();
        let data = vec![0u8; 100 * 100 * 3];
        writer.write_image_data(&data).unwrap();
        writer.finish().unwrap();

        let result = load_png_card(temp_file.path());
        assert!(result.is_err());

        match result.unwrap_err() {
            CardLoadError::InvalidJson(msg) => {
                assert!(msg.contains("Base64 decode failed") || msg.contains("UTF-8 decode failed") || msg.contains("expected"));
            }
            _ => panic!("Expected InvalidJson error"),
        }
    }

    #[test]
    fn test_load_png_card_invalid_json() {
        // Create a PNG with valid base64 but invalid JSON
        use std::io::BufWriter;
        
        let temp_file = NamedTempFile::new().unwrap();
        let file = fs::File::create(temp_file.path()).unwrap();
        let w = BufWriter::new(file);

        let mut encoder = png::Encoder::new(w, 100, 100);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);

        // Encode invalid JSON
        let invalid_json = "{ this is not valid json }";
        let encoded = base64::prelude::BASE64_STANDARD.encode(invalid_json.as_bytes());
        encoder.add_text_chunk("chara".to_string(), encoded).unwrap();

        let mut writer = encoder.write_header().unwrap();
        let data = vec![0u8; 100 * 100 * 3];
        writer.write_image_data(&data).unwrap();
        writer.finish().unwrap();

        let result = load_png_card(temp_file.path());
        assert!(result.is_err());

        match result.unwrap_err() {
            CardLoadError::InvalidJson(_) => {}
            _ => panic!("Expected InvalidJson error"),
        }
    }

    #[test]
    fn test_load_png_card_file_not_found() {
        let result = load_png_card("/nonexistent/path/to/card.png");
        assert!(result.is_err());

        match result.unwrap_err() {
            CardLoadError::FileNotFound(msg) => {
                assert!(msg.contains("/nonexistent/path/to/card.png"));
            }
            _ => panic!("Expected FileNotFound error"),
        }
    }

    #[test]
    fn test_load_png_card_not_a_png() {
        // Try to load a non-PNG file as PNG
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"This is not a PNG file").unwrap();
        temp_file.flush().unwrap();

        let result = load_png_card(temp_file.path());
        assert!(result.is_err());

        match result.unwrap_err() {
            CardLoadError::InvalidPng(_) => {}
            _ => panic!("Expected InvalidPng error"),
        }
    }

    #[test]
    fn test_load_picard_png() {
        // Test with the actual picard.png file if it exists
        let picard_path = "test-cards/picard.png";
        if std::path::Path::new(picard_path).exists() {
            let result = load_png_card(picard_path);
            assert!(result.is_ok(), "Failed to load picard.png: {:?}", result.err());

            let card = result.unwrap();
            assert_eq!(card.spec, "chara_card_v2");
            assert_eq!(card.data.name, "Jean Luc Picard");
            assert!(!card.data.description.is_empty());
            assert!(!card.data.first_mes.is_empty());
        }
    }

    #[test]
    fn test_load_spec_v2_png_cards() {
        // Test with the spec v2 PNG files if they exist
        let test_cards = [
            "test-cards/main_data-soong_spec_v2.png",
            "test-cards/main_moon-b1677cd10d61_spec_v2.png",
            "test-cards/main_seven-of-nine-8b4cd2352ade_spec_v2.png",
        ];

        for card_path in &test_cards {
            if std::path::Path::new(card_path).exists() {
                let result = load_png_card(card_path);
                assert!(
                    result.is_ok(),
                    "Failed to load {}: {:?}",
                    card_path,
                    result.err()
                );

                let card = result.unwrap();
                assert_eq!(card.spec, "chara_card_v2");
                assert!(!card.data.name.is_empty());
                println!("Successfully loaded: {} ({})", card_path, card.data.name);
            }
        }
    }

    #[test]
    fn test_png_and_json_equivalence() {
        // If both picard.json and picard.png exist, they should have the same data
        let json_path = "test-cards/picard.json";
        let png_path = "test-cards/picard.png";

        if std::path::Path::new(json_path).exists() && std::path::Path::new(png_path).exists() {
            let json_card = load_json_card(json_path).unwrap();
            let png_card = load_png_card(png_path).unwrap();

            assert_eq!(json_card.spec, png_card.spec);
            assert_eq!(json_card.data.name, png_card.data.name);
            assert_eq!(json_card.data.description, png_card.data.description);
            assert_eq!(json_card.data.personality, png_card.data.personality);
            assert_eq!(json_card.data.scenario, png_card.data.scenario);
            assert_eq!(json_card.data.first_mes, png_card.data.first_mes);
        }
    }

    #[test]
    fn test_card_load_error_display_png_errors() {
        let error = CardLoadError::InvalidPng("test.png: Invalid format".to_string());
        assert_eq!(format!("{}", error), "Invalid PNG: test.png: Invalid format");

        let error = CardLoadError::MissingMetadata("test.png: No chara chunk".to_string());
        assert_eq!(
            format!("{}", error),
            "Missing metadata: test.png: No chara chunk"
        );
    }

    // Card discovery tests

    #[test]
    fn test_get_cards_dir() {
        let cards_dir = get_cards_dir();
        assert!(cards_dir.to_string_lossy().contains("chabeau"));
        assert!(cards_dir.to_string_lossy().contains("cards"));
    }

    #[test]
    fn test_list_available_cards_empty_directory() {
        // When the cards directory doesn't exist, should return empty list
        // This test assumes the cards directory doesn't exist or is empty
        // In a real scenario, we'd use a temp directory
        let result = list_available_cards();
        assert!(result.is_ok());
        // We can't assert it's empty because the user might have cards installed
    }

    #[test]
    fn test_list_available_cards_with_test_cards() {
        // If test-cards directory has valid cards, we can test with those
        // But since list_available_cards() looks in the config directory,
        // we need to test with actual config directory cards
        let result = list_available_cards();
        assert!(result.is_ok());
        
        let cards = result.unwrap();
        // Cards should be sorted by name
        for i in 1..cards.len() {
            assert!(cards[i - 1].0 <= cards[i].0);
        }
    }

    #[test]
    fn test_find_card_by_name_not_found() {
        let result = find_card_by_name("nonexistent_character_that_does_not_exist");
        assert!(result.is_err());
        
        let err = result.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn test_load_card_json() {
        // Test loading a JSON card through the load_card function
        let test_path = "test-cards/test_simple.json";
        if std::path::Path::new(test_path).exists() {
            let result = load_card(test_path);
            assert!(result.is_ok());
            
            let card = result.unwrap();
            assert_eq!(card.data.name, "Simple Test Character");
        }
    }

    #[test]
    fn test_load_card_png() {
        // Test loading a PNG card through the load_card function
        let test_path = "test-cards/picard.png";
        if std::path::Path::new(test_path).exists() {
            let result = load_card(test_path);
            assert!(result.is_ok());
            
            let card = result.unwrap();
            assert_eq!(card.data.name, "Jean Luc Picard");
        }
    }

    #[test]
    fn test_load_card_invalid_extension() {
        let mut temp_file = NamedTempFile::with_suffix(".txt").unwrap();
        temp_file.write_all(b"some content").unwrap();
        temp_file.flush().unwrap();
        
        let result = load_card(temp_file.path());
        assert!(result.is_err());
        
        match result.unwrap_err() {
            CardLoadError::InvalidJson(msg) => {
                assert!(msg.contains("must be .json or .png"));
            }
            _ => panic!("Expected InvalidJson error for invalid extension"),
        }
    }

    #[test]
    fn test_card_discovery_with_temp_directory() {
        // Create a temporary directory structure to test card discovery
        use tempfile::TempDir;
        
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path();
        
        // Create a valid JSON card
        let card1_path = temp_path.join("alice.json");
        fs::write(&card1_path, create_valid_card_json()).unwrap();
        
        // Create another valid JSON card with different name
        let card2_json = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "Bob",
                "description": "Another test character",
                "personality": "Serious",
                "scenario": "Testing",
                "first_mes": "Hello, I'm Bob.",
                "mes_example": "Example"
            }
        }).to_string();
        let card2_path = temp_path.join("bob.json");
        fs::write(&card2_path, card2_json).unwrap();
        
        // Create an invalid file that should be skipped
        let invalid_path = temp_path.join("invalid.json");
        fs::write(&invalid_path, "not valid json").unwrap();
        
        // Create a non-card file that should be ignored
        let other_path = temp_path.join("readme.txt");
        fs::write(&other_path, "This is not a card").unwrap();
        
        // Note: We can't easily test list_available_cards() with a temp directory
        // because it uses the actual config directory. Instead, we test the logic
        // by manually scanning the temp directory using the same pattern.
        
        let mut cards = Vec::new();
        for entry in fs::read_dir(temp_path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            
            if path.is_file() {
                let extension = path.extension()
                    .and_then(|e| e.to_str())
                    .map(|s| s.to_lowercase());
                
                if matches!(extension.as_deref(), Some("json") | Some("png")) {
                    if let Ok(card) = load_card(&path) {
                        cards.push((card.data.name.clone(), path));
                    }
                }
            }
        }
        
        cards.sort_by(|a, b| a.0.cmp(&b.0));
        
        // Should have found 2 valid cards (alice and bob), skipped invalid and readme
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].0, "Bob");
        assert_eq!(cards[1].0, "Test Character");
    }

    #[test]
    fn test_find_card_by_filename_without_extension() {
        // Create a temporary directory with a test card
        use tempfile::TempDir;
        
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path();
        
        // Create a card file
        let card_path = temp_path.join("testchar.json");
        fs::write(&card_path, create_valid_card_json()).unwrap();
        
        // Test the logic of finding by filename (without actually using find_card_by_name
        // since it uses the config directory)
        
        // Check that the file exists with .json extension
        let json_path = temp_path.join("testchar.json");
        assert!(json_path.exists());
        
        // Check that we can load it
        let result = load_card(&json_path);
        assert!(result.is_ok());
    }

    #[test]
    fn test_find_card_by_character_name_case_insensitive() {
        // Test that character name matching is case-insensitive
        use tempfile::TempDir;
        
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path();
        
        // Create a card with a specific character name
        let card_json = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "Captain Picard",
                "description": "Test",
                "personality": "Test",
                "scenario": "Test",
                "first_mes": "Test",
                "mes_example": "Test"
            }
        }).to_string();
        let card_path = temp_path.join("picard_card.json");
        fs::write(&card_path, card_json).unwrap();
        
        // Simulate the search logic
        let cards = vec![("Captain Picard".to_string(), card_path.clone())];
        
        // Test case-insensitive matching
        let search_name = "captain picard";
        let found = cards.iter().find(|(name, _)| name.eq_ignore_ascii_case(search_name));
        assert!(found.is_some());
        
        let search_name = "CAPTAIN PICARD";
        let found = cards.iter().find(|(name, _)| name.eq_ignore_ascii_case(search_name));
        assert!(found.is_some());
        
        let search_name = "CaPtAiN pIcArD";
        let found = cards.iter().find(|(name, _)| name.eq_ignore_ascii_case(search_name));
        assert!(found.is_some());
    }

    #[test]
    fn test_card_discovery_prefers_json_over_png() {
        // Test that when both .json and .png exist with the same name,
        // the search finds the .json first
        use tempfile::TempDir;
        
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path();
        
        // Create both JSON and PNG versions
        let json_path = temp_path.join("character.json");
        let png_path = temp_path.join("character.png");
        
        fs::write(&json_path, create_valid_card_json()).unwrap();
        // Create a dummy PNG file (we're just testing the search order)
        fs::write(&png_path, b"fake png").unwrap();
        
        // Test the search order logic
        let name = "character";
        let mut found_path = None;
        
        for ext in &["json", "png"] {
            let path = temp_path.join(format!("{}.{}", name, ext));
            if path.exists() {
                found_path = Some(path);
                break;
            }
        }
        
        assert!(found_path.is_some());
        assert_eq!(found_path.unwrap(), json_path);
    }

    #[test]
    fn test_list_cards_ignores_subdirectories() {
        // Test that subdirectories are ignored during card discovery
        use tempfile::TempDir;
        
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path();
        
        // Create a valid card file
        let card_path = temp_path.join("valid.json");
        fs::write(&card_path, create_valid_card_json()).unwrap();
        
        // Create a subdirectory with a card in it (should be ignored)
        let subdir = temp_path.join("subdir");
        fs::create_dir(&subdir).unwrap();
        let subcard_path = subdir.join("subcard.json");
        fs::write(&subcard_path, create_valid_card_json()).unwrap();
        
        // Scan the directory (simulating list_available_cards logic)
        let mut cards = Vec::new();
        for entry in fs::read_dir(temp_path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            
            // Only process files, not directories
            if path.is_file() {
                let extension = path.extension()
                    .and_then(|e| e.to_str())
                    .map(|s| s.to_lowercase());
                
                if matches!(extension.as_deref(), Some("json") | Some("png")) {
                    if let Ok(card) = load_card(&path) {
                        cards.push((card.data.name.clone(), path));
                    }
                }
            }
        }
        
        // Should only find the card in the root directory, not the subdirectory
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].1, card_path);
    }
}
