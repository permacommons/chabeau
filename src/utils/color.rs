use ratatui::style::{Color, Style};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorDepth {
    Truecolor,
    X256,
    X16,
}

/// Detect terminal color depth from environment.
/// Priority: COLORTERM truecolor/24bit -> TERM *256color -> fallback 16.
pub fn detect_color_depth() -> ColorDepth {
    // Allow override for testing/advanced users
    if let Ok(force) = std::env::var("CHABEAU_COLOR") {
        match force.trim().to_ascii_lowercase().as_str() {
            "truecolor" | "24bit" | "24-bit" => return ColorDepth::Truecolor,
            "256" | "x256" | "256color" => return ColorDepth::X256,
            "16" | "ansi" | "x16" => return ColorDepth::X16,
            _ => {}
        }
    }

    if let Ok(colorterm) = std::env::var("COLORTERM") {
        let s = colorterm.to_ascii_lowercase();
        if s.contains("truecolor") || s.contains("24bit") || s.contains("24-bit") {
            return ColorDepth::Truecolor;
        }
    }
    if let Ok(term) = std::env::var("TERM") {
        let s = term.to_ascii_lowercase();
        if s.contains("256color") {
            return ColorDepth::X256;
        }
    }
    ColorDepth::X16
}

/// Map a Color to the nearest representable color in the chosen depth.
pub fn quantize_color(color: Color, depth: ColorDepth) -> Color {
    match depth {
        ColorDepth::Truecolor => color,
        ColorDepth::X256 => quantize_color_256(color),
        ColorDepth::X16 => quantize_color_16(color),
    }
}

pub fn quantize_style(mut style: Style, depth: ColorDepth) -> Style {
    if let Some(fg) = style.fg {
        style.fg = Some(quantize_color(fg, depth));
    }
    if let Some(bg) = style.bg {
        style.bg = Some(quantize_color(bg, depth));
    }
    if let Some(uc) = style.underline_color {
        style.underline_color = Some(quantize_color(uc, depth));
    }
    style
}

pub fn quantize_theme_if_needed(
    mut theme: crate::ui::theme::Theme,
    depth: ColorDepth,
) -> crate::ui::theme::Theme {
    if depth == ColorDepth::Truecolor {
        return theme;
    }
    theme.background_color = quantize_color(theme.background_color, depth);
    theme.user_prefix_style = quantize_style(theme.user_prefix_style, depth);
    theme.user_text_style = quantize_style(theme.user_text_style, depth);
    theme.assistant_text_style = quantize_style(theme.assistant_text_style, depth);
    theme.system_text_style = quantize_style(theme.system_text_style, depth);
    theme.error_text_style = quantize_style(theme.error_text_style, depth);

    theme.title_style = quantize_style(theme.title_style, depth);
    theme.streaming_indicator_style = quantize_style(theme.streaming_indicator_style, depth);
    theme.selection_highlight_style = quantize_style(theme.selection_highlight_style, depth);
    theme.input_border_style = quantize_style(theme.input_border_style, depth);
    theme.input_title_style = quantize_style(theme.input_title_style, depth);

    theme.input_text_style = quantize_style(theme.input_text_style, depth);
    theme.input_cursor_style = quantize_style(theme.input_cursor_style, depth);
    theme.input_cursor_line_style = quantize_style(theme.input_cursor_line_style, depth);

    theme.md_h1 = theme.md_h1.map(|s| quantize_style(s, depth));
    theme.md_h2 = theme.md_h2.map(|s| quantize_style(s, depth));
    theme.md_h3 = theme.md_h3.map(|s| quantize_style(s, depth));
    theme.md_h4 = theme.md_h4.map(|s| quantize_style(s, depth));
    theme.md_h5 = theme.md_h5.map(|s| quantize_style(s, depth));
    theme.md_h6 = theme.md_h6.map(|s| quantize_style(s, depth));
    theme.md_paragraph = theme.md_paragraph.map(|s| quantize_style(s, depth));
    theme.md_inline_code = theme.md_inline_code.map(|s| quantize_style(s, depth));
    theme.md_link = theme.md_link.map(|s| quantize_style(s, depth));
    theme.md_rule = theme.md_rule.map(|s| quantize_style(s, depth));
    theme.md_blockquote_text = theme.md_blockquote_text.map(|s| quantize_style(s, depth));
    theme.md_list_marker = theme.md_list_marker.map(|s| quantize_style(s, depth));
    theme.md_codeblock_text = theme.md_codeblock_text.map(|s| quantize_style(s, depth));
    theme.md_codeblock_bg = theme.md_codeblock_bg.map(|c| quantize_color(c, depth));

    theme
}

