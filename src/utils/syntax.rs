use crate::ui::theme::Theme;
use ratatui::style::Color as TuiColor;
use ratatui::text::{Line, Span};
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

// Simple FIFO cache (bounded) for highlighted blocks
// key = (lang_norm, hash)

fn hash_code(lang: &str, code: &str, theme_sig: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    lang.hash(&mut hasher);
    code.hash(&mut hasher);
    theme_sig.hash(&mut hasher);
    hasher.finish()
}

struct SimpleCache {
    map: HashMap<(String, u64), Vec<Line<'static>>>,
    order: VecDeque<(String, u64)>,
    cap: usize,
}

impl SimpleCache {
    fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            cap,
        }
    }
    fn get(&mut self, k: &(String, u64)) -> Option<Vec<Line<'static>>> {
        self.map.get(k).cloned()
    }
    fn put(&mut self, k: (String, u64), v: Vec<Line<'static>>) {
        if !self.map.contains_key(&k) {
            self.order.push_back(k.clone());
        }
        self.map.insert(k.clone(), v);
        while self.map.len() > self.cap {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            } else {
                break;
            }
        }
    }
}

static SYNTAX_CACHE: Mutex<Option<SimpleCache>> = Mutex::new(None);

fn get_cache() -> std::sync::MutexGuard<'static, Option<SimpleCache>> {
    SYNTAX_CACHE.lock().unwrap()
}

fn ensure_cache(cap: usize) {
    let mut guard = get_cache();
    if guard.is_none() {
        *guard = Some(SimpleCache::new(cap));
    }
}

fn is_dark_background(c: &TuiColor) -> bool {
    match c {
        TuiColor::Rgb(r, g, b) => {
            let br = 0.2126 * (*r as f32) + 0.7152 * (*g as f32) + 0.0722 * (*b as f32);
            br < 128.0
        }
        TuiColor::Black => true,
        TuiColor::White => false,
        TuiColor::Gray | TuiColor::DarkGray => true,
        _ => true,
    }
}

fn normalize_lang_hint(s: &str) -> String {
    let t = s.trim().to_ascii_lowercase();
    match t.as_str() {
        "py" | "python" => "python".into(),
        "bash" | "sh" | "zsh" | "shell" => "bash".into(),
        "js" | "javascript" | "jsx" => "javascript".into(),
        "ts" | "tsx" | "typescript" => "typescript".into(),
        "json" => "json".into(),
        "toml" => "toml".into(),
        "yaml" | "yml" => "yaml".into(),
        "rust" | "rs" => "rust".into(),
        "go" => "go".into(),
        "c" | "h" => "c".into(),
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" => "cpp".into(),
        "java" => "java".into(),
        "kotlin" | "kt" => "kotlin".into(),
        "swift" => "swift".into(),
        "html" => "html".into(),
        "css" => "css".into(),
        "sql" => "sql".into(),
        other => other.into(),
    }
}

fn parse_tui_color_from_syntect(c: syntect::highlighting::Color) -> TuiColor {
    TuiColor::Rgb(c.r, c.g, c.b)
}

// Helper to choose a syntect theme name based on background brightness.
// Kept small and pure for testing.
pub(crate) fn pick_syntect_theme_name_for_theme(theme: &Theme) -> &'static str {
    if is_dark_background(&theme.background_color) {
        "base16-ocean.dark"
    } else {
        "InspiredGitHub"
    }
}

// Helper to build the cache-relevant theme signature.
pub(crate) fn build_theme_signature(theme: &Theme, chosen_syntect: &str) -> String {
    fn color_sig_opt(c: Option<TuiColor>) -> String {
        match c {
            Some(TuiColor::Rgb(r, g, b)) => format!("#{:02x}{:02x}{:02x}", r, g, b),
            Some(other) => format!("{:?}", other),
            None => "none".to_string(),
        }
    }
    format!(
        "{}|{}|{:?}",
        chosen_syntect,
        color_sig_opt(theme.md_codeblock_bg_color()),
        theme.background_color
    )
}

