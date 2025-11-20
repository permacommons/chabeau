//! Test fixtures for code block span metadata testing.
//!
//! Provides canonical test cases covering edge cases for code block
//! rendering and metadata extraction.

use crate::core::message::{Message, ROLE_ASSISTANT, ROLE_USER};

/// Test fixture: single code block in assistant message.
///
/// Tests basic code block rendering with a language tag.
pub fn single_block() -> Message {
    Message {
        role: ROLE_ASSISTANT.to_string(),
        content: "Here's a function:\n\n```rust\nfn main() {}\n```\n".to_string(),
    }
}

/// Test fixture: multiple code blocks with different languages.
///
/// Tests block index assignment and language tracking across
/// multiple blocks in a single message.
pub fn multiple_blocks() -> Message {
    Message {
        role: ROLE_ASSISTANT.to_string(),
        content: concat!(
            "First, here's some Rust:\n\n",
            "```rust\nfn main() {\n    println!(\"Hello\");\n}\n```\n\n",
            "And some Python:\n\n",
            "```python\ndef greet():\n    print(\"Hello\")\n```\n\n",
            "Finally, plain text:\n\n",
            "```\nno language tag\n```\n"
        )
        .to_string(),
    }
}

/// Test fixture: code blocks across multiple messages.
///
/// Tests that block indices are per-message, not global.
pub fn blocks_across_messages() -> Vec<Message> {
    vec![
        Message {
            role: ROLE_USER.to_string(),
            content: "Show me Rust code".to_string(),
        },
        Message {
            role: ROLE_ASSISTANT.to_string(),
            content: "```rust\nfn first() {}\n```".to_string(),
        },
        Message {
            role: ROLE_USER.to_string(),
            content: "And Python?".to_string(),
        },
        Message {
            role: ROLE_ASSISTANT.to_string(),
            content: "```python\ndef second():\n    pass\n```".to_string(),
        },
    ]
}

/// Test fixture: code block in ordered list with indentation.
///
/// Tests that nested code blocks within list items are properly
/// tracked with metadata.
pub fn nested_in_list() -> Message {
    Message {
        role: ROLE_ASSISTANT.to_string(),
        content: concat!(
            "1. First step\n\n",
            "   ```rust\n",
            "   fn step_one() {}\n",
            "   ```\n\n",
            "2. Second step\n\n",
            "   ```rust\n",
            "   fn step_two() {}\n",
            "   ```\n"
        )
        .to_string(),
    }
}

/// Test fixture: code block with long lines requiring wrapping.
///
/// Tests that metadata is preserved across wrapped lines.
pub fn wrapped_code() -> Message {
    Message {
        role: ROLE_ASSISTANT.to_string(),
        content: concat!(
            "```rust\n",
            "fn very_long_function_name_that_will_definitely_wrap_on_narrow_terminals() {\n",
            "    let also_a_very_long_variable_name_that_exceeds_typical_terminal_width = 42;\n",
            "}\n",
            "```\n"
        )
        .to_string(),
    }
}

/// Test fixture: empty code block (edge case).
///
/// Tests handling of code blocks with no content.
pub fn empty_block() -> Message {
    Message {
        role: ROLE_ASSISTANT.to_string(),
        content: "Here's an empty block:\n\n```\n```\n\nDone.".to_string(),
    }
}

/// Test fixture: code block without language tag.
///
/// Tests that blocks without language tags still get metadata.
pub fn no_language_tag() -> Message {
    Message {
        role: ROLE_ASSISTANT.to_string(),
        content: "```\nplain code\nno language\n```".to_string(),
    }
}

/// Test fixture: code block immediately adjacent to text (no newlines).
///
/// Tests parsing when there's no whitespace around code blocks.
pub fn adjacent_to_text() -> Message {
    Message {
        role: ROLE_ASSISTANT.to_string(),
        content: "Before```rust\nfn adjacent() {}\n```After".to_string(),
    }
}

/// Test fixture: code block in user message.
///
/// Tests that user messages render code blocks with metadata.
pub fn user_message_with_code() -> Message {
    Message {
        role: ROLE_USER.to_string(),
        content: "Can you explain this?\n\n```python\ndef mystery():\n    pass\n```".to_string(),
    }
}

/// Test fixture: mixed content with code and links.
///
/// Tests that code block metadata coexists with link metadata.
pub fn code_and_links() -> Message {
    Message {
        role: ROLE_ASSISTANT.to_string(),
        content: concat!(
            "Check [the docs](https://example.com) for details:\n\n",
            "```rust\nfn example() {}\n```\n\n",
            "See also [this guide](https://example.org)."
        )
        .to_string(),
    }
}

/// Test fixture: code block with various language tags.
///
/// Tests language tag handling for common languages.
pub fn various_languages() -> Message {
    Message {
        role: ROLE_ASSISTANT.to_string(),
        content: concat!(
            "```bash\necho 'hello'\n```\n\n",
            "```javascript\nconsole.log('hi');\n```\n\n",
            "```json\n{\"key\": \"value\"}\n```\n\n",
            "```txt\nplain text\n```\n"
        )
        .to_string(),
    }
}
