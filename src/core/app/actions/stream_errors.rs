use super::{App, AppActionContext, AppCommand};

pub(super) fn handle_stream_error(
    app: &mut App,
    message: String,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    super::handle_stream_error(app, message, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::app::actions::{AppCommand, StreamingAction};
    use crate::core::message::{ROLE_APP_ERROR, ROLE_ASSISTANT, ROLE_USER};
    use crate::utils::test_utils::create_test_app;

    fn default_ctx() -> AppActionContext {
        AppActionContext {
            term_width: 80,
            term_height: 24,
        }
    }

    #[test]
    fn stream_errored_drops_empty_assistant_placeholder() {
        let mut app = create_test_app();
        let ctx = default_ctx();

        let command = super::super::handle_streaming_action(
            &mut app,
            StreamingAction::SubmitMessage {
                message: "Hello there".into(),
            },
            ctx,
        );

        let stream_id = match command {
            Some(AppCommand::SpawnStream(params)) => params.stream_id,
            _ => panic!("expected stream"),
        };

        let result = super::super::handle_streaming_action(
            &mut app,
            StreamingAction::StreamErrored {
                message: " network failure ".into(),
                stream_id,
            },
            ctx,
        );
        assert!(result.is_none());
        assert!(app
            .ui
            .messages
            .iter()
            .all(|m| m.role != ROLE_ASSISTANT || !m.content.trim().is_empty()));
        let last = app.ui.messages.back().expect("last");
        assert_eq!(last.role, ROLE_APP_ERROR);
        assert_eq!(last.content, "network failure");
        assert_eq!(app.ui.messages.front().expect("first").role, ROLE_USER);
    }
}
