//! Mode-aware keybinding registry system
//!
//! This module provides the core registry system for handling keybindings
//! in a mode-aware manner, including types, registry, and builder.

use crate::core::app::ui_state::UiMode;
use crate::core::app::{App, AppActionDispatcher};
use crate::ui::chat_loop::KeyLoopAction;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

// ============================================================================
// Types and Traits
// ============================================================================

/// Result of handling a key event
#[derive(Debug, Clone, PartialEq)]
pub enum KeyResult {
    /// Key was handled and should continue the loop
    Continue,
    /// Key was handled and should exit the loop
    Exit,
    /// Key was handled (generic)
    Handled,
    /// Key was not handled by this handler
    NotHandled,
}

impl From<KeyLoopAction> for KeyResult {
    fn from(action: KeyLoopAction) -> Self {
        match action {
            KeyLoopAction::Continue => KeyResult::Continue,
            KeyLoopAction::Break => KeyResult::Exit,
        }
    }
}

impl From<bool> for KeyResult {
    fn from(handled: bool) -> Self {
        if handled {
            KeyResult::Handled
        } else {
            KeyResult::NotHandled
        }
    }
}

/// Trait for keybinding handlers
#[async_trait::async_trait]
pub trait KeyHandler: Send + Sync {
    /// Handle a key event
    async fn handle(
        &self,
        app: &Arc<Mutex<App>>,
        dispatcher: &AppActionDispatcher,
        key: &KeyEvent,
        term_width: u16,
        term_height: u16,
        last_input_layout_update: Option<std::time::Instant>,
    ) -> KeyResult;
}

