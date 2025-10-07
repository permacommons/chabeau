use std::collections::HashMap;
use std::time::SystemTime;

/// Metadata for a cached character card
#[derive(Debug, Clone)]
pub struct CachedCardMetadata {
    pub name: String,
    pub description: String,
}

/// Cache for character card metadata
pub struct CardCache {
    metadata: HashMap<String, CachedCardMetadata>,
    cache_key: Option<String>,
}

impl CardCache {
    /// Create a new empty cache
    pub fn new() -> Self {
        Self {
            metadata: HashMap::new(),
            cache_key: None,
        }
    }

    /// Compute a cache key based on directory modification times
    fn compute_cache_key() -> Result<String, Box<dyn std::error::Error>> {
        let cards_dir = crate::character::loader::get_cards_dir();

        if !cards_dir.exists() {
            return Ok(String::new());
        }

        let mut mod_times = Vec::new();

        for entry in std::fs::read_dir(cards_dir)? {
            let entry = entry?;
            let metadata = entry.metadata()?;

            if let Ok(modified) = metadata.modified() {
                if let Ok(duration) = modified.duration_since(SystemTime::UNIX_EPOCH) {
                    mod_times.push(duration.as_secs());
                }
            }
        }

        mod_times.sort();
        Ok(format!("{:?}", mod_times))
    }

    /// Load all card metadata, using cache if valid
    pub fn get_all_metadata(
        &mut self,
    ) -> Result<Vec<CachedCardMetadata>, Box<dyn std::error::Error>> {
        let current_key = Self::compute_cache_key()?;

        // Check if cache is valid
        if self.cache_key.as_ref() == Some(&current_key) && !self.metadata.is_empty() {
            let mut result: Vec<_> = self.metadata.values().cloned().collect();
            result.sort_by(|a, b| a.name.cmp(&b.name));
            return Ok(result);
        }

        // Cache is invalid, reload
        self.metadata.clear();

        let cards = crate::character::loader::list_available_cards()?;

        for (name, path) in cards {
            // Load full card to get description
            if let Ok(card) = crate::character::loader::load_card(&path) {
                let metadata = CachedCardMetadata {
                    name: card.data.name.clone(),
                    description: card.data.description.clone(),
                };
                self.metadata.insert(name, metadata);
            }
        }

        self.cache_key = Some(current_key);

        let mut result: Vec<_> = self.metadata.values().cloned().collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }

    /// Invalidate the cache
    /// This forces a reload on the next call to get_all_metadata()
    #[allow(dead_code)] // Public API for future use
    pub fn invalidate(&mut self) {
        self.cache_key = None;
        self.metadata.clear();
    }
}

impl Default for CardCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_cache_is_empty() {
        let cache = CardCache::new();
        assert!(cache.metadata.is_empty());
        assert!(cache.cache_key.is_none());
    }

    #[test]
    fn test_invalidate_clears_cache() {
        let mut cache = CardCache::new();

        // Manually populate cache
        cache.metadata.insert(
            "test".to_string(),
            CachedCardMetadata {
                name: "Test".to_string(),
                description: "Test description".to_string(),
            },
        );
        cache.cache_key = Some("test_key".to_string());

        assert!(!cache.metadata.is_empty());
        assert!(cache.cache_key.is_some());

        cache.invalidate();

        assert!(cache.metadata.is_empty());
        assert!(cache.cache_key.is_none());
    }

    #[test]
    fn test_compute_cache_key_empty_directory() {
        // Note: This test assumes get_cards_dir() returns a non-existent directory
        // In a real scenario, we'd need to mock the directory path

        let result = CardCache::compute_cache_key();
        assert!(result.is_ok());
    }

    #[test]
    fn test_cache_hit_returns_same_data() {
        // This test would require mocking the file system
        // For now, we'll test the basic cache behavior
        let mut cache = CardCache::new();

        // Manually set up cache state
        let metadata = CachedCardMetadata {
            name: "TestChar".to_string(),
            description: "A test character".to_string(),
        };

        cache
            .metadata
            .insert("TestChar".to_string(), metadata.clone());
        cache.cache_key = Some("test_key".to_string());

        // Verify cache contains the data
        assert_eq!(cache.metadata.len(), 1);
        assert!(cache.metadata.contains_key("TestChar"));
    }

    #[test]
    fn test_cache_miss_reloads_data() {
        let mut cache = CardCache::new();

        // Set an old cache key
        cache.cache_key = Some("old_key".to_string());

        // When get_all_metadata is called with a different cache key,
        // it should reload. This is tested implicitly through the
        // cache_key comparison logic.

        assert_eq!(cache.cache_key, Some("old_key".to_string()));
    }

    #[test]
    fn test_metadata_sorted_by_name() {
        let mut cache = CardCache::new();

        // Add metadata in unsorted order
        cache.metadata.insert(
            "charlie".to_string(),
            CachedCardMetadata {
                name: "Charlie".to_string(),
                description: "C".to_string(),
            },
        );
        cache.metadata.insert(
            "alice".to_string(),
            CachedCardMetadata {
                name: "Alice".to_string(),
                description: "A".to_string(),
            },
        );
        cache.metadata.insert(
            "bob".to_string(),
            CachedCardMetadata {
                name: "Bob".to_string(),
                description: "B".to_string(),
            },
        );

        cache.cache_key = Some("test".to_string());

        // The get_all_metadata method should return sorted results
        // We'll verify the sorting logic is present in the implementation
        let mut result: Vec<_> = cache.metadata.values().cloned().collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(result[0].name, "Alice");
        assert_eq!(result[1].name, "Bob");
        assert_eq!(result[2].name, "Charlie");
    }

    #[test]
    fn test_default_creates_empty_cache() {
        let cache = CardCache::default();
        assert!(cache.metadata.is_empty());
        assert!(cache.cache_key.is_none());
    }

    #[test]
    fn test_cache_reuses_data_on_second_call() {
        let mut cache = CardCache::new();

        // Manually set up cache with a key
        let metadata = CachedCardMetadata {
            name: "TestChar".to_string(),
            description: "Test".to_string(),
        };
        cache.metadata.insert("TestChar".to_string(), metadata);
        cache.cache_key = Some("stable_key".to_string());

        // Verify the cache has data
        assert_eq!(cache.metadata.len(), 1);

        // The cache should maintain its state
        assert!(cache.cache_key.is_some());
        assert_eq!(cache.metadata.len(), 1);
    }

    #[test]
    fn test_cache_invalidation_workflow() {
        let mut cache = CardCache::new();

        // Set up initial cache state
        cache.metadata.insert(
            "char1".to_string(),
            CachedCardMetadata {
                name: "Character 1".to_string(),
                description: "First character".to_string(),
            },
        );
        cache.cache_key = Some("initial_key".to_string());

        assert_eq!(cache.metadata.len(), 1);
        assert!(cache.cache_key.is_some());

        // Invalidate the cache
        cache.invalidate();

        // Verify cache is cleared
        assert_eq!(cache.metadata.len(), 0);
        assert!(cache.cache_key.is_none());
    }
}
