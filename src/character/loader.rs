use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::character::{png_text, CharacterCard};
use base64::Engine;

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
                writeln!(f, "Card validation failed:")?;
                for error in errors {
                    writeln!(f, "  • {}", error)?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for CardLoadError {}

/// Get the cards directory path
/// Returns the path to the cards directory in the config directory
/// unless `CHABEAU_CARDS_DIR` is set to override it.
pub fn get_cards_dir() -> PathBuf {
    if let Some(override_dir) = std::env::var_os("CHABEAU_CARDS_DIR") {
        return PathBuf::from(override_dir);
    }

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
            let extension = path
                .extension()
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

/// Load a character card from a file (JSON or PNG)
/// Automatically detects the file type based on extension
pub fn load_card<P: AsRef<Path>>(path: P) -> Result<CharacterCard, CardLoadError> {
    let path = path.as_ref();
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase());

    match extension.as_deref() {
        Some("json") => load_json_card(path),
        Some("png") => load_png_card(path),
        _ => Err(CardLoadError::InvalidJson(format!(
            "{}: File must be .json or .png",
            path.display()
        ))),
    }
}

/// Load a character card from a JSON file
pub fn load_json_card<P: AsRef<Path>>(path: P) -> Result<CharacterCard, CardLoadError> {
    let path = path.as_ref();

    // Read file contents
    let contents = fs::read_to_string(path)
        .map_err(|e| CardLoadError::FileNotFound(format!("{}: {}", path.display(), e)))?;

    // Parse JSON
    let card: CharacterCard = serde_json::from_str(&contents)
        .map_err(|e| CardLoadError::InvalidJson(format!("{}: {}", path.display(), e)))?;

    // Validate the card
    validate_card(&card)?;

    Ok(card)
}

/// Load a character card from a PNG file with embedded metadata
pub fn load_png_card<P: AsRef<Path>>(path: P) -> Result<CharacterCard, CardLoadError> {
    let path = path.as_ref();

    let data = fs::read(path)
        .map_err(|e| CardLoadError::FileNotFound(format!("{}: {}", path.display(), e)))?;

    let chara_text = match png_text::extract_text(&data, "chara") {
        Ok(text) => text,
        Err(png_text::PngTextError::MissingKeyword(_)) => {
            return Err(CardLoadError::MissingMetadata(format!(
                "{}: PNG does not contain 'chara' metadata in tEXt chunk",
                path.display()
            )))
        }
        Err(err) => {
            return Err(CardLoadError::InvalidPng(format!(
                "{}: {}",
                path.display(),
                err
            )))
        }
    };

    // Base64 decode the chara data
    let decoded = base64::prelude::BASE64_STANDARD
        .decode(chara_text.as_bytes())
        .map_err(|e| {
            CardLoadError::InvalidJson(format!("{}: Base64 decode failed: {}", path.display(), e))
        })?;

    // Convert to UTF-8 string
    let json_str = String::from_utf8(decoded).map_err(|e| {
        CardLoadError::InvalidJson(format!("{}: UTF-8 decode failed: {}", path.display(), e))
    })?;

    // Parse JSON
    let card: CharacterCard = serde_json::from_str(&json_str)
        .map_err(|e| CardLoadError::InvalidJson(format!("{}: {}", path.display(), e)))?;

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
    use crate::character::{png_text, CharacterData};
    use crate::utils::test_utils::TestEnvVarGuard;
    use crc32fast::Hasher;
    use std::fs;
    use std::io::Write;
    use tempfile::{Builder, NamedTempFile, TempDir};

    struct CardsDirTestEnv {
        _env_guard: TestEnvVarGuard,
        temp_dir: TempDir,
    }

    impl CardsDirTestEnv {
        fn new() -> Self {
            let temp_dir = TempDir::new().expect("failed to create temp cards dir");
            let mut env_guard = TestEnvVarGuard::new();
            env_guard.set_var("CHABEAU_CARDS_DIR", temp_dir.path().as_os_str());

            Self {
                _env_guard: env_guard,
                temp_dir,
            }
        }

        fn path(&self) -> &std::path::Path {
            self.temp_dir.path()
        }
    }

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

    fn create_simple_test_card_json() -> String {
        serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "Simple Test Character",
                "description": "A simple test character for validation",
                "personality": "Friendly and helpful",
                "scenario": "Testing the character card loader",
                "first_mes": "Hello! I'm a test character.",
                "mes_example": "{{user}}: Hi\n{{char}}: Hello there!"
            }
        })
        .to_string()
    }

    fn create_invalid_test_card_json() -> String {
        serde_json::json!({
            "spec": "wrong_spec",
            "spec_version": "2.0",
            "data": {
                "name": "",
                "description": "Invalid card",
                "personality": "Test",
                "scenario": "Test",
                "first_mes": "Test",
                "mes_example": "Test"
            }
        })
        .to_string()
    }

    fn write_json_to_tempfile(contents: &str) -> NamedTempFile {
        let mut temp_file = Builder::new()
            .suffix(".json")
            .tempfile()
            .expect("failed to create temp json file");
        temp_file.write_all(contents.as_bytes()).unwrap();
        temp_file.flush().unwrap();
        temp_file
    }

    const IHDR_DATA: [u8; 13] = [
        0x00, 0x00, 0x00, 0x01, // width = 1
        0x00, 0x00, 0x00, 0x01, // height = 1
        0x08, // bit depth
        0x02, // color type (truecolor)
        0x00, // compression method
        0x00, // filter method
        0x00, // interlace method
    ];

    const IDAT_DATA: [u8; 12] = [
        0x78, 0xDA, 0x63, 0x60, 0x60, 0x60, 0x00, 0x00, 0x00, 0x04, 0x00, 0x01,
    ];

    fn png_chunk(chunk_type: [u8; 4], data: &[u8]) -> Vec<u8> {
        let mut chunk = Vec::with_capacity(12 + data.len());
        chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
        chunk.extend_from_slice(&chunk_type);
        chunk.extend_from_slice(data);
        let mut hasher = Hasher::new();
        hasher.update(&chunk_type);
        hasher.update(data);
        chunk.extend_from_slice(&hasher.finalize().to_be_bytes());
        chunk
    }

    fn assemble_png(chara_payload: Option<&[u8]>) -> Vec<u8> {
        let mut png = Vec::new();
        png.extend_from_slice(&png_text::PNG_SIGNATURE);
        png.extend_from_slice(&png_chunk(*b"IHDR", &IHDR_DATA));
        if let Some(payload) = chara_payload {
            let mut text_data = Vec::with_capacity("chara".len() + 1 + payload.len());
            text_data.extend_from_slice(b"chara");
            text_data.push(0);
            text_data.extend_from_slice(payload);
            png.extend_from_slice(&png_chunk(*b"tEXt", &text_data));
        }
        png.extend_from_slice(&png_chunk(*b"IDAT", &IDAT_DATA));
        png.extend_from_slice(&png_chunk(*b"IEND", &[]));
        png
    }

    fn build_png_with_text(text: &[u8]) -> Vec<u8> {
        assemble_png(Some(text))
    }

    fn build_png_without_text() -> Vec<u8> {
        assemble_png(None)
    }

    #[test]
    fn test_load_valid_json_card() {
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file
            .write_all(create_valid_card_json().as_bytes())
            .unwrap();
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

        let error =
            CardLoadError::ValidationFailed(vec!["Error 1".to_string(), "Error 2".to_string()]);
        let display = format!("{}", error);
        assert!(display.contains("Card validation failed"));
        assert!(display.contains("Error 1"));
        assert!(display.contains("Error 2"));
    }

    #[test]
    fn test_load_hypatia_json() {
        let hypatia_path = "examples/hypatia.json";
        if std::path::Path::new(hypatia_path).exists() {
            let result = load_json_card(hypatia_path);
            assert!(
                result.is_ok(),
                "Failed to load hypatia.json: {:?}",
                result.err()
            );

            let card = result.unwrap();
            assert_eq!(card.spec, "chara_card_v2");
            assert_eq!(card.data.name, "Hypatia of Alexandria");
            assert!(!card.data.description.is_empty());
            assert!(!card.data.first_mes.is_empty());
            assert_eq!(card.data.creator, Some("Chabeau Examples".to_string()));
            assert!(card.data.tags.is_some());
        }
    }

    #[test]
    fn test_load_simple_test_card() {
        let card_json = create_simple_test_card_json();
        let temp_file = write_json_to_tempfile(&card_json);

        let result = load_json_card(temp_file.path());
        assert!(result.is_ok(), "Failed to load simple test card");

        let card = result.unwrap();
        assert_eq!(card.spec, "chara_card_v2");
        assert_eq!(card.data.name, "Simple Test Character");
        assert_eq!(
            card.data.description,
            "A simple test character for validation"
        );
        assert_eq!(card.data.personality, "Friendly and helpful");
    }

    #[test]
    fn test_load_invalid_test_card() {
        let card_json = create_invalid_test_card_json();
        let temp_file = write_json_to_tempfile(&card_json);

        let result = load_json_card(temp_file.path());
        assert!(result.is_err(), "Expected error loading invalid test card");

        match result.unwrap_err() {
            CardLoadError::ValidationFailed(errors) => {
                assert!(!errors.is_empty());
                assert!(errors
                    .iter()
                    .any(|e| e.contains("spec") || e.contains("name")));
            }
            _ => panic!("Expected ValidationFailed error"),
        }
    }

    // PNG loading tests

    #[test]
    fn test_load_png_card_with_metadata() {
        // Create character card JSON
        let card_json = create_valid_card_json();
        let encoded = base64::prelude::BASE64_STANDARD.encode(card_json.as_bytes());
        let png_bytes = build_png_with_text(encoded.as_bytes());

        let temp_file = NamedTempFile::new().unwrap();
        temp_file.as_file().write_all(&png_bytes).unwrap();

        // Now try to load the PNG card
        let result = load_png_card(temp_file.path());
        assert!(
            result.is_ok(),
            "Failed to load PNG card: {:?}",
            result.err()
        );

        let card = result.unwrap();
        assert_eq!(card.spec, "chara_card_v2");
        assert_eq!(card.data.name, "Test Character");
        assert_eq!(card.data.description, "A test character for unit tests");
    }

    #[test]
    fn test_load_png_card_without_metadata() {
        // Create a PNG without chara metadata
        let png_bytes = build_png_without_text();
        let temp_file = NamedTempFile::new().unwrap();
        temp_file.as_file().write_all(&png_bytes).unwrap();

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
        let png_bytes = build_png_with_text(b"not-valid-base64!!!");
        let temp_file = NamedTempFile::new().unwrap();
        temp_file.as_file().write_all(&png_bytes).unwrap();

        let result = load_png_card(temp_file.path());
        assert!(result.is_err());

        match result.unwrap_err() {
            CardLoadError::InvalidJson(msg) => {
                assert!(
                    msg.contains("Base64 decode failed")
                        || msg.contains("UTF-8 decode failed")
                        || msg.contains("expected")
                );
            }
            _ => panic!("Expected InvalidJson error"),
        }
    }

    #[test]
    fn test_load_png_card_invalid_json() {
        // Encode invalid JSON
        let invalid_json = "{ this is not valid json }";
        let encoded = base64::prelude::BASE64_STANDARD.encode(invalid_json.as_bytes());
        let png_bytes = build_png_with_text(encoded.as_bytes());

        let temp_file = NamedTempFile::new().unwrap();
        temp_file.as_file().write_all(&png_bytes).unwrap();

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
    fn test_load_hypatia_png() {
        // Test with PNG file if it exists
        let hypatia_path = "examples/hypatia.png";
        if std::path::Path::new(hypatia_path).exists() {
            let result = load_png_card(hypatia_path);
            assert!(
                result.is_ok(),
                "Failed to load hypatia.png: {:?}",
                result.err()
            );

            let card = result.unwrap();
            assert_eq!(card.spec, "chara_card_v2");
            assert_eq!(card.data.name, "Hypatia of Alexandria");
            assert!(!card.data.description.is_empty());
            assert!(!card.data.first_mes.is_empty());
        }
    }

    #[test]
    fn test_load_spec_v2_png_cards() {
        // Test with the spec v2 PNG files if it exists
        let test_cards = ["examples/hypatia.png"];

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
        // If both exist, they should have the same data
        let json_path = "examples/hypatia.json";
        let png_path = "examples/hypatia.png";

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
        assert_eq!(
            format!("{}", error),
            "Invalid PNG: test.png: Invalid format"
        );

        let error = CardLoadError::MissingMetadata("test.png: No chara chunk".to_string());
        assert_eq!(
            format!("{}", error),
            "Missing metadata: test.png: No chara chunk"
        );
    }

    // Card discovery tests

    #[test]
    fn test_get_cards_dir() {
        let mut env_guard = TestEnvVarGuard::new();
        env_guard.remove_var("CHABEAU_CARDS_DIR");

        let cards_dir = get_cards_dir();
        assert!(cards_dir.to_string_lossy().contains("chabeau"));
        assert!(cards_dir.to_string_lossy().contains("cards"));
    }

    #[test]
    fn test_get_cards_dir_env_override() {
        let mut env_guard = TestEnvVarGuard::new();
        let temp_dir = tempfile::tempdir().unwrap();

        env_guard.set_var("CHABEAU_CARDS_DIR", temp_dir.path().as_os_str());

        let cards_dir = get_cards_dir();
        assert_eq!(cards_dir, temp_dir.path());
    }

    #[test]
    fn test_list_available_cards_empty_directory() {
        // Ensure the cards directory is isolated per test
        let _cards_env = CardsDirTestEnv::new();

        let result = list_available_cards();
        assert!(result.is_ok());

        let cards = result.unwrap();
        assert!(cards.is_empty());
    }

    #[test]
    fn test_list_available_cards_with_test_cards() {
        let env = CardsDirTestEnv::new();
        let card_path = env.path().join("sample_card.json");

        let card_json = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "Temp Tester",
                "description": "Temporary card for list tests",
                "personality": "Curious",
                "scenario": "Running unit tests",
                "first_mes": "Hello from a temp card!",
                "mes_example": "{{user}}: Hi\n{{char}}: Hello!"
            }
        });

        fs::write(
            &card_path,
            serde_json::to_string_pretty(&card_json).unwrap(),
        )
        .unwrap();

        let result = list_available_cards();
        assert!(result.is_ok());

        let cards = result.unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].0, "Temp Tester");
        assert_eq!(cards[0].1, card_path);
    }

    #[test]
    fn test_load_card_json() {
        let card_json = create_simple_test_card_json();
        let temp_file = write_json_to_tempfile(&card_json);

        let result = load_card(temp_file.path());
        assert!(result.is_ok());

        let card = result.unwrap();
        assert_eq!(card.data.name, "Simple Test Character");
    }

    #[test]
    fn test_load_card_png() {
        // Test loading a PNG card through the load_card function
        let test_path = "examples/hypatia.png";
        if std::path::Path::new(test_path).exists() {
            let result = load_card(test_path);
            assert!(result.is_ok());

            let card = result.unwrap();
            assert_eq!(card.data.name, "Hypatia of Alexandria");
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
        })
        .to_string();
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
                let extension = path
                    .extension()
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
                let extension = path
                    .extension()
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
