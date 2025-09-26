//! Mode-aware keybinding system
//!
//! This module provides a mode-aware keybinding system that dispatches
//! key events based on the current UI mode.

pub mod handlers;
pub mod registry;

// Public exports
pub use handlers::{scroll_block_into_view, wrap_next_index, wrap_previous_index};
pub use registry::{KeyContext, KeyResult, ModeAwareRegistry};

/// Action to take in the main event loop
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyLoopAction {
    Continue,
    Break,
}

/// Build a complete mode-aware registry with all handlers
pub fn build_mode_aware_registry(
    stream_dispatcher: std::sync::Arc<crate::ui::chat_loop::stream::StreamDispatcher>,
    terminal: std::sync::Arc<
        tokio::sync::Mutex<ratatui::Terminal<crate::ui::osc_backend::OscBackend<std::io::Stdout>>>,
    >,
) -> ModeAwareRegistry {
    use handlers::*;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    use registry::{KeyContext, KeyPattern, ModeAwareBuilder};

    ModeAwareBuilder::new()
        // These handlers only work when NOT in picker mode
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::simple(KeyCode::Esc),
            Box::new(EscapeHandler),
        )
        .register_for_context(
            KeyContext::EditSelect,
            KeyPattern::simple(KeyCode::Esc),
            Box::new(EscapeHandler),
        )
        .register_for_context(
            KeyContext::BlockSelect,
            KeyPattern::simple(KeyCode::Esc),
            Box::new(EscapeHandler),
        )
        .register_for_context(
            KeyContext::InPlaceEdit,
            KeyPattern::simple(KeyCode::Esc),
            Box::new(EscapeHandler),
        )
        .register_for_context(
            KeyContext::FilePrompt,
            KeyPattern::simple(KeyCode::Esc),
            Box::new(EscapeHandler),
        )
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::ctrl(KeyCode::Char('l')),
            Box::new(CtrlLHandler),
        )
        .register_for_context(
            KeyContext::EditSelect,
            KeyPattern::ctrl(KeyCode::Char('l')),
            Box::new(CtrlLHandler),
        )
        .register_for_context(
            KeyContext::BlockSelect,
            KeyPattern::ctrl(KeyCode::Char('l')),
            Box::new(CtrlLHandler),
        )
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::simple(KeyCode::F(4)),
            Box::new(F4Handler),
        )
        .register_for_context(
            KeyContext::EditSelect,
            KeyPattern::simple(KeyCode::F(4)),
            Box::new(F4Handler),
        )
        .register_for_context(
            KeyContext::BlockSelect,
            KeyPattern::simple(KeyCode::F(4)),
            Box::new(F4Handler),
        )
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::ctrl(KeyCode::Char('d')),
            Box::new(CtrlDHandler),
        )
        .register_for_context(
            KeyContext::EditSelect,
            KeyPattern::ctrl(KeyCode::Char('d')),
            Box::new(CtrlDHandler),
        )
        .register_for_context(
            KeyContext::BlockSelect,
            KeyPattern::ctrl(KeyCode::Char('d')),
            Box::new(CtrlDHandler),
        )
        // Navigation handlers for typing mode
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::simple(KeyCode::Home),
            Box::new(NavigationHandler),
        )
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::simple(KeyCode::End),
            Box::new(NavigationHandler),
        )
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::simple(KeyCode::PageUp),
            Box::new(NavigationHandler),
        )
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::simple(KeyCode::PageDown),
            Box::new(NavigationHandler),
        )
        // Arrow keys for typing mode
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::simple(KeyCode::Up),
            Box::new(ArrowKeyHandler),
        )
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::simple(KeyCode::Down),
            Box::new(ArrowKeyHandler),
        )
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::simple(KeyCode::Left),
            Box::new(ArrowKeyHandler),
        )
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::simple(KeyCode::Right),
            Box::new(ArrowKeyHandler),
        )
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::with_modifiers(KeyCode::Up, KeyModifiers::SHIFT),
            Box::new(ArrowKeyHandler),
        )
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::with_modifiers(KeyCode::Down, KeyModifiers::SHIFT),
            Box::new(ArrowKeyHandler),
        )
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::with_modifiers(KeyCode::Left, KeyModifiers::SHIFT),
            Box::new(ArrowKeyHandler),
        )
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::with_modifiers(KeyCode::Right, KeyModifiers::SHIFT),
            Box::new(ArrowKeyHandler),
        )
        // Text editing keys for typing mode (specific keys only)
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::ctrl(KeyCode::Char('a')),
            Box::new(TextEditingHandler),
        )
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::ctrl(KeyCode::Char('e')),
            Box::new(TextEditingHandler),
        )
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::simple(KeyCode::Delete),
            Box::new(TextEditingHandler),
        )
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::simple(KeyCode::Backspace),
            Box::new(TextEditingHandler),
        )
        // Mode switching handlers
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::ctrl(KeyCode::Char('b')),
            Box::new(CtrlBHandler),
        )
        .register_for_context(
            KeyContext::BlockSelect,
            KeyPattern::ctrl(KeyCode::Char('b')),
            Box::new(CtrlBHandler),
        )
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::ctrl(KeyCode::Char('p')),
            Box::new(CtrlPHandler),
        )
        .register_for_context(
            KeyContext::EditSelect,
            KeyPattern::ctrl(KeyCode::Char('p')),
            Box::new(CtrlPHandler),
        )
        // Complex handlers that need dependencies
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::ctrl(KeyCode::Char('j')),
            Box::new(CtrlJHandler {
                stream_dispatcher: stream_dispatcher.clone(),
            }),
        )
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::simple(KeyCode::Enter),
            Box::new(EnterHandler {
                stream_dispatcher: stream_dispatcher.clone(),
            }),
        )
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::with_modifiers(KeyCode::Enter, KeyModifiers::ALT),
            Box::new(AltEnterHandler {
                stream_dispatcher: stream_dispatcher.clone(),
            }),
        )
        // Enter handlers for FilePrompt and InPlaceEdit modes
        .register_for_context(
            KeyContext::FilePrompt,
            KeyPattern::simple(KeyCode::Enter),
            Box::new(EnterHandler {
                stream_dispatcher: stream_dispatcher.clone(),
            }),
        )
        .register_for_context(
            KeyContext::FilePrompt,
            KeyPattern::with_modifiers(KeyCode::Enter, KeyModifiers::ALT),
            Box::new(AltEnterHandler {
                stream_dispatcher: stream_dispatcher.clone(),
            }),
        )
        .register_for_context(
            KeyContext::InPlaceEdit,
            KeyPattern::simple(KeyCode::Enter),
            Box::new(EnterHandler {
                stream_dispatcher: stream_dispatcher.clone(),
            }),
        )
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::ctrl(KeyCode::Char('r')),
            Box::new(CtrlRHandler {
                stream_dispatcher: stream_dispatcher.clone(),
            }),
        )
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::ctrl(KeyCode::Char('t')),
            Box::new(CtrlTHandler {
                stream_dispatcher,
                terminal,
            }),
        )
        // Mode-specific system handlers (must come before catch-all handlers)
        // Ctrl+C should work in all modes (emergency exit)
        .register_for_context(
            KeyContext::Typing,
            KeyPattern::ctrl(KeyCode::Char('c')),
            Box::new(CtrlCHandler),
        )
        .register_for_context(
            KeyContext::Picker,
            KeyPattern::ctrl(KeyCode::Char('c')),
            Box::new(CtrlCHandler),
        )
        .register_for_context(
            KeyContext::EditSelect,
            KeyPattern::ctrl(KeyCode::Char('c')),
            Box::new(CtrlCHandler),
        )
        .register_for_context(
            KeyContext::BlockSelect,
            KeyPattern::ctrl(KeyCode::Char('c')),
            Box::new(CtrlCHandler),
        )
        .register_for_context(
            KeyContext::InPlaceEdit,
            KeyPattern::ctrl(KeyCode::Char('c')),
            Box::new(CtrlCHandler),
        )
        .register_for_context(
            KeyContext::FilePrompt,
            KeyPattern::ctrl(KeyCode::Char('c')),
            Box::new(CtrlCHandler),
        )
        // Mode-specific catch-all handlers (register last)
        .register_for_context(
            KeyContext::EditSelect,
            KeyPattern::any(),
            Box::new(EditSelectHandler),
        )
        .register_for_context(
            KeyContext::BlockSelect,
            KeyPattern::any(),
            Box::new(BlockSelectHandler),
        )
        .register_for_context(
            KeyContext::Picker,
            KeyPattern::any(),
            Box::new(PickerHandler),
        )
        .build()
}
