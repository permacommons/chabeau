use crate::character::CharacterCard;
use crate::core::builtin_providers::BuiltinProvider;
use crate::core::config::data::CustomProvider;
use crate::ui::builtin_themes::ThemeSpec;

use super::{sanitize_picker_metadata, sanitize_picker_metadata_for_inspect};

#[derive(Debug, Clone, Copy)]
pub(super) enum ThemeSource {
    Builtin,
    Custom,
}

impl ThemeSource {
    fn label(self) -> &'static str {
        match self {
            ThemeSource::Builtin => "Built-in theme",
            ThemeSource::Custom => "Custom theme (config.toml)",
        }
    }
}

pub(super) fn theme_metadata(
    spec: &ThemeSpec,
    source: ThemeSource,
    is_default: bool,
) -> (String, String) {
    let mut summary_parts = vec![source.label().to_string(), format!("ID: {}", spec.id)];
    if is_default {
        summary_parts.push("Default from config".to_string());
    }
    let summary = sanitize_picker_metadata(&summary_parts.join(" • "));

    let mut lines = vec![
        format!("Theme: {} (ID: {})", spec.display_name, spec.id),
        format!("Source: {}", source.label()),
    ];

    if is_default {
        lines.push("Status: Default theme from config".to_string());
    }

    append_theme_sections(&mut lines, spec);

    let inspect = build_inspect_text(lines);
    (summary, inspect)
}

fn append_theme_sections(lines: &mut Vec<String>, spec: &ThemeSpec) {
    lines.push(String::new());
    append_theme_section(
        lines,
        "General",
        &[
            "Background",
            "Selection highlight",
            "Title",
            "Streaming indicator",
        ],
        &[
            &spec.background,
            &spec.selection_highlight,
            &spec.title,
            &spec.streaming_indicator,
        ],
    );
    append_theme_section(
        lines,
        "Chat",
        &["User prefix", "User text", "Assistant text", "System text"],
        &[
            &spec.user_prefix,
            &spec.user_text,
            &spec.assistant_text,
            &spec.system_text,
        ],
    );
    append_theme_section(
        lines,
        "App messages",
        &[
            "Info prefix",
            "Info prefix style",
            "Info text",
            "Warning prefix",
            "Warning prefix style",
            "Warning text",
            "Error prefix",
            "Error prefix style",
            "Error text",
        ],
        &[
            &spec.app_info_prefix,
            &spec.app_info_prefix_style,
            &spec.app_info_text,
            &spec.app_warning_prefix,
            &spec.app_warning_prefix_style,
            &spec.app_warning_text,
            &spec.app_error_prefix,
            &spec.app_error_prefix_style,
            &spec.app_error_text,
        ],
    );
    append_theme_section(
        lines,
        "Input",
        &[
            "Border",
            "Title",
            "Text",
            "Cursor modifiers",
            "Cursor color",
        ],
        &[
            &spec.input_border,
            &spec.input_title,
            &spec.input_text,
            &spec.input_cursor_modifiers,
            &spec.cursor_color,
        ],
    );

    append_theme_section(
        lines,
        "Markdown",
        &[
            "Heading 1",
            "Heading 2",
            "Heading 3",
            "Heading 4",
            "Heading 5",
            "Heading 6",
            "Paragraph",
            "Inline code",
            "Link",
            "Rule",
            "Blockquote",
            "List marker",
            "Code block text",
            "Code block background",
        ],
        &[
            &spec.md_h1,
            &spec.md_h2,
            &spec.md_h3,
            &spec.md_h4,
            &spec.md_h5,
            &spec.md_h6,
            &spec.md_paragraph,
            &spec.md_inline_code,
            &spec.md_link,
            &spec.md_rule,
            &spec.md_blockquote_text,
            &spec.md_list_marker,
            &spec.md_codeblock_text,
            &spec.md_codeblock_bg,
        ],
    );
}

fn append_theme_section(
    lines: &mut Vec<String>,
    heading: &str,
    labels: &[&str],
    values: &[&Option<String>],
) {
    if let Some(section) = build_theme_section(heading, labels, values) {
        if !lines.last().map(|s| s.is_empty()).unwrap_or(false) {
            lines.push(String::new());
        }
        lines.extend(section.lines().map(|line| line.to_string()));
    }
}

fn build_theme_section(
    heading: &str,
    labels: &[&str],
    values: &[&Option<String>],
) -> Option<String> {
    let mut section_lines = Vec::new();
    for (label, value) in labels.iter().zip(values.iter()) {
        if let Some(value) = value {
            section_lines.push(format!("  {}: {}", label, value));
        }
    }

    if section_lines.is_empty() {
        None
    } else {
        let mut result = Vec::with_capacity(section_lines.len() + 1);
        result.push(heading.to_string());
        result.extend(section_lines);
        Some(result.join("\n"))
    }
}

