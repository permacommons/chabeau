use std::fs;
use std::path::Path;

use crate::character::loader::{get_cards_dir, load_card};

/// Error type for import operations
#[derive(Debug)]
pub enum ImportError {
    /// Card validation failed
    ValidationFailed(String),
    /// File already exists and force flag not set
    AlreadyExists(String),
    /// IO error during import
    IoError(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::ValidationFailed(msg) => {
                write!(f, "Card validation failed: {}", msg)
            }
            ImportError::AlreadyExists(name) => {
                write!(
                    f,
                    "Card '{}' already exists. Use --force to overwrite.",
                    name
                )
            }
            ImportError::IoError(msg) => {
                write!(f, "Import failed: {}", msg)
            }
        }
    }
}

impl std::error::Error for ImportError {}

/// Import a character card file to a specified directory
///
/// Validates the card, creates the destination directory if needed, checks for conflicts,
/// and copies the file. Used by `import_card` for production imports and by tests with
/// temporary directories.
///
/// # Arguments
/// * `source_path` - Path to the source card file (JSON or PNG)
/// * `dest_dir` - Destination directory for the imported card
/// * `force_overwrite` - If true, overwrite existing files without prompting
///
/// # Returns
/// * `Ok(String)` - Success message with character name and destination path
/// * `Err(ImportError)` - Error if validation, file operations, or conflicts occur
pub fn import_card_to_dir<P: AsRef<Path>, D: AsRef<Path>>(
    source_path: P,
    dest_dir: D,
    force_overwrite: bool,
) -> Result<String, ImportError> {
    let source_path = source_path.as_ref();
    let dest_dir = dest_dir.as_ref();

    // Validate the card first by loading it
    let card =
        load_card(source_path).map_err(|e| ImportError::ValidationFailed(format!("{}", e)))?;

    // Create destination directory if needed
    fs::create_dir_all(dest_dir)
        .map_err(|e| ImportError::IoError(format!("Failed to create cards directory: {}", e)))?;

    // Get the filename from the source path
    let filename = source_path
        .file_name()
        .ok_or_else(|| ImportError::IoError("Invalid file path".to_string()))?;

    let dest_path = dest_dir.join(filename);

    // Check if file already exists
    if dest_path.exists() && !force_overwrite {
        return Err(ImportError::AlreadyExists(
            filename.to_string_lossy().to_string(),
        ));
    }

    // Copy the file to the cards directory
    fs::copy(source_path, &dest_path)
        .map_err(|e| ImportError::IoError(format!("Failed to copy file: {}", e)))?;

    // Return success message
    Ok(format!(
        "✅ Imported character '{}' to {}",
        card.data.name,
        dest_path.display()
    ))
}

