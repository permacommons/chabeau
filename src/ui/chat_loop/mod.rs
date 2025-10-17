//! Main chat event loop and UI rendering façade.
//!
//! This module exposes the public API for the chat UI while delegating the heavy
//! lifting to internal modules that handle lifecycle management, event polling, and
//! mode-specific behaviour.

mod event_loop;
pub mod keybindings;
mod lifecycle;
pub mod modes;
mod setup;

#[allow(unused_imports)]
pub use event_loop::{run_chat, UiEvent};
pub use keybindings::KeyLoopAction;

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::core::app::App;

#[derive(Clone)]
pub struct AppHandle {
    inner: Arc<Mutex<App>>,
}

impl AppHandle {
    pub fn new(inner: Arc<Mutex<App>>) -> Self {
        Self { inner }
    }

    pub async fn read<R>(&self, f: impl FnOnce(&App) -> R) -> R {
        let guard = self.inner.lock().await;
        f(&guard)
    }

    pub async fn update<R>(&self, f: impl FnOnce(&mut App) -> R) -> R {
        let mut guard = self.inner.lock().await;
        f(&mut guard)
    }
}
