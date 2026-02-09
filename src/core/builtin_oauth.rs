pub enum OAuthCallbackVariant {
    Success,
    Error,
}

pub fn render_oauth_callback_page(
    title: &str,
    heading: &str,
    detail: &str,
    variant: OAuthCallbackVariant,
) -> String {
    let (accent_light, accent_dark) = match variant {
        OAuthCallbackVariant::Success => ("var(--success-light)", "var(--success-dark)"),
        OAuthCallbackVariant::Error => ("var(--error-light)", "var(--error-dark)"),
    };

    include_str!("../builtins/oauth-callback.html")
        .replace("{{TITLE}}", &escape_html(title))
        .replace("{{HEADING}}", &escape_html(heading))
        .replace("{{DETAIL}}", &escape_html(detail))
        .replace("{{ACCENT_COLOR_LIGHT}}", accent_light)
        .replace("{{ACCENT_COLOR_DARK}}", accent_dark)
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::{render_oauth_callback_page, OAuthCallbackVariant};

    #[test]
    fn render_oauth_callback_page_includes_text_and_accent() {
        let html = render_oauth_callback_page(
            "OAuth complete",
            "You're signed in to Chabeau",
            "Close this tab and return to Chabeau.",
            OAuthCallbackVariant::Success,
        );

        assert!(html.contains("You&#39;re signed in to Chabeau"));
        assert!(html.contains("Close this tab and return to Chabeau."));
        assert!(html.contains("var(--success-light)"));
        assert!(html.contains("var(--success-dark)"));
    }

    #[test]
    fn render_oauth_callback_page_escapes_html() {
        let html = render_oauth_callback_page(
            "<title>",
            "<heading>",
            "\"detail\" & more",
            OAuthCallbackVariant::Error,
        );

        assert!(html.contains("&lt;title&gt;"));
        assert!(html.contains("&lt;heading&gt;"));
        assert!(html.contains("&quot;detail&quot; &amp; more"));
    }
}
