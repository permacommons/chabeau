use crate::core::message::AppMessageKind;
use crate::ui::builtin_themes::ThemeSpec;
use crate::utils::color::color_to_rgb;
use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone)]
pub struct AppMessageStyle {
    pub prefix: String,
    pub prefix_style: Style,
    pub text_style: Style,
}

#[derive(Debug, Clone)]
pub struct AppMessageStyles {
    pub info: AppMessageStyle,
    pub warning: AppMessageStyle,
    pub error: AppMessageStyle,
    pub log: AppMessageStyle,
}

impl AppMessageStyles {
    pub fn fallback() -> Self {
        AppMessageStyles {
            info: AppMessageStyle {
                prefix: "ℹ️  ".to_string(),
                prefix_style: Style::default().fg(Color::Rgb(125, 211, 252)),
                text_style: Style::default().fg(Color::Rgb(191, 219, 254)),
            },
            warning: AppMessageStyle {
                prefix: "⚠️  ".to_string(),
                prefix_style: Style::default().fg(Color::Rgb(253, 224, 71)),
                text_style: Style::default().fg(Color::Rgb(250, 204, 21)),
            },
            error: AppMessageStyle {
                prefix: "⛔  ".to_string(),
                prefix_style: Style::default().fg(Color::LightRed),
                text_style: Style::default().fg(Color::LightRed),
            },
            log: AppMessageStyle {
                prefix: "📝  ".to_string(),
                prefix_style: Style::default().fg(Color::Rgb(156, 163, 175)),
                text_style: Style::default().fg(Color::Rgb(209, 213, 219)),
            },
        }
    }