fn trim_trailing_blank_lines(mut lines: Vec<String>) -> Vec<String> {
    while matches!(lines.last(), Some(line) if line.trim().is_empty()) {
        lines.pop();
    }
    lines
}

pub(crate) fn build_inspect_text(lines: Vec<String>) -> String {
    sanitize_picker_metadata_for_inspect(&trim_trailing_blank_lines(lines).join("\n"))
}

pub(super) fn provider_metadata_builtin(
    provider: &BuiltinProvider,
    is_default: bool,
) -> (String, String) {
    provider_metadata(
        &provider.display_name,
        &provider.id,
        &provider.base_url,
        "Built-in provider",
        Some(provider.auth_mode()),
        is_default,
    )
}

pub(super) fn provider_metadata_custom(
    provider: &CustomProvider,
    is_default: bool,
) -> (String, String) {
    provider_metadata(
        &provider.display_name,
        &provider.id,
        &provider.base_url,
        "Custom provider (config.toml)",
        provider.mode.as_deref(),
        is_default,
    )
}

fn provider_metadata(
    display_name: &str,
    id: &str,
    base_url: &str,
    source: &str,
    auth_mode: Option<&str>,
    is_default: bool,
) -> (String, String) {
    let mut summary_parts = vec![
        source.to_string(),
        format!("ID: {}", id),
        base_url.to_string(),
    ];
    if is_default {
        summary_parts.push("Default from config".to_string());
    }
    let summary = sanitize_picker_metadata(&summary_parts.join(" • "));

    let mut lines = vec![
        format!("Provider: {}", display_name),
        format!("ID: {}", id),
        format!("Source: {}", source),
        format!("Base URL: {}", base_url),
    ];

    if let Some(mode) = auth_mode {
        lines.push(format!("Authentication mode: {}", mode));
    }

    if is_default {
        lines.push("Status: Default provider from config".to_string());
    }

    let inspect = build_inspect_text(lines);
    (summary, inspect)
}

pub(super) fn character_inspect(card: &CharacterCard) -> String {
    let mut lines = vec![
        format!("Character: {}", card.data.name),
        format!("Spec: {} (version {})", card.spec, card.spec_version),
    ];

    append_character_block(&mut lines, "Description", &card.data.description);
    append_character_block(&mut lines, "Personality", &card.data.personality);
    append_character_block(&mut lines, "Scenario", &card.data.scenario);
    append_character_block(&mut lines, "First message", &card.data.first_mes);
    append_character_block(&mut lines, "Example dialogue", &card.data.mes_example);
    append_character_block_optional(
        &mut lines,
        "System prompt",
        card.data.system_prompt.as_deref(),
    );
    append_character_block_optional(
        &mut lines,
        "Creator notes",
        card.data.creator_notes.as_deref(),
    );
    append_character_block_optional(
        &mut lines,
        "Post-history instructions",
        card.data.post_history_instructions.as_deref(),
    );
    append_character_list(
        &mut lines,
        "Alternate greetings",
        card.data.alternate_greetings.as_ref(),
    );

    if let Some(tags) = card.data.tags.as_ref() {
        if !tags.is_empty() {
            lines.push(String::new());
            lines.push(format!("Tags: {}", tags.join(", ")));
        }
    }

    if let Some(creator) = card.data.creator.as_ref() {
        if !creator.trim().is_empty() {
            lines.push(String::new());
            lines.push(format!("Creator: {}", creator.trim()));
        }
    }

    if let Some(version) = card.data.character_version.as_ref() {
        if !version.trim().is_empty() {
            lines.push(String::new());
            lines.push(format!("Version: {}", version.trim()));
        }
    }

    build_inspect_text(lines)
}

fn append_character_block(lines: &mut Vec<String>, heading: &str, content: &str) {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return;
    }

    lines.push(String::new());
    lines.push(format!("{}:", heading));
    for line in trimmed.lines() {
        lines.push(format!("  {}", line.trim_end()));
    }
}

fn append_character_block_optional(lines: &mut Vec<String>, heading: &str, content: Option<&str>) {
    if let Some(content) = content {
        append_character_block(lines, heading, content);
    }
}

fn append_character_list(lines: &mut Vec<String>, heading: &str, items: Option<&Vec<String>>) {
    if let Some(items) = items {
        let filtered: Vec<&str> = items
            .iter()
            .map(|item| item.trim())
            .filter(|item| !item.is_empty())
            .collect();
        if filtered.is_empty() {
            return;
        }

        lines.push(String::new());
        lines.push(format!("{}:", heading));
        for item in filtered {
            for (idx, line) in item.lines().enumerate() {
                if idx == 0 {
                    lines.push(format!("  - {}", line.trim_end()));
                } else {
                    lines.push(format!("    {}", line.trim_end()));
                }
            }
        }
    }
}
