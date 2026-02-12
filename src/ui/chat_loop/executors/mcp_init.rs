use tokio::sync::mpsc;

use crate::core::app::{AppActionContext, AppActionDispatcher, StreamingAction};
use crate::core::mcp_auth::McpTokenStore;

use super::super::AppHandle;

pub fn spawn_mcp_initializer(
    app: AppHandle,
    dispatcher: AppActionDispatcher,
    request_tx: mpsc::UnboundedSender<crate::mcp::events::McpServerRequest>,
) {
    tokio::spawn(async move {
        let mcp_disabled = app.read(|app| app.session.mcp_disabled).await;
        if mcp_disabled {
            dispatcher.dispatch_many(
                [StreamingAction::McpInitCompleted],
                AppActionContext::default(),
            );
            return;
        }

        let has_enabled_servers = app
            .read(|app| app.mcp.servers().any(|server| server.config.is_enabled()))
            .await;

        if !has_enabled_servers {
            dispatcher.dispatch_many(
                [StreamingAction::McpInitCompleted],
                AppActionContext::default(),
            );
            return;
        }

        let keyring_enabled = app.read(|app| !app.session.startup_env_only).await;
        let token_store = McpTokenStore::new_with_keyring(keyring_enabled);

        let config = app.read(|app| app.config.clone()).await;
        let mut mcp = crate::mcp::client::McpClientManager::from_config(&config);
        mcp.set_request_sender(request_tx.clone());
        mcp.connect_all(&token_store).await;

        let server_ids: Vec<String> = mcp
            .servers()
            .filter(|server| server.config.is_enabled())
            .map(|server| server.config.id.clone())
            .collect();

        for server_id in server_ids {
            mcp.refresh_server_metadata_concurrently(&server_id).await;
        }

        app.update(|app| {
            app.mcp = mcp;
            app.session.mcp_init_in_progress = false;
            app.session.mcp_init_complete = true;
        })
        .await;

        dispatcher.dispatch_many(
            [StreamingAction::McpInitCompleted],
            AppActionContext::default(),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::app::actions::{AppAction, AppActionDispatcher};
    use crate::ui::chat_loop::AppHandle;
    use crate::ui::theme::Theme;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Mutex;

    fn new_app_handle() -> AppHandle {
        let app = crate::core::app::App::new_test_app(Theme::dark_default(), true, true);
        AppHandle::new(Arc::new(Mutex::new(app)))
    }

    #[tokio::test]
    async fn mcp_initializer_dispatches_completion_when_connection_fails() {
        let app = new_app_handle();
        let failing_servers = vec![
            crate::core::config::data::McpServerConfig {
                id: "alpha".to_string(),
                display_name: "Alpha".to_string(),
                base_url: None,
                command: Some("/definitely-missing-command".to_string()),
                args: None,
                env: None,
                transport: Some("stdio".to_string()),
                allowed_tools: None,
                protocol_version: None,
                enabled: Some(true),
                tool_payloads: None,
                tool_payload_window: None,
                yolo: None,
            },
            crate::core::config::data::McpServerConfig {
                id: "beta".to_string(),
                display_name: "Beta".to_string(),
                base_url: None,
                command: Some("/definitely-missing-command-2".to_string()),
                args: None,
                env: None,
                transport: Some("stdio".to_string()),
                allowed_tools: None,
                protocol_version: None,
                enabled: Some(true),
                tool_payloads: None,
                tool_payload_window: None,
                yolo: None,
            },
        ];

        app.update(|app| {
            app.config.mcp_servers = failing_servers.clone();
            app.mcp = crate::mcp::client::McpClientManager::from_config(&app.config);
            app.session.mcp_init_in_progress = true;
            app.session.mcp_init_complete = false;
        })
        .await;

        let (action_tx, mut action_rx) = tokio::sync::mpsc::unbounded_channel();
        let dispatcher = AppActionDispatcher::new(action_tx);
        let (request_tx, _request_rx) = tokio::sync::mpsc::unbounded_channel();

        spawn_mcp_initializer(app.clone(), dispatcher, request_tx);

        let action = tokio::time::timeout(Duration::from_secs(5), action_rx.recv())
            .await
            .expect("initializer should dispatch completion")
            .expect("action should be present");
        assert!(matches!(
            action.action,
            AppAction::Streaming(crate::core::app::StreamingAction::McpInitCompleted)
        ));

        let (init_complete, init_in_progress) = app
            .read(|app| {
                (
                    app.session.mcp_init_complete,
                    app.session.mcp_init_in_progress,
                )
            })
            .await;

        assert!(init_complete);
        assert!(!init_in_progress);
    }
}