pub fn highlight_code_block(
    lang_hint: &str,
    code: &str,
    theme: &Theme,
) -> Option<Vec<Line<'static>>> {
    // Cache + syntect setup
    ensure_cache(64);
    let lang_norm = normalize_lang_hint(lang_hint);

    // Initialize syntect lazily
    use std::sync::OnceLock;
    static SYNTAX_SET: OnceLock<syntect::parsing::SyntaxSet> = OnceLock::new();
    static THEME_SET: OnceLock<syntect::highlighting::ThemeSet> = OnceLock::new();
    let ps = SYNTAX_SET.get_or_init(syntect::parsing::SyntaxSet::load_defaults_newlines);
    let ts = THEME_SET.get_or_init(syntect::highlighting::ThemeSet::load_defaults);

    // Pick a syntect theme that matches background brightness (higher contrast on light)
    let theme_name = pick_syntect_theme_name_for_theme(theme);
    let fallback_names = [
        "base16-ocean.light",
        "Solarized (light)",
        "base16-ocean.dark",
    ];
    let mut syn_theme = ts.themes.get(theme_name);
    if syn_theme.is_none() {
        for name in &fallback_names {
            if let Some(th) = ts.themes.get(*name) {
                syn_theme = Some(th);
                break;
            }
        }
    }
    let syn_theme = syn_theme?;

    // Build a theme signature so cache respects theme changes
    let theme_sig = build_theme_signature(theme, theme_name);
    let key = (lang_norm.clone(), hash_code(&lang_norm, code, &theme_sig));
    if let Some(lines) = get_cache().as_mut().and_then(|c| c.get(&key)) {
        return Some(lines);
    }

    // Find syntax
    let syntax = ps
        .find_syntax_by_token(&lang_norm)
        .unwrap_or_else(|| ps.find_syntax_plain_text());

    let mut h = syntect::easy::HighlightLines::new(syntax, syn_theme);
    let bg = theme.md_codeblock_bg_color();

    let mut out: Vec<Line<'static>> = Vec::new();
    for line in syntect::util::LinesWithEndings::from(code) {
        let ranges = h.highlight_line(line, ps).ok()?;
        let mut spans: Vec<Span<'static>> = Vec::new();
        for (style, text) in ranges {
            // strip trailing newline from the fragment before rendering in a Line
            let mut frag = text;
            if let Some(stripped) = frag.strip_suffix('\n') {
                frag = stripped;
            }
            let mut st =
                ratatui::style::Style::default().fg(parse_tui_color_from_syntect(style.foreground));
            if let Some(bgcol) = bg {
                st = st.bg(bgcol);
            }
            spans.push(Span::styled(frag.to_string(), st));
        }
        if spans.is_empty() {
            out.push(Line::from(""));
        } else {
            out.push(Line::from(spans));
        }
    }

    // Cache result
    {
        let mut guard = get_cache();
        if let Some(cache) = guard.as_mut() {
            cache.put(key, out.clone());
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn normalize_lang_hint_maps_common_aliases() {
        assert_eq!(normalize_lang_hint("py"), "python");
        assert_eq!(normalize_lang_hint("JS"), "javascript");
        assert_eq!(normalize_lang_hint("TsX"), "typescript");
        assert_eq!(normalize_lang_hint("yml"), "yaml");
        assert_eq!(normalize_lang_hint("hpp"), "cpp");
        assert_eq!(normalize_lang_hint("rs"), "rust");
    }

    #[test]
    fn dark_background_heuristic_basic() {
        assert!(is_dark_background(&Color::Black));
        assert!(!is_dark_background(&Color::White));
        assert!(is_dark_background(&Color::Rgb(10, 10, 10)));
        assert!(!is_dark_background(&Color::Rgb(240, 240, 240)));
    }

    #[test]
    fn theme_selection_matches_brightness() {
        let mut dark = crate::ui::theme::Theme::dark_default();
        dark.background_color = Color::Rgb(10, 10, 10);
        let mut light = crate::ui::theme::Theme::light();
        light.background_color = Color::Rgb(245, 245, 245);
        assert_eq!(
            pick_syntect_theme_name_for_theme(&dark),
            "base16-ocean.dark"
        );
        assert_eq!(pick_syntect_theme_name_for_theme(&light), "InspiredGitHub");
    }

    #[test]
    fn theme_signature_changes_with_theme() {
        let mut dark = crate::ui::theme::Theme::dark_default();
        dark.background_color = Color::Rgb(10, 10, 10);
        let mut light = crate::ui::theme::Theme::light();
        light.background_color = Color::Rgb(245, 245, 245);
        // Also vary codeblock bg to ensure it’s captured
        dark.md_codeblock_bg = Some(Color::Rgb(30, 30, 30));
        light.md_codeblock_bg = Some(Color::Rgb(230, 230, 230));

        let s1 = build_theme_signature(&dark, pick_syntect_theme_name_for_theme(&dark));
        let s2 = build_theme_signature(&light, pick_syntect_theme_name_for_theme(&light));
        assert_ne!(s1, s2);
    }
}
