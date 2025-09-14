This repository contains Chabeau, an open source (CC-0) chatbot TUI for remote API-based access.

## General guidelines

- Follow best practices for Rust development and build/test/release processes.
- Ensure new functionality does not introduce usability, security or accessibility regressions.
- Add unit tests opportunistically.
- Update the README.md after you've completed the main objective.
- Always run `cargo test`, `cargo check`, `cargo fmt` and `cargo clippy` after you've completed the main objective.
- Feel free to suggest changes to WISHLIST.md after all these steps have been completed. Thanks for making Chabeau better!
 - Do not add bugfix notes to the README. Keep README focused on user-facing features and behavior; document fixes in commit messages.

## Additional style note for agents
- Do not leave breadcrumb comments about past changes (e.g., "removed", "replaced", or historical notes in code). Keep comments focused on current behavior and intent only.

## Commit summaries
- When asked to draft a commit message, use a conventional commit format. The first line should be 50 characters wide max; subsequent lines 72 characters wide max. Keep it under 5 bullet points. Only summarize uncommitted (staged or working area) changes, not prior versions of those changes (e.g., if you refactored a function, and then refactored it again, only summarize its final state)
- When in interactive mode, present commit summaries to the user in a way that is easy to copy and paste with any formatting (e.g., markdown) intact.
- Do not mention passing tests.

## Performance validations
- When you implement performance-sensitive features (rendering, scrolling, parsing, streaming), validate with Criterion benchmarks in addition to unit tests.
- See `benches/README.md` for a quick scaffold on adding benches locally (and how to export internal modules temporarily via `src/lib.rs`).
- Keep unit-test perf checks green (`cargo test`). Use benches to quantify improvements and regressions (`cargo bench`).