/// Pattern for matching key events
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyPattern {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyPattern {
    pub fn simple(code: KeyCode) -> Self {
        Self {
            code,
            modifiers: KeyModifiers::NONE,
        }
    }

    pub fn ctrl(code: KeyCode) -> Self {
        Self {
            code,
            modifiers: KeyModifiers::CONTROL,
        }
    }

    pub fn with_modifiers(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }

    /// Match any key (catch-all for mode-specific handlers)
    pub fn any() -> Self {
        Self {
            code: KeyCode::Null,          // Special marker for any key
            modifiers: KeyModifiers::ALT, // Use ALT as a marker for "any"
        }
    }

    pub fn matches(&self, key: &KeyEvent) -> bool {
        // Handle special patterns
        if self.code == KeyCode::Null && self.modifiers == KeyModifiers::ALT {
            // "any" pattern matches everything (for mode-specific catch-all handlers)
            return true;
        }

        // Normal exact pattern matching
        self.code == key.code && self.modifiers == key.modifiers
    }
}

impl From<&KeyEvent> for KeyPattern {
    fn from(key: &KeyEvent) -> Self {
        Self {
            code: key.code,
            modifiers: key.modifiers,
        }
    }
}

// ============================================================================
// Context and Registry
// ============================================================================

/// Context for mode-aware key handling
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeyContext {
    /// Normal typing mode
    Typing,
    /// Edit select mode (selecting messages to edit)
    EditSelect,
    /// Block select mode (selecting code blocks)
    BlockSelect,
    /// In-place edit mode
    InPlaceEdit,
    /// File prompt mode
    FilePrompt,
    /// Picker is open (model/theme selection)
    Picker,
}

impl KeyContext {
    /// Convert from UiMode to KeyContext
    pub fn from_ui_mode(ui_mode: &UiMode, picker_open: bool) -> Self {
        if picker_open {
            return KeyContext::Picker;
        }

        match ui_mode {
            UiMode::Typing => KeyContext::Typing,
            UiMode::EditSelect { .. } => KeyContext::EditSelect,
            UiMode::BlockSelect { .. } => KeyContext::BlockSelect,
            UiMode::InPlaceEdit { .. } => KeyContext::InPlaceEdit,
            UiMode::FilePrompt(_) => KeyContext::FilePrompt,
        }
    }
}

/// Mode-aware keybinding registry
pub struct ModeAwareRegistry {
    /// Handlers organized by context and key pattern
    handlers: HashMap<KeyContext, HashMap<KeyPattern, Box<dyn KeyHandler>>>,
}

impl ModeAwareRegistry {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// Register a handler for a specific context
    pub fn register_for_context(
        &mut self,
        context: KeyContext,
        pattern: KeyPattern,
        handler: Box<dyn KeyHandler>,
    ) {
        self.handlers
            .entry(context)
            .or_default()
            .insert(pattern, handler);
    }

    /// Check if a key should be handled as text input (bypass registry)
    pub fn should_handle_as_text_input(&self, key: &KeyEvent, context: &KeyContext) -> bool {
        match context {
            KeyContext::Typing => {
                // In typing mode, only character keys are text input
                if let KeyCode::Char(c) = key.code {
                    // System shortcuts should not be treated as text input
                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                        // These have dedicated handlers
                        return !matches!(
                            c,
                            'c' | 'l' | 'd' | 'b' | 'p' | 'j' | 'r' | 't' | 'a' | 'e'
                        );
                    }
                    // All other character input (regular chars, Shift+chars, Alt+chars, etc.)
                    return true;
                }
                false
            }
            KeyContext::FilePrompt | KeyContext::InPlaceEdit => {
                // In these modes, use blacklist approach - let tui-textarea handle most keys
                // Only block keys that need special mode-specific handling
                match key.code {
                    // Keys that must go through registry for special handling
                    KeyCode::Esc => false,   // Cancel prompt/edit
                    KeyCode::Enter => false, // Submit (needs special handling)
                    KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        // Block mode-switching shortcuts, allow text editing shortcuts
                        !matches!(c, 'b' | 'p' | 'j' | 'r' | 't' | 'c' | 'l' | 'd')
                    }
                    KeyCode::F(4) => false, // F4 toggle compose mode
                    // Alt+Enter needs special handling in FilePrompt for overwrite
                    _ if key.modifiers.contains(KeyModifiers::ALT)
                        && key.code == KeyCode::Enter =>
                    {
                        false
                    }
                    // Everything else goes to tui-textarea (including all text editing keys)
                    _ => true,
                }
            }
            _ => false,
        }
    }

    /// Handle a key event in the given context
    #[allow(clippy::too_many_arguments)]
    pub async fn handle_key_event(
        &self,
        app: &Arc<Mutex<App>>,
        dispatcher: &AppActionDispatcher,
        key: &KeyEvent,
        context: KeyContext,
        term_width: u16,
        term_height: u16,
        last_input_layout_update: Option<std::time::Instant>,
    ) -> ModeAwareResult {
        // First try context-specific handlers (they have priority)
        if let Some(context_handlers) = self.handlers.get(&context) {
            // First pass: try exact matches (non-wildcard patterns)
            for (pattern, handler) in context_handlers {
                if pattern.matches(key) && !is_wildcard_pattern(pattern) {
                    let result = handler
                        .handle(
                            app,
                            dispatcher,
                            key,
                            term_width,
                            term_height,
                            last_input_layout_update,
                        )
                        .await;
                    // Only return if the handler actually handled the key
                    if result != KeyResult::NotHandled {
                        return ModeAwareResult {
                            result,
                            updated_layout_time: last_input_layout_update,
                        };
                    }
                }
            }

            // Second pass: try wildcard patterns (any(), any_char())
            for (pattern, handler) in context_handlers {
                if pattern.matches(key) && is_wildcard_pattern(pattern) {
                    let result = handler
                        .handle(
                            app,
                            dispatcher,
                            key,
                            term_width,
                            term_height,
                            last_input_layout_update,
                        )
                        .await;
                    // Only return if the handler actually handled the key
                    if result != KeyResult::NotHandled {
                        return ModeAwareResult {
                            result,
                            updated_layout_time: last_input_layout_update,
                        };
                    }
                    // If handler returned NotHandled, continue to try other handlers
                }
            }
        }

        ModeAwareResult {
            result: KeyResult::NotHandled,
            updated_layout_time: None,
        }
    }
}

/// Result from mode-aware key handling
pub struct ModeAwareResult {
    pub result: KeyResult,
    pub updated_layout_time: Option<std::time::Instant>,
}

/// Helper function to detect wildcard patterns that should have lower priority
fn is_wildcard_pattern(pattern: &KeyPattern) -> bool {
    // Wildcard patterns use KeyCode::Null as a marker
    pattern.code == KeyCode::Null
}

impl Default for ModeAwareRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Builder
// ============================================================================

/// Builder for creating a fully configured mode-aware registry
pub struct ModeAwareBuilder {
    registry: ModeAwareRegistry,
}

impl ModeAwareBuilder {
    pub fn new() -> Self {
        Self {
            registry: ModeAwareRegistry::new(),
        }
    }

    /// Build the final registry
    pub fn build(self) -> ModeAwareRegistry {
        self.registry
    }

    /// Register a handler for a specific context
    pub fn register_for_context(
        mut self,
        context: KeyContext,
        pattern: KeyPattern,
        handler: Box<dyn KeyHandler>,
    ) -> Self {
        self.registry
            .register_for_context(context, pattern, handler);
        self
    }
}

impl Default for ModeAwareBuilder {
    fn default() -> Self {
        Self::new()
    }
}
