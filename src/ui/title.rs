use crate::core::app::ui_state::UiFocus;
use crate::core::app::App;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const SEPARATOR: &str = " • ";

#[derive(Debug, Clone)]
struct FieldVariant {
    text: String,
    width: usize,
}

impl FieldVariant {
    fn new(text: String) -> Self {
        let width = UnicodeWidthStr::width(text.as_str());
        Self { text, width }
    }
}

fn build_variants(label: &str, value: &str) -> Vec<FieldVariant> {
    let mut variants = Vec::new();
    let full_text = format!("{}{}", label, value);
    variants.push(FieldVariant::new(full_text));

    let graphemes: Vec<&str> = UnicodeSegmentation::graphemes(value, true).collect();
    if graphemes.len() > 3 {
        for keep in (3..graphemes.len()).rev() {
            let mut truncated_value = graphemes[..keep].concat();
            truncated_value.push('…');
            let text = format!("{}{}", label, truncated_value);
            if variants
                .last()
                .map(|variant| variant.text == text)
                .unwrap_or(false)
            {
                continue;
            }
            variants.push(FieldVariant::new(text));
        }
    }

    variants
}

fn compute_total_width(
    base_width: usize,
    logging_width: usize,
    char_variant: Option<&FieldVariant>,
    preset_variant: Option<&FieldVariant>,
    mcp_variant: Option<&FieldVariant>,
    separator_width: usize,
) -> usize {
    let mut widths = vec![base_width];
    if let Some(char_variant) = char_variant {
        widths.push(char_variant.width);
    }
    if let Some(preset_variant) = preset_variant {
        widths.push(preset_variant.width);
    }
    if let Some(mcp_variant) = mcp_variant {
        widths.push(mcp_variant.width);
    }
    widths.push(logging_width);

    let separators = widths.len().saturating_sub(1);
    widths.into_iter().sum::<usize>() + separators * separator_width
}

fn select_char_only(
    char_variants: &[FieldVariant],
    base_width: usize,
    logging_width: usize,
    separator_width: usize,
    available_width: usize,
) -> Option<usize> {
    for (index, variant) in char_variants.iter().enumerate() {
        if compute_total_width(
            base_width,
            logging_width,
            Some(variant),
            None,
            None,
            separator_width,
        ) <= available_width
        {
            return Some(index);
        }
    }

    None
}

fn select_preset_only(
    preset_variants: &[FieldVariant],
    base_width: usize,
    logging_width: usize,
    separator_width: usize,
    available_width: usize,
) -> Option<usize> {
    for (index, variant) in preset_variants.iter().enumerate() {
        if compute_total_width(
            base_width,
            logging_width,
            None,
            Some(variant),
            None,
            separator_width,
        ) <= available_width
        {
            return Some(index);
        }
    }

    None
}

fn select_char_and_preset(
    char_variants: &[FieldVariant],
    preset_variants: &[FieldVariant],
    base_width: usize,
    logging_width: usize,
    separator_width: usize,
    available_width: usize,
) -> Option<(usize, usize)> {
    if char_variants.is_empty() || preset_variants.is_empty() {
        return None;
    }

    let preset_full = &preset_variants[0];
    for (index, char_variant) in char_variants.iter().enumerate() {
        if compute_total_width(
            base_width,
            logging_width,
            Some(char_variant),
            Some(preset_full),
            None,
            separator_width,
        ) <= available_width
        {
            return Some((index, 0));
        }
    }

    for (preset_index, preset_variant) in preset_variants.iter().enumerate().skip(1) {
        for (char_index, char_variant) in char_variants.iter().enumerate() {
            if compute_total_width(
                base_width,
                logging_width,
                Some(char_variant),
                Some(preset_variant),
                None,
                separator_width,
            ) <= available_width
            {
                return Some((char_index, preset_index));
            }
        }
    }

    None
}