/// Convenience: quantize a theme for the current terminal's color depth.
pub fn quantize_theme_for_current_terminal(
    theme: crate::ui::theme::Theme,
) -> crate::ui::theme::Theme {
    let depth = detect_color_depth();
    quantize_theme_if_needed(theme, depth)
}

fn quantize_color_256(color: Color) -> Color {
    match color {
        Color::Rgb(r, g, b) => Color::Indexed(rgb_to_xterm256(r, g, b)),
        // Keep named and indexed as-is
        other => other,
    }
}

fn quantize_color_16(color: Color) -> Color {
    match color {
        Color::Rgb(r, g, b) => nearest_ansi16_from_rgb(r, g, b),
        Color::Indexed(i) => {
            let (r, g, b) = xterm256_to_rgb(i);
            nearest_ansi16_from_rgb(r, g, b)
        }
        other => other,
    }
}

fn nearest_ansi16_from_rgb(r: u8, g: u8, b: u8) -> Color {
    // Define 16-color palette approximations (RGB) and mapping to ratatui Color
    // 0..7 standard, 8..15 bright
    const ANSI16: &[(u8, u8, u8, Color); 16] = &[
        (0, 0, 0, Color::Black),            // 0 Black
        (205, 0, 0, Color::Red),            // 1 Red
        (0, 205, 0, Color::Green),          // 2 Green
        (205, 205, 0, Color::Yellow),       // 3 Yellow
        (0, 0, 205, Color::Blue),           // 4 Blue
        (205, 0, 205, Color::Magenta),      // 5 Magenta
        (0, 205, 205, Color::Cyan),         // 6 Cyan
        (192, 192, 192, Color::Gray),       // 7 Light gray
        (128, 128, 128, Color::DarkGray),   // 8 Dark gray (bright black)
        (255, 0, 0, Color::LightRed),       // 9 Bright red
        (0, 255, 0, Color::LightGreen),     // 10 Bright green
        (255, 255, 0, Color::LightYellow),  // 11 Bright yellow
        (92, 92, 255, Color::LightBlue),    // 12 Bright blue
        (255, 0, 255, Color::LightMagenta), // 13 Bright magenta
        (0, 255, 255, Color::LightCyan),    // 14 Bright cyan
        (255, 255, 255, Color::White),      // 15 Bright white
    ];

    let mut best = 0usize;
    let mut best_dist = u32::MAX;
    for (i, &(rr, gg, bb, _)) in ANSI16.iter().enumerate() {
        let dr = rr as i32 - r as i32;
        let dg = gg as i32 - g as i32;
        let db = bb as i32 - b as i32;
        let dist = (dr * dr + dg * dg + db * db) as u32;
        if dist < best_dist {
            best_dist = dist;
            best = i;
        }
    }
    ANSI16[best].3
}

