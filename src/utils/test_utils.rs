#[cfg(test)]
use crate::core::app::{App, UiMode};
#[cfg(test)]
use crate::core::message::Message;
#[cfg(test)]
use crate::ui::theme::Theme;
#[cfg(test)]
use crate::utils::logging::LoggingState;
#[cfg(test)]
use std::collections::VecDeque;

#[cfg(test)]
pub fn create_test_app() -> App {
    App {
        messages: VecDeque::new(),
        input: String::new(),
        input_cursor_position: 0,
        mode: UiMode::Typing,
        current_response: String::new(),
        client: reqwest::Client::new(),
        model: "test-model".to_string(),
        api_key: "test-key".to_string(),
        base_url: "https://api.test.com".to_string(),
        provider_name: "test".to_string(),
        provider_display_name: "Test".to_string(),
        scroll_offset: 0,
        horizontal_scroll_offset: 0,
        auto_scroll: true,
        is_streaming: false,
        pulse_start: std::time::Instant::now(),
        stream_interrupted: false,
        logging: LoggingState::new(None).unwrap(),
        stream_cancel_token: None,
        current_stream_id: 0,
        last_retry_time: std::time::Instant::now(),
        retrying_message_index: None,
        input_scroll_offset: 0,
        textarea: tui_textarea::TextArea::default(),
        theme: Theme::dark_default(),
        current_theme_id: None,
        picker_session: None,
        in_provider_model_transition: false,
        provider_model_transition_state: None,
        markdown_enabled: true,
        syntax_enabled: true,
        prewrap_cache: None,
        status: None,
        status_set_at: None,
        startup_requires_provider: false,
        startup_requires_model: false,
        startup_multiple_providers_available: false,
        exit_requested: false,
        startup_env_only: false,
        compose_mode: false,
    }
}

#[cfg(test)]
pub fn create_test_message(role: &str, content: &str) -> Message {
    Message {
        role: role.to_string(),
        content: content.to_string(),
    }
}

#[cfg(test)]
pub fn create_test_messages() -> VecDeque<Message> {
    let mut messages = VecDeque::new();
    messages.push_back(create_test_message("user", "Hello"));
    messages.push_back(create_test_message("assistant", "Hi there!"));
    messages.push_back(create_test_message("user", "How are you?"));
    messages.push_back(create_test_message(
        "assistant",
        "I'm doing well, thank you for asking!",
    ));
    messages
}

#[cfg(test)]
pub const SAMPLE_HYPERTEXT_PARAGRAPH: &str = "The story of hypertext begins not with Tim Berners-Lee's World Wide Web, but with Vannevar Bush's 1945 essay \"As We May Think,\" where he envisioned the Memex - a device that would store books, records, and communications, and mechanically link them together by association. Ted Nelson, inspired by Bush's vision, coined the term \"hypertext\" in 1963 and spent decades developing [the original web proposal](https://www.example.com) - a system that would revolutionize how we think about documents, copyright, and knowledge itself. Nelson's Xanadu wasn't just about linking documents; it was about creating a [hypertext dreams](https://docs.hypertext.org) where every quotation would be automatically linked to its source, authors would be compensated for every use of their work, and the sum of human knowledge would be accessible through an elegant web of associations.";
