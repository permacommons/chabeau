#[cfg(test)]
use crate::core::app::App;
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
        input_mode: true,
        edit_select_mode: false,
        selected_user_message_index: None,
        in_place_edit_index: None,
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
        picker: None,
        picker_mode: None,
        block_select_mode: false,
        selected_block_index: None,
        theme_before_picker: None,
        theme_id_before_picker: None,
        model_before_picker: None,
        model_search_filter: String::new(),
        all_available_models: Vec::new(),
        theme_search_filter: String::new(),
        all_available_themes: Vec::new(),
        markdown_enabled: true,
        syntax_enabled: true,
        prewrap_cache: None,
        status: None,
        status_set_at: None,
        file_prompt: None,
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
