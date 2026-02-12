mod code;
mod lists;
mod metadata;
mod parser;
mod render;
mod table;
#[path = "../markdown_wrap.rs"]
mod wrap;

#[cfg(test)]
pub mod test_fixtures;
#[cfg(test)]
mod tests;

pub use metadata::{RenderedMessage, RenderedMessageDetails};
pub use render::{
    render_message_markdown_details_with_policy_and_user_name, render_message_with_config,
    MessageRenderConfig,
};

#[cfg(test)]
pub use render::build_markdown_display_lines;
