use super::App;

#[derive(Debug, Clone)]
pub struct InspectState {
    pub title: String,
    pub content: String,
    pub scroll_offset: u16,
    pub mode: InspectMode,
}

impl InspectState {
    pub fn new(title: String, content: String) -> Self {
        Self {
            title,
            content,
            scroll_offset: 0,
            mode: InspectMode::Static,
        }
    }

    pub fn with_mode(title: String, content: String, mode: InspectMode) -> Self {
        Self {
            title,
            content,
            scroll_offset: 0,
            mode,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectMode {
    Static,
    ToolCalls {
        index: usize,
        view: ToolInspectView,
        kind: ToolInspectKind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolInspectView {
    Result,
    Request,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolInspectKind {
    Result,
    Pending,
}

impl ToolInspectView {
    pub fn toggle(self) -> Self {
        match self {
            ToolInspectView::Result => ToolInspectView::Request,
            ToolInspectView::Request => ToolInspectView::Result,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct InspectController {
    state: Option<InspectState>,
}

impl InspectController {
    pub fn new() -> Self {
        Self { state: None }
    }

    pub fn state(&self) -> Option<&InspectState> {
        self.state.as_ref()
    }

    pub fn state_mut(&mut self) -> Option<&mut InspectState> {
        self.state.as_mut()
    }

    pub fn open(&mut self, title: String, content: String) {
        self.state = Some(InspectState::new(title, content));
    }

    pub fn open_tool_calls(
        &mut self,
        title: String,
        content: String,
        index: usize,
        view: ToolInspectView,
        kind: ToolInspectKind,
    ) {
        self.state = Some(InspectState::with_mode(
            title,
            content,
            InspectMode::ToolCalls { index, view, kind },
        ));
    }

    pub fn close(&mut self) {
        self.state = None;
    }

    pub fn scroll(&mut self, lines: i32) {
        if let Some(state) = self.state.as_mut() {
            if lines.is_negative() {
                let magnitude = lines.saturating_abs() as u16;
                state.scroll_offset = state.scroll_offset.saturating_sub(magnitude);
            } else {
                let magnitude = lines.saturating_abs() as u16;
                state.scroll_offset = state.scroll_offset.saturating_add(magnitude);
            }
        }
    }

    pub fn scroll_to_start(&mut self) {
        if let Some(state) = self.state.as_mut() {
            state.scroll_offset = 0;
        }
    }

    pub fn scroll_to_end(&mut self) {
        if let Some(state) = self.state.as_mut() {
            state.scroll_offset = u16::MAX;
        }
    }
}

impl App {
    pub fn inspect_state(&self) -> Option<&InspectState> {
        self.inspect.state()
    }

    pub fn inspect_state_mut(&mut self) -> Option<&mut InspectState> {
        self.inspect.state_mut()
    }

    pub fn open_inspect(&mut self, title: String, content: String) {
        self.inspect.open(title, content);
    }

    pub fn open_tool_call_inspect(
        &mut self,
        title: String,
        content: String,
        index: usize,
        view: ToolInspectView,
        kind: ToolInspectKind,
    ) {
        self.inspect
            .open_tool_calls(title, content, index, view, kind);
    }

    pub fn close_inspect(&mut self) {
        self.inspect.close();
    }

    pub fn scroll_inspect(&mut self, lines: i32) {
        self.inspect.scroll(lines);
    }

    pub fn scroll_inspect_to_start(&mut self) {
        self.inspect.scroll_to_start();
    }

    pub fn scroll_inspect_to_end(&mut self) {
        self.inspect.scroll_to_end();
    }
}
