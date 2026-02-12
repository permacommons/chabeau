use std::fs;
use std::path::Path;

use super::{input, App, AppActionContext, AppCommand, FilePromptAction};

pub(super) fn handle_file_prompt_action(
    app: &mut App,
    action: FilePromptAction,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    match action {
        FilePromptAction::CompleteDump {
            filename,
            overwrite,
        } => {
            handle_file_prompt_dump(app, filename, overwrite, ctx);
            None
        }
        FilePromptAction::CompleteSaveBlock {
            filename,
            content,
            overwrite,
        } => {
            handle_file_prompt_save_block(app, filename, content, overwrite, ctx);
            None
        }
    }
}

fn handle_file_prompt_dump(
    app: &mut App,
    filename: String,
    overwrite: bool,
    ctx: AppActionContext,
) {
    if filename.is_empty() {
        return;
    }

    match crate::commands::dump_conversation_with_overwrite(app, &filename, overwrite) {
        Ok(()) => {
            input::set_status_message(app, format!("Dumped: {}", filename), ctx);
            app.cancel_file_prompt();
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("exists") && !overwrite {
                input::set_status_message(
                    app,
                    "File exists (Alt+Enter to overwrite)".to_string(),
                    ctx,
                );
            } else {
                input::set_status_message(app, format!("Dump error: {}", msg), ctx);
            }
        }
    }
}

fn handle_file_prompt_save_block(
    app: &mut App,
    filename: String,
    content: String,
    overwrite: bool,
    ctx: AppActionContext,
) {
    if filename.is_empty() {
        return;
    }

    if Path::new(&filename).exists() && !overwrite {
        input::set_status_message(app, "File already exists.".to_string(), ctx);
        return;
    }

    match fs::write(&filename, content) {
        Ok(()) => {
            input::set_status_message(app, format!("Saved to {}", filename), ctx);
            app.cancel_file_prompt();
        }
        Err(_e) => {
            input::set_status_message(app, "Error saving code block".to_string(), ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::test_utils::{create_test_app, create_test_message};
    use tempfile::tempdir;

    fn default_ctx() -> AppActionContext {
        AppActionContext {
            term_width: 80,
            term_height: 24,
        }
    }

    #[test]
    fn file_prompt_dump_success_sets_status_and_closes_prompt() {
        let mut app = create_test_app();
        app.ui.messages.push_back(create_test_message("user", "hi"));
        let dir = tempdir().unwrap();
        let path = dir.path().join("dump.txt");
        let filename = path.to_str().unwrap().to_string();

        app.ui.start_file_prompt_dump(filename.clone());

        handle_file_prompt_dump(&mut app, filename.clone(), false, default_ctx());

        assert!(path.exists());
        assert_eq!(
            app.ui.status.as_deref(),
            Some(&format!("Dumped: {}", filename)[..])
        );
        assert!(app.ui.file_prompt().is_none());
    }

    #[test]
    fn file_prompt_dump_existing_without_overwrite_sets_status() {
        let mut app = create_test_app();
        app.ui.messages.push_back(create_test_message("user", "hi"));
        let dir = tempdir().unwrap();
        let path = dir.path().join("dump.txt");
        std::fs::write(&path, "existing").unwrap();
        let filename = path.to_str().unwrap().to_string();

        app.ui.start_file_prompt_dump(filename.clone());

        handle_file_prompt_dump(&mut app, filename, false, default_ctx());

        assert_eq!(
            app.ui.status.as_deref(),
            Some("File exists (Alt+Enter to overwrite)")
        );
        assert!(app.ui.file_prompt().is_some());
    }

    #[test]
    fn file_prompt_save_block_success_writes_file() {
        let mut app = create_test_app();
        let dir = tempdir().unwrap();
        let path = dir.path().join("snippet.rs");
        let filename = path.to_str().unwrap().to_string();

        app.ui
            .start_file_prompt_save_block(filename.clone(), "fn main() {}".into());

        handle_file_prompt_save_block(
            &mut app,
            filename.clone(),
            "fn main() {}".into(),
            true,
            default_ctx(),
        );

        assert!(path.exists());
        assert_eq!(
            app.ui.status.as_deref(),
            Some(&format!("Saved to {}", filename)[..])
        );
        assert!(app.ui.file_prompt().is_none());
    }
}
