use crate::ui::builtin_themes::ThemeSpec;
use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone)]
pub struct Theme {
    // Overall background color to paint the full frame
    pub background_color: Color,
    // Chat message styles
    pub user_prefix_style: Style,
    pub user_text_style: Style,
    pub assistant_text_style: Style,
    pub system_text_style: Style,

    // Chrome
    pub title_style: Style,
    pub streaming_indicator_style: Style,
    pub input_border_style: Style,
    pub input_title_style: Style,

    // Input area
    pub input_text_style: Style,
    pub input_cursor_style: Style,
    pub input_cursor_line_style: Style,

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

            title_style: Style::default().fg(Color::Gray),
            streaming_indicator_style: Style::default().fg(Color::White),
            input_border_style: Style::default().fg(Color::Gray),
            input_title_style: Style::default().fg(Color::Gray),

            input_text_style: Style::default().fg(Color::White),
            input_cursor_style: Style::default().add_modifier(Modifier::REVERSED),
            input_cursor_line_style: Style::default(),
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

            title_style: Style::default().fg(Color::DarkGray),
            streaming_indicator_style: Style::default().fg(Color::Black),
            input_border_style: Style::default().fg(Color::Black),
            input_title_style: Style::default().fg(Color::DarkGray),

            input_text_style: Style::default().fg(Color::Black),
            input_cursor_style: Style::default().add_modifier(Modifier::REVERSED),
            input_cursor_line_style: Style::default(),
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

            title_style: Style::default().fg(Color::LightMagenta),
            streaming_indicator_style: Style::default().fg(Color::LightMagenta),
            input_border_style: Style::default().fg(Color::LightMagenta),
            input_title_style: Style::default().fg(Color::LightMagenta),

            input_text_style: Style::default().fg(Color::White),
            input_cursor_style: Style::default().add_modifier(Modifier::REVERSED),
            input_cursor_line_style: Style::default(),
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
                    if let Some(color) = parse_color(tok) {
                        style = style.fg(color);
                    } else {
                        match tok {
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

        let mut theme = Theme {
            background_color,
            user_prefix_style: parse_style(&spec.user_prefix),
            user_text_style: parse_style(&spec.user_text),
            assistant_text_style: parse_style(&spec.assistant_text),
            system_text_style: parse_style(&spec.system_text),

            title_style: parse_style(&spec.title),
            streaming_indicator_style: parse_style(&spec.streaming_indicator),
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
        theme
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