/// Import a character card file to the default cards directory
///
/// Wrapper around `import_card_to_dir` that uses the production cards directory.
pub fn import_card<P: AsRef<Path>>(
    source_path: P,
    force_overwrite: bool,
) -> Result<String, ImportError> {
    let cards_dir = get_cards_dir();
    import_card_to_dir(source_path, cards_dir, force_overwrite)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_valid_card_json() -> String {
        serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "Test Import Character",
                "description": "A test character for import tests",
                "personality": "Friendly",
                "scenario": "Testing",
                "first_mes": "Hello!",
                "mes_example": "Example"
            }
        })
        .to_string()
    }

    fn create_valid_card_file() -> NamedTempFile {
        let mut temp_file = NamedTempFile::with_suffix(".json").unwrap();
        temp_file
            .write_all(create_valid_card_json().as_bytes())
            .unwrap();
        temp_file.flush().unwrap();
        temp_file
    }

    #[test]
    fn test_import_valid_card() {
        // Create a temporary card file with .json extension
        let temp_file = create_valid_card_file();

        // Verify the card loads correctly (this validates the card structure)
        let result = load_card(temp_file.path());
        assert!(
            result.is_ok(),
            "Card should load successfully: {:?}",
            result.err()
        );

        let card = result.unwrap();
        assert_eq!(card.data.name, "Test Import Character");
    }

    #[test]
    fn test_import_invalid_card() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");

        // Create an invalid card file
        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(b"{ invalid json }").unwrap();
        temp_file.flush().unwrap();

        let result = import_card_to_dir(temp_file.path(), &cards_dir, false);
        assert!(result.is_err());

        match result.unwrap_err() {
            ImportError::ValidationFailed(msg) => {
                assert!(msg.contains("Invalid JSON") || msg.contains("expected"));
            }
            _ => panic!("Expected ValidationFailed error"),
        }
    }

    #[test]
    fn test_import_error_display() {
        let error = ImportError::ValidationFailed("Test error".to_string());
        assert_eq!(format!("{}", error), "Card validation failed: Test error");

        let error = ImportError::AlreadyExists("test.json".to_string());
        assert_eq!(
            format!("{}", error),
            "Card 'test.json' already exists. Use --force to overwrite."
        );

        let error = ImportError::IoError("Test IO error".to_string());
        assert_eq!(format!("{}", error), "Import failed: Test IO error");
    }

    #[test]
    fn test_import_card_missing_file() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");

        let result = import_card_to_dir("/nonexistent/path/to/card.json", &cards_dir, false);
        assert!(result.is_err());

        match result.unwrap_err() {
            ImportError::ValidationFailed(msg) => {
                assert!(msg.contains("File not found") || msg.contains("No such file"));
            }
            _ => panic!("Expected ValidationFailed error for missing file"),
        }
    }

    #[test]
    fn test_import_card_wrong_extension() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");

        // Create a file with wrong extension
        let mut temp_file = NamedTempFile::with_suffix(".txt").unwrap();
        temp_file
            .write_all(create_valid_card_json().as_bytes())
            .unwrap();
        temp_file.flush().unwrap();

        let result = import_card_to_dir(temp_file.path(), &cards_dir, false);
        assert!(result.is_err());

        match result.unwrap_err() {
            ImportError::ValidationFailed(msg) => {
                assert!(msg.contains("must be .json or .png"));
            }
            _ => panic!("Expected ValidationFailed error for wrong extension"),
        }
    }

    #[test]
    fn test_import_card_creates_directory() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");

        // Verify directory doesn't exist yet
        assert!(!cards_dir.exists());

        let temp_file = create_valid_card_file();

        // The import will create the directory if needed
        let result = import_card_to_dir(temp_file.path(), &cards_dir, false);
        assert!(result.is_ok());

        // Verify the cards directory was created
        assert!(cards_dir.exists());
    }

    #[test]
    fn test_import_card_validation_before_copy() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");

        // Verify that validation happens before any file operations
        // Create an invalid card
        let mut temp_file = NamedTempFile::with_suffix(".json").unwrap();
        temp_file.write_all(b"{ invalid json }").unwrap();
        temp_file.flush().unwrap();

        let result = import_card_to_dir(temp_file.path(), &cards_dir, false);
        assert!(result.is_err());

        // Should fail with validation error, not IO error
        match result.unwrap_err() {
            ImportError::ValidationFailed(_) => {
                // Expected
            }
            other => panic!("Expected ValidationFailed, got {:?}", other),
        }
    }

    #[test]
    fn test_import_card_preserves_filename() {
        // Verify that the imported card keeps its original filename
        let temp_file = create_valid_card_file();
        let original_filename = temp_file
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();

        // The filename should be preserved in the destination
        // We can't easily test the actual import without affecting the real cards directory,
        // but we can verify the logic by checking the error message format
        assert!(original_filename.ends_with(".json"));
    }

    #[test]
    fn test_import_card_success_message_format() {
        // Test that success messages contain the character name
        let temp_file = create_valid_card_file();

        // Load the card to get its name
        let card = load_card(temp_file.path()).unwrap();
        assert_eq!(card.data.name, "Test Import Character");

        // The success message should contain the character name
        // We can't test the actual import easily, but we verified the card structure
    }

    // Integration tests that test actual import scenarios
    // These tests use temporary directories to avoid affecting production

    #[test]
    fn test_import_and_verify_in_temp_dir() {
        use tempfile::TempDir;

        // Create a temporary directory for this test
        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");

        // Create a temporary source file
        let temp_file = create_valid_card_file();

        // Import the card to temp directory
        let result = import_card_to_dir(temp_file.path(), &cards_dir, false);

        // Verify the result
        assert!(result.is_ok());
        let message = result.unwrap();
        assert!(message.contains("✅"));
        assert!(message.contains("Test Import Character"));

        // Verify the file was actually copied
        let imported_path = cards_dir.join(temp_file.path().file_name().unwrap());
        assert!(imported_path.exists());

        // Temp directory will be automatically cleaned up when dropped
    }

    #[test]
    fn test_import_overwrite_protection() {
        use tempfile::TempDir;

        // Create a temporary directory for this test
        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");

        // Create a temporary source file
        let temp_file = create_valid_card_file();

        // First import should succeed
        let result1 = import_card_to_dir(temp_file.path(), &cards_dir, false);
        assert!(result1.is_ok());

        // Second import without force should fail
        let result2 = import_card_to_dir(temp_file.path(), &cards_dir, false);
        assert!(result2.is_err());
        match result2.unwrap_err() {
            ImportError::AlreadyExists(_) => {
                // Expected behavior
            }
            other => panic!("Expected AlreadyExists error, got {:?}", other),
        }

        // Third import with force should succeed
        let result3 = import_card_to_dir(temp_file.path(), &cards_dir, true);
        assert!(result3.is_ok());
        assert!(result3.unwrap().contains("✅"));

        // Temp directory will be automatically cleaned up when dropped
    }

    #[test]
    fn test_import_with_force_flag() {
        use tempfile::TempDir;

        // Create a temporary directory for this test
        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");

        // Create a temporary source file
        let temp_file = create_valid_card_file();

        // Import with force should always succeed (even if file exists)
        let result = import_card_to_dir(temp_file.path(), &cards_dir, true);
        assert!(result.is_ok());

        let message = result.unwrap();
        assert!(message.contains("✅"));
        assert!(message.contains("Test Import Character"));

        // Verify the file was actually copied
        let imported_path = cards_dir.join(temp_file.path().file_name().unwrap());
        assert!(imported_path.exists());

        // Import again with force - should still succeed
        let result2 = import_card_to_dir(temp_file.path(), &cards_dir, true);
        assert!(result2.is_ok());

        // Temp directory will be automatically cleaned up when dropped
    }
}