fn rgb_to_xterm256(r: u8, g: u8, b: u8) -> u8 {
    // Try mapping to color cube 6x6x6 first, then grayscale, pick nearest overall
    let cube_index = rgb_to_xterm_cube_index(r, g, b);
    let (cr, cg, cb) = xterm256_to_rgb(cube_index);
    let cube_dist = color_dist_sq(r, g, b, cr, cg, cb);

    let gray_index = rgb_to_xterm_gray_index(r, g, b);
    let (gr, gg, gb) = xterm256_to_rgb(gray_index);
    let gray_dist = color_dist_sq(r, g, b, gr, gg, gb);

    if gray_dist < cube_dist {
        gray_index
    } else {
        cube_index
    }
}

fn rgb_to_xterm_cube_index(r: u8, g: u8, b: u8) -> u8 {
    fn map_comp(c: u8) -> u8 {
        if c < 48 {
            0
        } else if c < 114 {
            1
        } else {
            ((c - 35) / 40).min(5)
        }
    }
    let ri = map_comp(r);
    let gi = map_comp(g);
    let bi = map_comp(b);
    16 + 36 * ri + 6 * gi + bi
}

fn rgb_to_xterm_gray_index(r: u8, g: u8, b: u8) -> u8 {
    let avg = (r as u16 + g as u16 + b as u16) / 3;
    // Map [0,255] to grayscale 232..255 with thresholds around midpoints
    let idx = if avg <= 3 {
        16
    } else {
        // black corner case prefers cube sometimes
        ((avg.saturating_sub(8)) / 10) as u8
    };
    let idx = idx.min(23);
    232 + idx
}

fn color_dist_sq(r1: u8, g1: u8, b1: u8, r2: u8, g2: u8, b2: u8) -> u32 {
    let dr = r1 as i32 - r2 as i32;
    let dg = g1 as i32 - g2 as i32;
    let db = b1 as i32 - b2 as i32;
    (dr * dr + dg * dg + db * db) as u32
}

fn xterm_cube_comp(i: u8) -> u8 {
    if i == 0 {
        0
    } else {
        55 + 40 * i
    }
}

pub fn xterm256_to_rgb(i: u8) -> (u8, u8, u8) {
    match i {
        0 => (0, 0, 0),
        1 => (205, 0, 0),
        2 => (0, 205, 0),
        3 => (205, 205, 0),
        4 => (0, 0, 205),
        5 => (205, 0, 205),
        6 => (0, 205, 205),
        7 => (229, 229, 229),
        8 => (127, 127, 127),
        9 => (255, 0, 0),
        10 => (0, 255, 0),
        11 => (255, 255, 0),
        12 => (92, 92, 255),
        13 => (255, 0, 255),
        14 => (0, 255, 255),
        15 => (255, 255, 255),
        16..=231 => {
            let mut n = i - 16;
            let r = n / 36;
            n %= 36;
            let g = n / 6;
            n %= 6;
            let b = n;
            (xterm_cube_comp(r), xterm_cube_comp(g), xterm_cube_comp(b))
        }
        232..=255 => {
            let v = 8 + 10 * (i - 232);
            (v, v, v)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_truecolor_from_env() {
        std::env::set_var("COLORTERM", "truecolor");
        assert_eq!(detect_color_depth(), ColorDepth::Truecolor);
        std::env::remove_var("COLORTERM");
    }

    #[test]
    fn detects_256_from_term() {
        // Ensure COLORTERM doesn't force truecolor in this environment
        std::env::remove_var("COLORTERM");
        std::env::set_var("TERM", "xterm-256color");
        assert_eq!(detect_color_depth(), ColorDepth::X256);
        std::env::remove_var("TERM");
    }

    #[test]
    fn quantize_rgb_to_256_index() {
        let idx = rgb_to_xterm256(255, 0, 0);
        // Should be a bright red close to 9 or in the color cube
        assert!(idx == 9 || (16..=231).contains(&idx));
    }

    #[test]
    fn quantize_rgb_to_ansi16() {
        let c = nearest_ansi16_from_rgb(250, 10, 10);
        // Should map to a red variant
        assert!(matches!(c, Color::Red | Color::LightRed));
    }
}