fn assemble_title(
    base: &str,
    char_variant: Option<&FieldVariant>,
    preset_variant: Option<&FieldVariant>,
    mcp_variant: Option<&FieldVariant>,
    logging: &FieldVariant,
) -> String {
    let mut parts: Vec<&str> = Vec::new();
    parts.push(base);
    if let Some(char_variant) = char_variant {
        parts.push(char_variant.text.as_str());
    }
    if let Some(preset_variant) = preset_variant {
        parts.push(preset_variant.text.as_str());
    }
    if let Some(mcp_variant) = mcp_variant {
        parts.push(mcp_variant.text.as_str());
    }
    parts.push(logging.text.as_str());
    parts.join(SEPARATOR)
}

fn mcp_field(app: &App) -> Option<FieldVariant> {
    let enabled_servers: Vec<_> = app
        .mcp
        .servers()
        .filter(|server| server.config.is_enabled())
        .collect();

    if enabled_servers.is_empty() {
        return None;
    }

    let status = if app.session.mcp_tools_unsupported {
        "unsupported"
    } else if app.session.mcp_init.in_progress {
        "loading"
    } else if enabled_servers
        .iter()
        .any(|server| server.last_error.is_some())
    {
        "error"
    } else if enabled_servers.iter().any(|server| server.connected) {
        "connected"
    } else if app.session.mcp_init.complete {
        "error"
    } else {
        "loading"
    };

    Some(FieldVariant::new(format!("MCP: {}", status)))
}

pub fn build_main_title(app: &App, available_width: u16) -> String {
    let available_width = available_width as usize;

    let model_display = if app.picker.in_provider_model_transition || app.session.model.is_empty() {
        "no model selected".to_string()
    } else {
        app.session.model.clone()
    };
    let provider_display = if app.session.provider_display_name.trim().is_empty() {
        "(no provider selected)".to_string()
    } else {
        app.session.provider_display_name.clone()
    };

    let focus_prefix = if app.ui.focus == UiFocus::Transcript {
        "› "
    } else {
        "· "
    };
    let base_text = format!(
        "{}Chabeau v{} - {} ({})",
        focus_prefix,
        env!("CARGO_PKG_VERSION"),
        provider_display,
        model_display
    );
    let base_width = UnicodeWidthStr::width(base_text.as_str());

    let logging_variant = FieldVariant::new(format!("Logging: {}", app.get_logging_status()));

    let char_variants = app
        .session
        .active_character
        .as_ref()
        .map(|character| build_variants("Character: ", character.data.name.as_str()));
    let preset_variants = app
        .preset_manager
        .get_active_preset()
        .map(|preset| build_variants("Preset: ", preset.id.as_str()));
    let mcp_variant = mcp_field(app);

    let separator_width = UnicodeWidthStr::width(SEPARATOR);

    let mut selected_char: Option<&FieldVariant> = None;
    let mut selected_preset: Option<&FieldVariant> = None;

    if let Some(char_variants) = char_variants.as_ref() {
        if let Some(preset_variants) = preset_variants.as_ref() {
            if let Some((char_index, preset_index)) = select_char_and_preset(
                char_variants,
                preset_variants,
                base_width,
                logging_variant.width,
                separator_width,
                available_width,
            ) {
                selected_char = Some(&char_variants[char_index]);
                selected_preset = Some(&preset_variants[preset_index]);
            }
        } else if let Some(char_index) = select_char_only(
            char_variants,
            base_width,
            logging_variant.width,
            separator_width,
            available_width,
        ) {
            selected_char = Some(&char_variants[char_index]);
        }
    }

    if selected_preset.is_none() {
        if let Some(preset_variants) = preset_variants.as_ref() {
            if let Some(preset_index) = select_preset_only(
                preset_variants,
                base_width,
                logging_variant.width,
                separator_width,
                available_width,
            ) {
                selected_preset = Some(&preset_variants[preset_index]);
            }
        }
    }

    if let Some(mcp_variant) = mcp_variant.as_ref() {
        if compute_total_width(
            base_width,
            logging_variant.width,
            selected_char,
            selected_preset,
            Some(mcp_variant),
            separator_width,
        ) <= available_width
        {
            return assemble_title(
                &base_text,
                selected_char,
                selected_preset,
                Some(mcp_variant),
                &logging_variant,
            );
        }
    }

    assemble_title(
        &base_text,
        selected_char,
        selected_preset,
        None,
        &logging_variant,
    )
}
