//! Character card loading and caching service.
//!
//! This module provides the [`CharacterService`] which manages character cards,
//! including loading from disk, caching for performance, and resolving characters
//! by name or path. The service invalidates cached entries when the underlying
//! card directory changes to ensure fresh data.
//!
//! Character cards define AI personas and greetings for chat sessions.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::character::cache::{CachedCardMetadata, CardCache};
use crate::character::loader::{self, CardLoadError};
use crate::character::CharacterCard;
use crate::core::config::data::Config;

/// Errors that can occur during character card operations.
#[derive(Debug)]
pub enum CharacterServiceError {
    /// Cache initialization or operation failed.
    Cache(String),

    /// Failed to load or parse a character card file.
    Load(CardLoadError),

    /// I/O error while accessing the character card directory or files.
    Io(std::io::Error),

    /// Character with the specified name was not found in the cards directory.
    NotFound(String),
}

impl std::fmt::Display for CharacterServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CharacterServiceError::Cache(msg) => write!(f, "Character cache error: {msg}"),
            CharacterServiceError::Load(err) => write!(f, "{err}"),
            CharacterServiceError::Io(err) => write!(f, "I/O error: {err}"),
            CharacterServiceError::NotFound(name) => {
                write!(f, "Character '{}' not found in cards directory", name)
            }
        }
    }
}

impl std::error::Error for CharacterServiceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CharacterServiceError::Cache(_) => None,
            CharacterServiceError::Load(err) => Some(err),
            CharacterServiceError::Io(err) => Some(err),
            CharacterServiceError::NotFound(_) => None,
        }
    }
}

#[derive(Clone)]
struct CachedCardEntry {
    card: CharacterCard,
    modified: Option<SystemTime>,
}

/// Service for loading and caching character cards.
///
/// This service manages character card loading from disk with automatic caching
/// based on file modification times. It invalidates cached entries when the
/// cards directory changes (based on cache key) and provides fuzzy name matching
/// for resolving character cards.
///
/// Character cards can be stored as JSON files or PNG images with embedded
/// metadata. The service supports both direct file path lookups and name-based
/// resolution with normalization (case-insensitive, space-to-underscore conversion).
pub struct CharacterService {
    cache: CardCache,
    cards: HashMap<PathBuf, CachedCardEntry>,
    last_cache_key: Option<String>,
}

impl CharacterService {
    /// Creates a new character service with an empty cache.
    ///
    /// The service will lazily initialize the cache on first use and track
    /// file modification times to automatically reload changed cards.
    pub fn new() -> Self {
        Self {
            cache: CardCache::new(),
            cards: HashMap::new(),
            last_cache_key: None,
        }
    }

    /// Returns metadata for all character cards in the cards directory.
    ///
    /// This method scans the cards directory and returns lightweight metadata
    /// (name, description, tags) without loading full card contents. The cache
    /// is invalidated if the directory has changed since the last call.
    ///
    /// # Errors
    ///
    /// Returns an error if the cache initialization or metadata retrieval fails.
    pub fn list_metadata(&mut self) -> Result<Vec<CachedCardMetadata>, CharacterServiceError> {
        let metadata = self
            .cache
            .get_all_metadata()
            .map_err(|err| CharacterServiceError::Cache(err.to_string()))?;

        let cache_key = self.cache.cache_key().map(|k| k.to_string());
        if cache_key != self.last_cache_key {
            self.cards.clear();
            self.last_cache_key = cache_key;
        }

        Ok(metadata)
    }

    /// Returns metadata for all character cards along with their file paths.
    ///
    /// This is similar to [`list_metadata`](Self::list_metadata) but includes
    /// the full path to each card file, useful for operations that need to
    /// access the original files.
    ///
    /// # Errors
    ///
    /// Returns an error if the cache initialization or metadata retrieval fails.
    pub fn list_metadata_with_paths(
        &mut self,
    ) -> Result<Vec<(CachedCardMetadata, PathBuf)>, CharacterServiceError> {
        let metadata = self.list_metadata()?;
        let mut result = Vec::with_capacity(metadata.len());
        for entry in metadata {
            if let Some(path) = self.cache.path_for(&entry.name) {
                result.push((entry, path.clone()));
            }
        }
        Ok(result)
    }

    /// Resolves a character card by name or file path.
    ///
    /// This method attempts to load a character card using flexible input:
    /// - If `input` is a valid file path, loads from that path directly
    /// - Otherwise, treats `input` as a character name and performs fuzzy lookup
    ///
    /// The method caches loaded cards by path and reloads only if the file
    /// modification time has changed.
    ///
    /// # Arguments
    ///
    /// * `input` - Either a file path to a character card or a character name
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The file path does not exist or cannot be read
    /// - The character name cannot be found in the cards directory
    /// - The card file is malformed or fails validation
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use chabeau::character::service::CharacterService;
    /// let mut service = CharacterService::new();
    ///
    /// // Load by file path
    /// let card = service.resolve("/path/to/character.json")?;
    ///
    /// // Load by name
    /// let card = service.resolve("Alice")?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn resolve(&mut self, input: &str) -> Result<CharacterCard, CharacterServiceError> {
        let path = Path::new(input);
        if path.is_file() {
            return self.load_from_path(path.to_path_buf());
        }

