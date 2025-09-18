use ratatui::layout::Rect;

#[derive(Clone, Debug)]
pub struct LinkHotspot {
    pub url: String,
    pub rect: Rect,
}

#[derive(Clone, Debug)]
pub struct UrlOverlay {
    pub url: String,
    pub rect: Rect,
}
