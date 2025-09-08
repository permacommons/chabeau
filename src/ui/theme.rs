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

        Theme {
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
                            _ => {}
                        }
                    }
                }
                s
            },
            input_cursor_line_style: Style::default(),
        }
    }
}