        self.resolve_by_name(input)
    }

    /// Resolves a character card by name only (no path lookup).
    ///
    /// This method performs fuzzy matching on character names, trying:
    /// 1. Exact name match with `.json` and `.png` extensions
    /// 2. Normalized name (lowercase, spaces replaced with underscores)
    /// 3. Case-insensitive search through cached metadata
    ///
    /// # Arguments
    ///
    /// * `name` - The character name to search for
    ///
    /// # Errors
    ///
    /// Returns [`CharacterServiceError::NotFound`] if no character with a
    /// matching name can be found in the cards directory.
    pub fn resolve_by_name(&mut self, name: &str) -> Result<CharacterCard, CharacterServiceError> {
        if let Some(path) = self.try_find_card_path(name)? {
            return self.load_from_path(path);
        }

        Err(CharacterServiceError::NotFound(name.to_string()))
    }

    /// Loads the default character for a specific provider and model.
    ///
    /// This method consults the user configuration to find a default character
    /// configured for the given provider/model combination. If a default is
    /// configured, it resolves and loads that character card.
    ///
    /// # Arguments
    ///
    /// * `provider` - Provider ID (e.g., "openai", "anthropic")
    /// * `model` - Model identifier (e.g., "gpt-4", "claude-3-opus")
    /// * `config` - User configuration containing default character mappings
    ///
    /// # Returns
    ///
    /// Returns `Ok(Some((name, card)))` if a default character is configured
    /// and successfully loaded, or `Ok(None)` if no default is configured.
    ///
    /// # Errors
    ///
    /// Returns an error if the configured default character cannot be found
    /// or loaded.
    pub fn load_default_for_session(
        &mut self,
        provider: &str,
        model: &str,
        config: &Config,
    ) -> Result<Option<(String, CharacterCard)>, CharacterServiceError> {
        if let Some(default_character) = config.get_default_character(provider, model) {
            let name = default_character.to_string();
            self.resolve_by_name(default_character)
                .map(|card| Some((name, card)))
        } else {
            Ok(None)
        }
    }

    fn load_from_path(&mut self, path: PathBuf) -> Result<CharacterCard, CharacterServiceError> {
        let modified = match std::fs::metadata(&path) {
            Ok(metadata) => metadata.modified().ok(),
            Err(err) if err.kind() == ErrorKind::NotFound => {
                return Err(CharacterServiceError::NotFound(path.display().to_string()))
            }
            Err(err) => return Err(CharacterServiceError::Io(err)),
        };

        if let Some(entry) = self.cards.get(path.as_path()) {
            if entry.modified == modified {
                return Ok(entry.card.clone());
            }
        }

        let card = loader::load_card(&path).map_err(CharacterServiceError::Load)?;
        let card_clone = card.clone();
        self.cards.insert(path, CachedCardEntry { card, modified });
        Ok(card_clone)
    }

    fn try_find_card_path(&mut self, name: &str) -> Result<Option<PathBuf>, CharacterServiceError> {
        let cards_dir = loader::get_cards_dir();

        let normalized_lookup = Self::normalize_lookup_key(name);

        for ext in ["json", "png"] {
            let candidate = cards_dir.join(format!("{name}.{ext}"));
            if candidate.is_file() {
                return Ok(Some(candidate));
            }

            if normalized_lookup != name {
                let normalized_candidate = cards_dir.join(format!("{normalized_lookup}.{ext}"));
                if normalized_candidate.is_file() {
                    return Ok(Some(normalized_candidate));
                }
            }
        }

        let _ = self.list_metadata()?;

        if let Some(path) = self.cache.path_for(name).cloned() {
            return Ok(Some(path));
        }

        let lower_lookup = name.to_lowercase();
        if let Some(path) = self
            .cache
            .iter_paths()
            .find(|(cached_name, _)| {
                let cached_lower = cached_name.to_lowercase();
                cached_lower == lower_lookup
                    || Self::normalize_lookup_key(cached_name) == normalized_lookup
            })
            .map(|(_, path)| path.clone())
        {
            return Ok(Some(path));
        }

        Ok(None)
    }

    fn normalize_lookup_key(name: &str) -> String {
        name.trim().to_lowercase().replace(' ', "_")
    }
}

impl Default for CharacterService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::test_helpers::helpers::create_temp_cards_dir_with_cards;
    use crate::utils::test_utils::TestEnvVarGuard;
    use std::fs;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn resolves_updates_after_file_change() {
        let (_dir, cards_dir) = create_temp_cards_dir_with_cards(&[("Alice", "Hello there!")]);
        let mut env_guard = TestEnvVarGuard::new();
        env_guard.set_var("CHABEAU_CARDS_DIR", &cards_dir);

        let mut service = CharacterService::new();

        // Initial load
        let first = service.resolve_by_name("Alice").expect("initial card load");
        assert_eq!(first.data.first_mes, "Hello there!");

        // Ensure cache hit returns same data before modification
        let second = service.resolve_by_name("Alice").expect("second card load");
        assert_eq!(second.data.first_mes, "Hello there!");

        // Modify the card on disk and ensure cache miss reloads updated data
        thread::sleep(Duration::from_millis(1100));
        let mut updated = first.clone();
        updated.data.first_mes = "Updated greeting".to_string();
        let card_path = cards_dir.join("alice.json");
        fs::write(&card_path, serde_json::to_string(&updated).unwrap()).unwrap();

        service.list_metadata().expect("metadata reload");

        let third = service
            .resolve_by_name("Alice")
            .expect("card after modification");
        assert_eq!(third.data.first_mes, "Updated greeting");

        drop(env_guard);
    }
}
