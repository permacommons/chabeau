//! Registry of setting handlers.

use std::collections::HashMap;

use super::handlers::boolean::{builtin_presets_handler, markdown_handler, syntax_handler};
use super::handlers::{
    DefaultCharacterHandler, DefaultModelHandler, DefaultPersonaHandler, DefaultPresetHandler,
    DefaultProviderHandler, McpHandler, RefineInstructionsHandler, RefinePrefixHandler,
    ThemeHandler,
};
use super::SettingHandler;

/// Registry of all available setting handlers.
pub struct SettingRegistry {
    handlers: HashMap<&'static str, Box<dyn SettingHandler>>,
    /// Keys in display order for `chabeau set` output.
    display_order: Vec<&'static str>,
}

impl SettingRegistry {
    /// Create a new registry with all handlers registered.
    pub fn new() -> Self {
        let mut registry = Self {
            handlers: HashMap::new(),
            display_order: Vec::new(),
        };

        // Register handlers in display order
        registry.register(Box::new(DefaultProviderHandler));
        registry.register(Box::new(ThemeHandler));
        registry.register(Box::new(markdown_handler()));
        registry.register(Box::new(syntax_handler()));
        registry.register(Box::new(builtin_presets_handler()));
        registry.register(Box::new(RefineInstructionsHandler));
        registry.register(Box::new(RefinePrefixHandler));
        registry.register(Box::new(DefaultModelHandler));
        registry.register(Box::new(DefaultCharacterHandler));
        registry.register(Box::new(DefaultPersonaHandler));
        registry.register(Box::new(DefaultPresetHandler));
        registry.register(Box::new(McpHandler));

        registry
    }

    fn register(&mut self, handler: Box<dyn SettingHandler>) {
        let key = handler.key();
        self.display_order.push(key);
        self.handlers.insert(key, handler);
    }

    /// Get a handler by key.
    pub fn get(&self, key: &str) -> Option<&dyn SettingHandler> {
        self.handlers.get(key).map(|h| h.as_ref())
    }

    /// Get all keys in sorted order.
    pub fn keys_sorted(&self) -> Vec<&'static str> {
        let mut keys: Vec<_> = self.handlers.keys().copied().collect();
        keys.sort();
        keys
    }

    /// Get all keys in display order.
    pub fn keys_display_order(&self) -> &[&'static str] {
        &self.display_order
    }
}

impl Default for SettingRegistry {
    fn default() -> Self {
        Self::new()
    }
}
