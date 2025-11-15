use std::fs;
use std::path::Path;

use crate::character::loader::{get_cards_dir, load_card};

/// Errors that can occur when importing character cards.
#[derive(Debug)]
pub enum ImportError {
    /// Character card validation failed (invalid card structure or required fields missing).
    ValidationFailed(String),

    /// Destination file already exists and force overwrite was not requested.
    AlreadyExists(String),

    /// I/O error occurred while copying or writing the character card file.
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

/// Import a character card file into the configured cards directory.
///
/// The cards directory defaults to the config location returned by [`get_cards_dir`] and may
/// be overridden by setting the `CHABEAU_CARDS_DIR` environment variable. Tests rely on that
/// override so they can exercise the real import workflow without touching the user's files.
/// The actual import work happens in [`import_card_into`], which keeps filesystem details in
/// one place while leaving this public API focused on its high-level behavior.
///
/// # Arguments
/// * `source_path` - Path to the source card file (JSON or PNG)
/// * `force_overwrite` - If true, overwrite existing files without prompting
///
/// # Returns
/// * `Ok(String)` - Success message with character name and destination path
/// * `Err(ImportError)` - Error if validation, file operations, or conflicts occur
pub fn import_card<P: AsRef<Path>>(
    source_path: P,
    force_overwrite: bool,
) -> Result<String, ImportError> {
    let cards_dir = get_cards_dir();
    import_card_into(source_path.as_ref(), &cards_dir, force_overwrite)
}

fn import_card_into(
    source_path: &Path,
    dest_dir: &Path,
    force_overwrite: bool,
) -> Result<String, ImportError> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    use crate::utils::test_utils::TestEnvVarGuard;

    fn with_cards_dir<T, F>(cards_dir: &Path, f: F) -> T
    where
        F: FnOnce() -> T,
    {
        let mut env_guard = TestEnvVarGuard::new();
        env_guard.set_var("CHABEAU_CARDS_DIR", cards_dir.as_os_str());
        let result = f();
        drop(env_guard);
        result
    }

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
        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");

        let result = with_cards_dir(&cards_dir, || {
            // Create an invalid card file
            let mut temp_file = NamedTempFile::new().unwrap();
            temp_file.write_all(b"{ invalid json }").unwrap();
            temp_file.flush().unwrap();

            import_card(temp_file.path(), false)
        });
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
        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        let result = with_cards_dir(&cards_dir, || {
            import_card("/nonexistent/path/to/card.json", false)
        });
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
        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        let mut temp_file = NamedTempFile::with_suffix(".txt").unwrap();
        temp_file
            .write_all(create_valid_card_json().as_bytes())
            .unwrap();
        temp_file.flush().unwrap();

        let result = with_cards_dir(&cards_dir, || import_card(temp_file.path(), false));
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
        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        // Verify directory doesn't exist yet
        assert!(!cards_dir.exists());

        let temp_file = create_valid_card_file();

        // The import will create the directory if needed
        let result = with_cards_dir(&cards_dir, || import_card(temp_file.path(), false));
        assert!(result.is_ok());

        // Verify the cards directory was created
        assert!(cards_dir.exists());
    }

    #[test]
    fn test_import_card_validation_before_copy() {
        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        // Verify that validation happens before any file operations
        // Create an invalid card
        let mut temp_file = NamedTempFile::with_suffix(".json").unwrap();
        temp_file.write_all(b"{ invalid json }").unwrap();
        temp_file.flush().unwrap();

        let result = with_cards_dir(&cards_dir, || import_card(temp_file.path(), false));
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
        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        // Verify that the imported card keeps its original filename
        let temp_file = create_valid_card_file();
        let original_filename = temp_file.path().file_name().unwrap().to_owned();

        let result = with_cards_dir(&cards_dir, || import_card(temp_file.path(), false))
            .expect("import should succeed");
        assert!(result.contains("Test Import Character"));

        let imported_path = cards_dir.join(&original_filename);
        assert!(imported_path.exists());
    }

    #[test]
    fn test_import_card_success_message_format() {
        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        // Test that success messages contain the character name
        let temp_file = create_valid_card_file();

        let message = with_cards_dir(&cards_dir, || import_card(temp_file.path(), false))
            .expect("import should succeed");
        assert!(message.contains("✅"));
        assert!(message.contains("Test Import Character"));
    }

    // Integration tests that test actual import scenarios
    // These tests use temporary directories to avoid affecting production

    #[test]
    fn test_import_and_verify_in_temp_dir() {
        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        // Create a temporary source file
        let temp_file = create_valid_card_file();

        // Import the card to temp directory
        let result = with_cards_dir(&cards_dir, || import_card(temp_file.path(), false));

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
        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        // Create a temporary source file
        let temp_file = create_valid_card_file();

        let (result1, result2, result3) = with_cards_dir(&cards_dir, || {
            let first = import_card(temp_file.path(), false);
            let second = import_card(temp_file.path(), false);
            let third = import_card(temp_file.path(), true);
            (first, second, third)
        });
        assert!(result1.is_ok());

        assert!(result2.is_err());
        match result2.unwrap_err() {
            ImportError::AlreadyExists(_) => {
                // Expected behavior
            }
            other => panic!("Expected AlreadyExists error, got {:?}", other),
        }

        assert!(result3.is_ok());
        assert!(result3.unwrap().contains("✅"));

        // Temp directory will be automatically cleaned up when dropped
    }

    #[test]
    fn test_import_with_force_flag() {
        let temp_dir = TempDir::new().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        // Create a temporary source file
        let temp_file = create_valid_card_file();

        // Import with force should always succeed (even if file exists)
        let result = with_cards_dir(&cards_dir, || import_card(temp_file.path(), true));
        assert!(result.is_ok());

        let message = result.unwrap();
        assert!(message.contains("✅"));
        assert!(message.contains("Test Import Character"));

        // Verify the file was actually copied
        let imported_path = cards_dir.join(temp_file.path().file_name().unwrap());
        assert!(imported_path.exists());

        // Import again with force - should still succeed
        let result2 = with_cards_dir(&cards_dir, || import_card(temp_file.path(), true));
        assert!(result2.is_ok());

        // Temp directory will be automatically cleaned up when dropped
    }
}