    pub fn style(&self, kind: AppMessageKind) -> &AppMessageStyle {
        match kind {
            AppMessageKind::Info => &self.info,
            AppMessageKind::Warning => &self.warning,
            AppMessageKind::Error => &self.error,
            AppMessageKind::Log => &self.log,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    // Overall background color to paint the full frame
    pub background_color: Color,
    // Chat message styles
    pub user_prefix_style: Style,
    pub user_text_style: Style,
    pub assistant_text_style: Style,
    pub system_text_style: Style,
    pub error_text_style: Style,
    pub app_messages: AppMessageStyles,

    // Chrome
    pub title_style: Style,
    pub streaming_indicator_style: Style,
    pub selection_highlight_style: Style,
    pub input_border_style: Style,
    pub input_title_style: Style,

    // Input area
    pub input_text_style: Style,
    pub input_cursor_style: Style,
    pub input_cursor_line_style: Style,
    pub input_cursor_color: Option<Color>,

    // Markdown styles (optional overrides with sensible fallbacks)
    pub md_h1: Option<Style>,
    pub md_h2: Option<Style>,
    pub md_h3: Option<Style>,
    pub md_h4: Option<Style>,
    pub md_h5: Option<Style>,
    pub md_h6: Option<Style>,
    pub md_paragraph: Option<Style>,
    pub md_inline_code: Option<Style>,
    pub md_link: Option<Style>,
    pub md_rule: Option<Style>,
    pub md_blockquote_text: Option<Style>,
    pub md_list_marker: Option<Style>,
    pub md_codeblock_text: Option<Style>,
    pub md_codeblock_bg: Option<Color>,
}

impl Theme {
    pub fn dark_default() -> Self {
        // Prefer built-in spec for consistent RGB colors
        if let Some(spec) = crate::ui::builtin_themes::find_builtin_theme("dark") {
            return Self::from_spec(&spec);
        }
        // Fallback palette-based theme
        Theme {
            background_color: Color::Black,
            user_prefix_style: Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            user_text_style: Style::default().fg(Color::Cyan),
            assistant_text_style: Style::default().fg(Color::White),
            system_text_style: Style::default().fg(Color::DarkGray),
            error_text_style: Style::default().fg(Color::LightRed),
            app_messages: AppMessageStyles::fallback(),

            title_style: Style::default().fg(Color::Gray),
            streaming_indicator_style: Style::default().fg(Color::White),
            selection_highlight_style: Style::default().bg(Color::Rgb(31, 41, 55)),
            input_border_style: Style::default().fg(Color::Gray),
            input_title_style: Style::default().fg(Color::Gray),

            input_text_style: Style::default().fg(Color::White),
            input_cursor_style: Style::default().add_modifier(Modifier::REVERSED),
            input_cursor_line_style: Style::default(),
            input_cursor_color: Some(Self::default_cursor_color_for_bg(Color::Black)),
            md_h1: None,
            md_h2: None,
            md_h3: None,
            md_h4: None,
            md_h5: None,
            md_h6: None,
            md_paragraph: None,
            md_inline_code: None,
            md_link: None,
            md_rule: None,
            md_blockquote_text: None,
            md_list_marker: None,
            md_codeblock_text: None,
            md_codeblock_bg: None,
        }
    }

    pub fn light() -> Self {
        // Prefer built-in spec for consistent RGB colors
        if let Some(spec) = crate::ui::builtin_themes::find_builtin_theme("light") {
            return Self::from_spec(&spec);
        }
        // Fallback palette-based theme
        Theme {
            background_color: Color::White,
            user_prefix_style: Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
            user_text_style: Style::default().fg(Color::Blue),
            assistant_text_style: Style::default().fg(Color::Black),
            system_text_style: Style::default().fg(Color::Gray),
            error_text_style: Style::default().fg(Color::Red),
            app_messages: AppMessageStyles {
                info: AppMessageStyle {
                    prefix: "ℹ️  ".to_string(),
                    prefix_style: Style::default().fg(Color::Rgb(37, 99, 235)),
                    text_style: Style::default().fg(Color::Rgb(29, 78, 216)),
                },
                warning: AppMessageStyle {
                    prefix: "⚠️  ".to_string(),
                    prefix_style: Style::default().fg(Color::Rgb(217, 119, 6)),
                    text_style: Style::default().fg(Color::Rgb(202, 138, 4)),
                },
                error: AppMessageStyle {
                    prefix: "⛔  ".to_string(),
                    prefix_style: Style::default().fg(Color::Red),
                    text_style: Style::default().fg(Color::Red),
                },
                log: AppMessageStyle {
                    prefix: "📝  ".to_string(),
                    prefix_style: Style::default().fg(Color::Rgb(75, 85, 99)),
                    text_style: Style::default().fg(Color::Rgb(55, 65, 81)),
                },
            },

            title_style: Style::default().fg(Color::DarkGray),
            streaming_indicator_style: Style::default().fg(Color::Black),
            selection_highlight_style: Style::default().bg(Color::Rgb(219, 234, 254)),
            input_border_style: Style::default().fg(Color::Black),
            input_title_style: Style::default().fg(Color::DarkGray),

            input_text_style: Style::default().fg(Color::Black),
            input_cursor_style: Style::default().add_modifier(Modifier::REVERSED),
            input_cursor_line_style: Style::default(),
            input_cursor_color: Some(Self::default_cursor_color_for_bg(Color::White)),
            md_h1: None,
            md_h2: None,
            md_h3: None,
            md_h4: None,
            md_h5: None,
            md_h6: None,
            md_paragraph: None,
            md_inline_code: None,
            md_link: None,
            md_rule: None,
            md_blockquote_text: None,
            md_list_marker: None,
            md_codeblock_text: None,
            md_codeblock_bg: None,
        }
    }

    pub fn dracula() -> Self {
        // Prefer built-in spec for consistency
        if let Some(spec) = crate::ui::builtin_themes::find_builtin_theme("dracula") {
            return Self::from_spec(&spec);
        }
        // Fallback palette-based theme
        Theme {
            background_color: Color::Black,
            user_prefix_style: Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
            user_text_style: Style::default().fg(Color::Magenta),
            assistant_text_style: Style::default().fg(Color::Gray),
            system_text_style: Style::default().fg(Color::DarkGray),
            error_text_style: Style::default().fg(Color::LightRed),
            app_messages: AppMessageStyles {
                info: AppMessageStyle {
                    prefix: "ℹ️  ".to_string(),
                    prefix_style: Style::default().fg(Color::Rgb(139, 233, 253)),
                    text_style: Style::default().fg(Color::Rgb(189, 246, 255)),
                },
                warning: AppMessageStyle {
                    prefix: "⚠️  ".to_string(),
                    prefix_style: Style::default().fg(Color::Rgb(255, 184, 108)),
                    text_style: Style::default().fg(Color::Rgb(255, 170, 0)),
                },
                error: AppMessageStyle {
                    prefix: "⛔  ".to_string(),
                    prefix_style: Style::default().fg(Color::Rgb(255, 85, 85)),
                    text_style: Style::default().fg(Color::Rgb(255, 121, 121)),
                },
                log: AppMessageStyle {
                    prefix: "📝  ".to_string(),
                    prefix_style: Style::default().fg(Color::Rgb(189, 147, 249)),
                    text_style: Style::default().fg(Color::Rgb(139, 233, 253)),
                },
            },

            title_style: Style::default().fg(Color::LightMagenta),
            streaming_indicator_style: Style::default().fg(Color::LightMagenta),
            selection_highlight_style: Style::default().bg(Color::Rgb(68, 71, 90)),
            input_border_style: Style::default().fg(Color::LightMagenta),
            input_title_style: Style::default().fg(Color::LightMagenta),

            input_text_style: Style::default().fg(Color::White),
            input_cursor_style: Style::default().add_modifier(Modifier::REVERSED),
            input_cursor_line_style: Style::default(),
            input_cursor_color: Some(Self::default_cursor_color_for_bg(Color::Black)),
            md_h1: None,
            md_h2: None,
            md_h3: None,
            md_h4: None,
            md_h5: None,
            md_h6: None,
            md_paragraph: None,
            md_inline_code: None,
            md_link: None,
            md_rule: None,
            md_blockquote_text: None,
            md_list_marker: None,
            md_codeblock_text: None,
            md_codeblock_bg: None,
        }
    }

    pub fn from_name(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "dark" | "default" | "default-dark" => Self::dark_default(),
            "light" => Self::light(),
            "dracula" => Self::dracula(),
            // Fallback
            _ => Self::dark_default(),
        }
    }

    pub fn monochrome() -> Self {
        Theme {
            background_color: Color::Reset,
            user_prefix_style: Style::default().add_modifier(Modifier::BOLD),
            user_text_style: Style::default(),
            assistant_text_style: Style::default(),
            system_text_style: Style::default(),
            error_text_style: Style::default(),
            app_messages: AppMessageStyles {
                info: AppMessageStyle {
                    prefix: "ℹ️  ".to_string(),
                    prefix_style: Style::default(),
                    text_style: Style::default(),
                },
                warning: AppMessageStyle {
                    prefix: "⚠️  ".to_string(),
                    prefix_style: Style::default(),
                    text_style: Style::default(),
                },
                error: AppMessageStyle {
                    prefix: "⛔  ".to_string(),
                    prefix_style: Style::default(),
                    text_style: Style::default(),
                },
                log: AppMessageStyle {
                    prefix: "📝  ".to_string(),
                    prefix_style: Style::default(),
                    text_style: Style::default(),
                },
            },
            title_style: Style::default(),
            streaming_indicator_style: Style::default(),
            selection_highlight_style: Style::default().add_modifier(Modifier::REVERSED),
            input_border_style: Style::default(),
            input_title_style: Style::default(),
            input_text_style: Style::default(),
            input_cursor_style: Style::default().add_modifier(Modifier::REVERSED),
            input_cursor_line_style: Style::default(),
            input_cursor_color: Some(Self::default_cursor_color_for_bg(Color::Reset)),
            md_h1: Some(Style::default().add_modifier(Modifier::BOLD)),
            md_h2: Some(Style::default().add_modifier(Modifier::BOLD)),
            md_h3: Some(Style::default().add_modifier(Modifier::BOLD)),
            md_h4: Some(Style::default().add_modifier(Modifier::BOLD)),
            md_h5: Some(Style::default().add_modifier(Modifier::BOLD)),
            md_h6: Some(Style::default().add_modifier(Modifier::BOLD)),
            md_paragraph: Some(Style::default()),
            md_inline_code: Some(Style::default().add_modifier(Modifier::REVERSED)),
            md_link: Some(Style::default().add_modifier(Modifier::UNDERLINED)),
            md_rule: Some(Style::default()),
            md_blockquote_text: Some(Style::default().add_modifier(Modifier::ITALIC)),
            md_list_marker: Some(Style::default()),
            md_codeblock_text: Some(Style::default()),
            md_codeblock_bg: None,
        }
    }

    pub fn from_spec(spec: &ThemeSpec) -> Self {
        // Helper parsers
        fn parse_color(s: &str) -> Option<Color> {
            let lower = s.trim().to_ascii_lowercase();
            // Hex: #rgb or #rrggbb
            if let Some(c) = parse_hex_color(&lower) {
                return Some(c);
            }
            // rgb(r,g,b)
            if let Some(c) = parse_rgb_func(&lower) {
                return Some(c);
            }
            match lower.as_str() {
                "black" => Some(Color::Black),
                "white" => Some(Color::White),
                "gray" | "grey" => Some(Color::Gray),
                "dark_gray" | "dark-grey" | "darkgray" => Some(Color::DarkGray),
                "red" => Some(Color::Red),
                "light_red" | "light-red" => Some(Color::LightRed),
                "green" => Some(Color::Green),
                "light_green" | "light-green" => Some(Color::LightGreen),
                "blue" => Some(Color::Blue),
                "light_blue" | "light-blue" => Some(Color::LightBlue),
                "cyan" => Some(Color::Cyan),
                "light_cyan" | "light-cyan" => Some(Color::LightCyan),
                "magenta" => Some(Color::Magenta),
                "light_magenta" | "light-magenta" => Some(Color::LightMagenta),
                "yellow" => Some(Color::Yellow),
                "light_yellow" | "light-yellow" => Some(Color::LightYellow),
                "reset" => Some(Color::Reset),
                _ => None,
            }
        }

        fn parse_hex_color(s: &str) -> Option<Color> {
            if !s.starts_with('#') {
                return None;
            }
            let hex = &s[1..];
            if hex.len() == 3 {
                let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
                let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
                let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
                Some(Color::Rgb(r, g, b))
            } else if hex.len() == 6 {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some(Color::Rgb(r, g, b))
            } else {
                None
            }
        }

        fn parse_rgb_func(s: &str) -> Option<Color> {
            // Format: rgb(r,g,b)
            if !s.starts_with("rgb(") || !s.ends_with(')') {
                return None;
            }
            let content = &s[4..s.len() - 1];
            let parts: Vec<_> = content
                .split([',', ' '])
                .filter(|t| !t.is_empty())
                .collect();
            if parts.len() != 3 {
                return None;
            }
            let r = parts[0].parse::<u16>().ok()?;
            let g = parts[1].parse::<u16>().ok()?;
            let b = parts[2].parse::<u16>().ok()?;
            Some(Color::Rgb(
                r.min(255) as u8,
                g.min(255) as u8,
                b.min(255) as u8,
            ))
        }

        fn parse_style(s: &Option<String>) -> Style {
            let mut style = Style::default();
            if let Some(ref spec) = s {
                for tok in spec.split(',').map(|t| t.trim()).filter(|t| !t.is_empty()) {
                    let lower = tok.to_ascii_lowercase();
                    if let Some(rest) = lower.strip_prefix("bg:") {
                        if let Some(color) = parse_color(rest.trim()) {
                            style = style.bg(color);
                            continue;
                        }
                    }
                    if let Some(rest) = lower.strip_prefix("bg(").and_then(|s| s.strip_suffix(')'))
                    {
                        if let Some(color) = parse_color(rest.trim()) {
                            style = style.bg(color);
                            continue;
                        }
                    }
                    if let Some(color) = parse_color(tok) {
                        style = style.fg(color);
                    } else {
                        match lower.as_str() {
                            "bold" => style = style.add_modifier(Modifier::BOLD),
                            "reversed" => style = style.add_modifier(Modifier::REVERSED),
                            "italic" => style = style.add_modifier(Modifier::ITALIC),
                            "underline" | "underlined" => {
                                style = style.add_modifier(Modifier::UNDERLINED)
                            }
                            _ => {}
                        }
                    }
                }
            }
            style
        }

        let background_color = spec
            .background
            .as_deref()
            .and_then(parse_color)
            .unwrap_or(Color::Black);
        let cursor_color = spec
            .cursor_color
            .as_deref()
            .and_then(parse_color)
            .or_else(|| Some(Self::default_cursor_color_for_bg(background_color)));

        let mut app_messages = AppMessageStyles::fallback();
        if let Some(prefix) = &spec.app_info_prefix {
            app_messages.info.prefix = prefix.clone();
        }
        if let Some(style) = &spec.app_info_prefix_style {
            app_messages.info.prefix_style = parse_style(&Some(style.clone()));
        }
        if let Some(style) = &spec.app_info_text {
            app_messages.info.text_style = parse_style(&Some(style.clone()));
        }
        if let Some(prefix) = &spec.app_warning_prefix {
            app_messages.warning.prefix = prefix.clone();
        }
        if let Some(style) = &spec.app_warning_prefix_style {
            app_messages.warning.prefix_style = parse_style(&Some(style.clone()));
        }
        if let Some(style) = &spec.app_warning_text {
            app_messages.warning.text_style = parse_style(&Some(style.clone()));
        }
        if let Some(prefix) = &spec.app_error_prefix {
            app_messages.error.prefix = prefix.clone();
        }
        if let Some(style) = &spec.app_error_prefix_style {
            app_messages.error.prefix_style = parse_style(&Some(style.clone()));
        }
        if let Some(style) = &spec.app_error_text {
            app_messages.error.text_style = parse_style(&Some(style.clone()));
        }
        if let Some(prefix) = &spec.app_log_prefix {
            app_messages.log.prefix = prefix.clone();
        }
        if let Some(style) = &spec.app_log_prefix_style {
            app_messages.log.prefix_style = parse_style(&Some(style.clone()));
        }
        if let Some(style) = &spec.app_log_text {
            app_messages.log.text_style = parse_style(&Some(style.clone()));
        }

        let mut theme = Theme {
            background_color,
            user_prefix_style: parse_style(&spec.user_prefix),
            user_text_style: parse_style(&spec.user_text),
            assistant_text_style: parse_style(&spec.assistant_text),
            system_text_style: parse_style(&spec.system_text),
            error_text_style: Style::default()
                .fg(Self::select_error_color_for_bg(background_color)),
            app_messages,

            title_style: parse_style(&spec.title),
            streaming_indicator_style: parse_style(&spec.streaming_indicator),
            selection_highlight_style: parse_style(&spec.selection_highlight),
            input_border_style: parse_style(&spec.input_border),
            input_title_style: parse_style(&spec.input_title),

            input_text_style: parse_style(&spec.input_text),
            input_cursor_style: {
                let mut s = Style::default();
                if let Some(ref mods) = spec.input_cursor_modifiers {
                    for tok in mods.split(',').map(|t| t.trim()) {
                        match tok.to_ascii_lowercase().as_str() {
                            "bold" => s = s.add_modifier(Modifier::BOLD),
                            "reversed" => s = s.add_modifier(Modifier::REVERSED),
                            "italic" => s = s.add_modifier(Modifier::ITALIC),
                            "underline" | "underlined" => s = s.add_modifier(Modifier::UNDERLINED),
                            _ => {}
                        }
                    }
                }
                s
            },
            input_cursor_line_style: Style::default(),
            input_cursor_color: cursor_color,
            md_h1: spec.md_h1.as_ref().map(|s| parse_style(&Some(s.clone()))),
            md_h2: spec.md_h2.as_ref().map(|s| parse_style(&Some(s.clone()))),
            md_h3: spec.md_h3.as_ref().map(|s| parse_style(&Some(s.clone()))),
            md_h4: spec.md_h4.as_ref().map(|s| parse_style(&Some(s.clone()))),
            md_h5: spec.md_h5.as_ref().map(|s| parse_style(&Some(s.clone()))),
            md_h6: spec.md_h6.as_ref().map(|s| parse_style(&Some(s.clone()))),
            md_paragraph: spec
                .md_paragraph
                .as_ref()
                .map(|s| parse_style(&Some(s.clone()))),
            md_inline_code: spec
                .md_inline_code
                .as_ref()
                .map(|s| parse_style(&Some(s.clone()))),
            md_link: spec.md_link.as_ref().map(|s| parse_style(&Some(s.clone()))),
            md_rule: spec.md_rule.as_ref().map(|s| parse_style(&Some(s.clone()))),
            md_blockquote_text: spec
                .md_blockquote_text
                .as_ref()
                .map(|s| parse_style(&Some(s.clone()))),
            md_list_marker: spec
                .md_list_marker
                .as_ref()
                .map(|s| parse_style(&Some(s.clone()))),
            md_codeblock_text: spec
                .md_codeblock_text
                .as_ref()
                .map(|s| parse_style(&Some(s.clone()))),
            md_codeblock_bg: spec.md_codeblock_bg.as_deref().and_then(parse_color),
        };

        // Fallbacks for markdown styles when not provided
        theme = theme.with_md_fallbacks();
        if theme.selection_highlight_style.bg.is_none() {
            let fallback = Self::default_selection_highlight_for_bg(background_color);
            theme.selection_highlight_style = theme.selection_highlight_style.patch(fallback);
        }
        theme
    }

    pub fn app_message_style(&self, kind: AppMessageKind) -> &AppMessageStyle {
        self.app_messages.style(kind)
    }

    // Choose a readable error color for the given background.
    fn select_error_color_for_bg(bg: Color) -> Color {
        use Color::*;
        match bg {
            // For light backgrounds, prefer deeper red for contrast
            White => Rgb(200, 45, 0),
            Rgb(r, g, b) if Self::is_bright(r, g, b) => Rgb(200, 45, 0),
            // For dark backgrounds, prefer lighter red/orange
            Black | DarkGray => LightRed,
            Rgb(_r, _g, _b) => Rgb(255, 100, 100),
            _ => LightRed,
        }
    }

    fn default_selection_highlight_for_bg(bg: Color) -> Style {
        use Color::*;
        match bg {
            White | LightYellow | LightCyan | LightBlue | LightMagenta | LightGreen | LightRed => {
                Style::default().bg(Color::Rgb(219, 234, 254))
            }
            Gray => Style::default().bg(Color::Rgb(219, 234, 254)),
            Rgb(r, g, b) => {
                if Self::is_bright(r, g, b) {
                    Style::default().bg(Color::Rgb(219, 234, 254))
                } else {
                    Style::default().bg(Color::Rgb(31, 41, 55))
                }
            }
            Indexed(_) => Style::default().bg(Color::Rgb(45, 55, 72)),
            _ => Style::default().bg(Color::Rgb(31, 41, 55)),
        }
    }

    fn is_bright(r: u8, g: u8, b: u8) -> bool {
        // Perceptual luminance approximation
        let lum = 0.2126 * (r as f32) + 0.7152 * (g as f32) + 0.0722 * (b as f32);
        lum >= 140.0
    }

    fn default_cursor_color_for_bg(bg: Color) -> Color {
        if let Some((r, g, b)) = color_to_rgb(bg) {
            if Self::is_bright(r, g, b) {
                Color::Black
            } else {
                Color::White
            }
        } else {
            Color::White
        }
    }

    fn with_md_fallbacks(mut self) -> Self {
        // Headings default to assistant_text_style with bold for H1/H2
        self.md_h1 = self
            .md_h1
            .or(Some(self.assistant_text_style.add_modifier(Modifier::BOLD)));
        self.md_h2 = self
            .md_h2
            .or(Some(self.assistant_text_style.add_modifier(Modifier::BOLD)));
        self.md_h3 = self.md_h3.or(Some(self.assistant_text_style));
        self.md_h4 = self.md_h4.or(self.md_h3);
        self.md_h5 = self.md_h5.or(self.md_h3);
        self.md_h6 = self.md_h6.or(self.md_h3);
        // Paragraph fallback
        self.md_paragraph = self.md_paragraph.or(Some(self.assistant_text_style));
        // Inline code fallback: reversed
        self.md_inline_code = self.md_inline_code.or(Some(
            self.assistant_text_style.add_modifier(Modifier::REVERSED),
        ));
        // Link fallback: underlined assistant text
        self.md_link = self.md_link.or(Some(
            self.assistant_text_style.add_modifier(Modifier::UNDERLINED),
        ));
        // Rule fallback
        self.md_rule = self.md_rule.or(Some(self.input_border_style));
        // Blockquote text fallback
        self.md_blockquote_text = self.md_blockquote_text.or(Some(self.system_text_style));
        // List marker fallback
        self.md_list_marker = self.md_list_marker.or(Some(self.streaming_indicator_style));
        // Code block text fallback
        self.md_codeblock_text = self
            .md_codeblock_text
            .or(Some(self.assistant_text_style.add_modifier(Modifier::DIM)));
        self
    }

    // Public accessors for markdown styles
    pub fn md_heading_style(&self, level: u8) -> Style {
        match level {
            1 => self.md_h1.unwrap_or(self.assistant_text_style),
            2 => self.md_h2.unwrap_or(self.assistant_text_style),
            3 => self.md_h3.unwrap_or(self.assistant_text_style),
            4 => self.md_h4.unwrap_or(self.assistant_text_style),
            5 => self.md_h5.unwrap_or(self.assistant_text_style),
            _ => self.md_h6.unwrap_or(self.assistant_text_style),
        }
    }
    pub fn md_paragraph_style(&self) -> Style {
        self.md_paragraph.unwrap_or(self.assistant_text_style)
    }
    pub fn md_blockquote_style(&self) -> Style {
        self.md_blockquote_text.unwrap_or(self.system_text_style)
    }
    pub fn md_rule_style(&self) -> Style {
        self.md_rule.unwrap_or(self.input_border_style)
    }
    pub fn md_list_marker_style(&self) -> Style {
        self.md_list_marker
            .unwrap_or(self.streaming_indicator_style)
    }
    pub fn md_codeblock_text_style(&self) -> Style {
        self.md_codeblock_text
            .unwrap_or(self.assistant_text_style.add_modifier(Modifier::DIM))
    }
    pub fn md_inline_code_style(&self) -> Style {
        self.md_inline_code
            .unwrap_or(self.assistant_text_style.add_modifier(Modifier::REVERSED))
    }
    pub fn md_link_style(&self) -> Style {
        self.md_link
            .unwrap_or(self.assistant_text_style.add_modifier(Modifier::UNDERLINED))
    }
    pub fn md_codeblock_bg_color(&self) -> Option<Color> {
        self.md_codeblock_bg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_spec() -> ThemeSpec {
        ThemeSpec {
            id: "test".to_string(),
            display_name: "Test".to_string(),
            background: None,
            cursor_color: None,
            user_prefix: None,
            user_text: None,
            assistant_text: None,
            system_text: None,
            app_info_prefix: None,
            app_info_prefix_style: None,
            app_info_text: None,
            app_warning_prefix: None,
            app_warning_prefix_style: None,
            app_warning_text: None,
            app_error_prefix: None,
            app_error_prefix_style: None,
            app_error_text: None,
            app_log_prefix: None,
            app_log_prefix_style: None,
            app_log_text: None,
            title: None,
            streaming_indicator: None,
            selection_highlight: None,
            input_border: None,
            input_title: None,
            input_text: None,
            input_cursor_modifiers: None,
            md_h1: None,
            md_h2: None,
            md_h3: None,
            md_h4: None,
            md_h5: None,
            md_h6: None,
            md_paragraph: None,
            md_inline_code: None,
            md_link: None,
            md_rule: None,
            md_blockquote_text: None,
            md_list_marker: None,
            md_codeblock_text: None,
            md_codeblock_bg: None,
        }
    }

    #[test]
    fn parses_cursor_color_from_spec() {
        let mut spec = base_spec();
        spec.background = Some("#000000".to_string());
        spec.cursor_color = Some("#123456".to_string());

        let theme = Theme::from_spec(&spec);

        assert_eq!(theme.input_cursor_color, Some(Color::Rgb(0x12, 0x34, 0x56)));
    }

    #[test]
    fn defaults_cursor_color_from_background() {
        let mut spec = base_spec();
        spec.background = Some("#ffffff".to_string());

        let theme = Theme::from_spec(&spec);

        assert_eq!(theme.input_cursor_color, Some(Color::Black));
    }
}
