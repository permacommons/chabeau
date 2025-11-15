# Chabeau Code Documentation Sprint Plan

**Created**: 2025-11-15
**Status**: Draft
**Effort Estimate**: 12-16 hours (can be parallelized across modules)

## Executive Summary

Current documentation coverage is estimated at **28-32%** of the public API. While the codebase demonstrates excellent test coverage and logical structure, critical public APIs lack documentation, forcing users to read implementation code to understand functionality.

**Target**: Achieve **70%+ documentation coverage** with all public APIs documented according to Rust and industry best practices.

---

## Current State Analysis

### Documentation Coverage Breakdown

| Category | Current | Target | Gap |
|----------|---------|--------|-----|
| Module-level (//!) docs | 14% (13/95 files) | 90%+ | High |
| Public struct/fn docs | ~32% | 95%+ | High |
| Enum variant docs | <5% | 100% | Critical |
| Doc examples | <1% | 50%+ | Critical |
| Inline comments | 40% | 60% | Medium |
| Cross-doc links | 0% | 40%+ | High |

### Strengths to Preserve

1. **Excellent Test Coverage** (~90% of functions have tests)
   - Tests demonstrate functionality effectively
   - Serve as executable documentation
   - Examples: `src/core/chat_stream.rs` has 76 lines of detailed tests

2. **Good Inline Comments for Complexity**
   - Render logic has excellent explanatory comments
   - Complex algorithms are well-explained
   - Example: `src/ui/renderer.rs` has 15+ lines explaining rendering logic

3. **Clear Code Structure**
   - Module organization is logical
   - Function names are self-documenting
   - API boundaries are well-defined

4. **Some High-Quality Examples**
   - `AuthManager::resolve_authentication()` has excellent multi-step documentation
   - CLI module has good structural documentation

### Critical Gaps

#### 1. Core Modules Completely Undocumented

**High Impact, High Priority**

- `src/core/app/mod.rs` - Application state core (0% documented)
  - `App` struct: undocumented
  - `AppInitConfig`: undocumented
  - `new_with_auth()`, `new_uninitialized()`: undocumented

- `src/character/service.rs` - Character management (0% documented)
  - `CharacterService`: undocumented
  - All public methods undocumented
  - Error types lack explanatory docs

- `src/core/chat_stream.rs` - SSE streaming core (0% documented)
  - `ChatStreamService`: undocumented
  - `StreamParams`: undocumented
  - `StreamMessage` enum: undocumented

#### 2. Enum Variants Without Explanation

**Medium Impact, High Priority**

- `CommandResult` (10 variants, 0 documented) - users cannot understand variant purposes
- `StreamMessage` (4 variants, 0 documented)
- Various error enums lack variant-specific documentation

#### 3. Missing Module Overviews

**Medium Impact, Medium Priority**

- 82 of 95 files (86%) lack module-level `//!` documentation
- No architectural context for developers
- No guidance on public API usage patterns

#### 4. No Usage Examples in Docs

**High Impact for Users, Medium Priority**

- Zero `///` doc comments contain executable examples
- Users must search tests to understand API usage
- No quick-start guidance in public API docs

#### 5. Async/Error Documentation Gaps

**Medium Impact, Medium Priority**

- Async functions lack behavior documentation
- Cancellation token usage not explained
- Error types not documented on Result-returning functions

---

## Rust Documentation Best Practices Reference

### Official Rust Standards (rustdoc)

#### Documentation Comment Types

```rust
//! Module-level documentation (inner doc comment)
//! Use at the top of files to explain the module's purpose

/// Item-level documentation (outer doc comment)
/// Use before functions, structs, enums, etc.
```

#### Required Structure for Public Items

1. **Summary Sentence** (mandatory)
   - Single line before first blank line
   - ~15 words maximum (Microsoft guideline)
   - Appears in search results and module overviews
   - Concise, non-technical where possible

2. **Extended Documentation** (strongly encouraged)
   - Detailed explanation of behavior
   - Context and use cases
   - Edge cases and limitations

3. **Examples** (strongly encouraged)
   - At least one copyable, runnable example
   - Examples are tested by `cargo test`
   - Show typical usage patterns

4. **Special Sections** (when applicable)
   - `# Panics` - When and why the code panics
   - `# Errors` - Error conditions for Result-returning functions
   - `# Safety` - Invariants for unsafe code
   - `# Examples` - Code examples (tested automatically)

#### Markdown Support

Rustdoc uses CommonMark with extensions:
- Code blocks with syntax highlighting: ` ```rust `
- Inline code: `` `code` ``
- Links to other items: `` [`Type`] ``, `` [`module::function`] ``
- External links: `[text](url)`
- Lists, tables, footnotes
- Warning blocks: `<div class="warning">...</div>`

### Microsoft Enterprise Guidelines

**M-FIRST-DOC-SENTENCE**: Opening sentence ≤15 words, single line
**M-MODULE-DOCS**: All public modules require `//!` documentation
**M-CANONICAL-DOCS**: Include all applicable sections (Summary, Examples, Errors, Panics, Safety)
**M-DOC-INLINE**: Use `#[doc(inline)]` for re-exported internal items

**Anti-pattern**: "Do not create a table of parameters. Instead parameter use is explained in plain text."

---

## Sprint Work Breakdown

### Phase 1: Quick Wins (4-5 hours)

**Goal**: Establish baseline documentation for all modules and critical enums

#### Task 1.1: Module-Level Documentation (2-3 hours)

Add `//!` comments to all public modules. Template:

```rust
//! Brief one-line description of module purpose.
//!
//! Extended explanation of what this module provides, its role in
//! the system, and when/how to use it.
//!
//! # Examples
//!
//! ```rust
//! // Quick usage example
//! ```
```

**Priority Modules** (alphabetically):
- [ ] `src/auth/mod.rs` - Authentication and provider management
- [ ] `src/character/service.rs` - Character card loading and caching
- [ ] `src/cli/mod.rs` - ✅ Already documented
- [ ] `src/commands/mod.rs` - Slash command processing
- [ ] `src/core/app/mod.rs` - Core application state
- [ ] `src/core/app/session.rs` - Session context and metadata
- [ ] `src/core/chat_stream.rs` - SSE streaming pipeline
- [ ] `src/core/config/data.rs` - Configuration structures
- [ ] `src/core/providers.rs` - Provider metadata
- [ ] `src/ui/chat_loop/mod.rs` - ✅ Already documented
- [ ] `src/ui/chat_loop/event_loop.rs` - Event handling
- [ ] `src/ui/renderer.rs` - Terminal UI rendering

**Estimated**: ~15-20 minutes per module = 2-3 hours total

#### Task 1.2: Document All Public Enums (1-2 hours)

Add documentation to all enum variants with purpose and usage.

**Critical Enums**:
- [ ] `CommandResult` (src/commands/mod.rs) - 10 variants, user-facing
- [ ] `StreamMessage` (src/core/chat_stream.rs) - 4 variants
- [ ] `CharacterServiceError` (src/character/service.rs)
- [ ] All other public enums in API surface

Template:
```rust
/// Result of processing a command input.
///
/// Variants indicate how the UI should respond to command execution.
pub enum CommandResult {
    /// Continue without action (command handled internally).
    Continue,

    /// Continue and focus the transcript area.
    ContinueWithTranscriptFocus,

    /// Process the string as a chat message to the model.
    ProcessAsMessage(String),

    // ... etc
}
```

**Estimated**: 1-2 hours

---

### Phase 2: Core Public API Documentation (5-6 hours)

**Goal**: Document all public structs, functions, and methods on critical modules

#### Task 2.1: Core Application Module (1.5 hours)

File: `src/core/app/mod.rs`

- [ ] `App` struct - document fields and purpose
- [ ] `AppInitConfig` struct - document all configuration fields
- [ ] `new_with_auth()` - document parameters, return value, errors
- [ ] `new_uninitialized()` - document use case vs `new_with_auth()`
- [ ] All other public methods on `App`

Example:
```rust
/// Creates a new authenticated application instance.
///
/// This initializes the full application state including personas, presets,
/// and character configuration. Use this for normal interactive sessions.
///
/// # Arguments
///
/// * `config` - Application initialization configuration
///
/// # Errors
///
/// Returns an error if persona/preset loading fails or character resolution
/// encounters an error.
///
/// # Examples
///
/// ```rust
/// let config = AppInitConfig { /* ... */ };
/// let app = App::new_with_auth(config)?;
/// ```
pub fn new_with_auth(config: AppInitConfig) -> Result<Self, Box<dyn std::error::Error>>
```

#### Task 2.2: Character Service (1.5 hours)

File: `src/character/service.rs`

- [ ] `CharacterService` struct
- [ ] `new()` - document caching behavior
- [ ] `resolve()` - document input format expectations
- [ ] `resolve_by_name()` - document name resolution strategy
- [ ] `list_metadata()` - document what metadata is returned
- [ ] `load_default_for_session()` - document fallback behavior
- [ ] `CharacterServiceError` variants

#### Task 2.3: Chat Streaming Service (1.5 hours)

File: `src/core/chat_stream.rs`

- [ ] `ChatStreamService` struct and channel semantics
- [ ] `StreamParams` struct fields
- [ ] `StreamMessage` enum (if not done in Phase 1)
- [ ] `new()` - document channel setup
- [ ] `start_stream()` - document async behavior, cancellation
- [ ] SSE framing traits and implementations

#### Task 2.4: Authentication Manager (1 hour)

File: `src/auth/mod.rs`

- [ ] `AuthManager` struct
- [ ] `Provider` struct fields
- [ ] Methods without docs: `new()`, `store_token()`, `get_token()`, etc.
- [ ] Expand `resolve_authentication()` example (already has good docs)

#### Task 2.5: Commands Module (0.5 hours)

File: `src/commands/mod.rs`

- [ ] `process_input()` - document command parsing and routing
- [ ] Major command handlers (help, clear, save, load, etc.)
- [ ] `dump_conversation_with_overwrite()` - document behavior

---

### Phase 3: Examples and Advanced Documentation (3-4 hours)

**Goal**: Add executable examples and cross-references

#### Task 3.1: Add Examples to Core Public APIs (2-3 hours)

Add `# Examples` sections to:
- [ ] `App::new_with_auth()`
- [ ] `CharacterService::resolve()`
- [ ] `ChatStreamService::start_stream()`
- [ ] `AuthManager::resolve_authentication()`
- [ ] `process_input()` (command handling)

Ensure examples:
- Compile and pass `cargo test --doc`
- Show realistic usage patterns
- Are copyable and runnable

#### Task 3.2: Add Cross-Documentation Links (1 hour)

Link related items using `` [`Type`] `` syntax:
- [ ] Link `App` to `SessionContext`
- [ ] Link `CharacterService` to `CharacterCard`
- [ ] Link command handlers to `CommandResult`
- [ ] Link `AuthManager` to `Provider`
- [ ] Add "See also" sections where relevant

Example:
```rust
/// Creates a new chat stream.
///
/// See also: [`StreamParams`] for configuration options,
/// [`StreamMessage`] for message types received.
```

---

### Phase 4: Polish and Verify (1 hour)

#### Task 4.1: Run Documentation Tests (15 min)

```bash
cargo test --doc
```

Fix any failing examples.

#### Task 4.2: Generate and Review HTML Docs (30 min)

```bash
cargo doc --no-deps --open
```

Review generated documentation for:
- Broken links
- Formatting issues
- Missing information
- Clarity and completeness

#### Task 4.3: Spot-Check Coverage (15 min)

Run quick grep to identify remaining gaps:

```bash
# Find public items without docs
rg "^\s*pub (fn|struct|enum|trait|type|const|static)" --no-heading | \
  grep -v "///" | head -20
```

---

## Documentation Templates

### Module Template

```rust
//! Brief one-line description of what this module does.
//!
//! Detailed explanation of the module's purpose, its role in the larger
//! system, and when developers should use it.
//!
//! # Examples
//!
//! ```rust
//! use crate::module_name::PublicItem;
//!
//! let item = PublicItem::new();
//! ```
```

### Struct Template

```rust
/// One-line description of what this struct represents.
///
/// Detailed explanation of the struct's purpose, lifecycle, and usage
/// patterns. Mention important invariants or constraints.
///
/// # Examples
///
/// ```rust
/// let instance = MyStruct::new();
/// instance.do_something();
/// ```
pub struct MyStruct {
    // Fields can have doc comments too for complex types
    /// The internal state maintained by this struct.
    field: String,
}
```

### Function Template

```rust
/// One-line summary of what this function does.
///
/// Extended explanation of the function's behavior, including any
/// important details about how it processes inputs or handles state.
///
/// # Arguments
///
/// * `param` - Description of what this parameter represents
/// * `config` - Configuration options for the operation
///
/// # Returns
///
/// Description of the return value and what it represents.
///
/// # Errors
///
/// This function returns an error if:
/// - Condition 1 occurs
/// - Condition 2 is not met
///
/// # Panics
///
/// Panics if the invariant X is violated.
///
/// # Examples
///
/// ```rust
/// let result = my_function("input", &config)?;
/// assert_eq!(result, expected);
/// ```
pub fn my_function(param: &str, config: &Config) -> Result<Output, Error> {
    // implementation
}
```

### Enum Template

```rust
/// One-line description of what this enum represents.
///
/// Extended explanation of when and how to use this enum.
///
/// # Examples
///
/// ```rust
/// match result {
///     MyEnum::Variant1 => println!("First case"),
///     MyEnum::Variant2(value) => println!("Second: {}", value),
/// }
/// ```
pub enum MyEnum {
    /// Description of what this variant means and when it's used.
    Variant1,

    /// Description of this variant and what the inner value represents.
    Variant2(String),
}
```

---

## Success Metrics

### Quantitative Goals

- [ ] 90%+ of public modules have `//!` documentation
- [ ] 95%+ of public structs/enums have `///` documentation
- [ ] 100% of enum variants are documented
- [ ] 90%+ of public functions have `///` documentation
- [ ] 50%+ of critical APIs have examples
- [ ] 0 broken links in generated docs
- [ ] All doc examples pass `cargo test --doc`

### Qualitative Goals

- [ ] New contributors can understand module purposes without reading code
- [ ] Public API usage is clear from documentation alone
- [ ] Error conditions are well-documented
- [ ] Examples demonstrate realistic usage patterns
- [ ] Documentation matches Rust ecosystem standards

---

## Recommendations

### Immediate Actions

1. **Start with Phase 1** (Quick Wins) - Module-level docs and enum documentation
2. **Prioritize user-facing APIs** in Phase 2 - `CharacterService`, `App`, `ChatStreamService`
3. **Leverage existing test coverage** - Convert test patterns into doc examples

### Long-Term Practices

1. **Documentation in PR Reviews**: Require docs for all new public APIs
2. **CI Integration**: Add `cargo doc` to CI to catch broken links
3. **Doc Tests in CI**: Run `cargo test --doc` to ensure examples stay valid
4. **Periodic Audits**: Quarterly review of documentation coverage

### Anti-Patterns to Avoid

1. ❌ **Don't duplicate type information** - Rust's type system already documents this
2. ❌ **Don't create parameter tables** - Use plain text explanations instead
3. ❌ **Don't over-document internal implementation** - Focus on public API contracts
4. ❌ **Don't write docs that just restate the function name** - Add meaningful context

### Good Patterns to Follow

1. ✅ **Start with a clear one-line summary** (<15 words)
2. ✅ **Include at least one realistic example** for complex APIs
3. ✅ **Document error conditions explicitly** with `# Errors` sections
4. ✅ **Link related types and functions** using `` [`Type`] `` syntax
5. ✅ **Explain the "why" not just the "what"** - provide context and rationale

---

## Parallelization Opportunities

The work can be distributed across multiple contributors:

- **Contributor A**: Phase 1 modules (A-C alphabetically)
- **Contributor B**: Phase 1 modules (D-M alphabetically)
- **Contributor C**: Phase 1 modules (N-Z alphabetically)
- **Contributor D**: Phase 2.1-2.2 (App and Character service)
- **Contributor E**: Phase 2.3-2.4 (Streaming and Auth)

After Phase 1-2, reconvene to review and proceed with Phase 3 collaboratively.

---

## References

### Official Rust Documentation

- [How to Write Documentation - rustdoc book](https://doc.rust-lang.org/rustdoc/how-to-write-documentation.html)
- [RFC 1574: API Documentation Conventions](https://rust-lang.github.io/rfcs/1574-more-api-documentation-conventions.html)
- [RFC 1946: Intra-rustdoc Links](https://rust-lang.github.io/rfcs/1946-intra-rustdoc-links.html)

### Industry Guidelines

- [Microsoft Pragmatic Rust Guidelines - Documentation](https://microsoft.github.io/rust-guidelines/guidelines/docs/)
- [Rust By Example - Documentation](https://doc.rust-lang.org/rust-by-example/meta/doc.html)

### Tools

```bash
# Generate documentation
cargo doc --no-deps --open

# Test documentation examples
cargo test --doc

# Find undocumented public items
cargo rustdoc -- -D missing_docs

# Check for broken intra-doc links
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
```

---

## Appendix: Current Documentation Analysis

### Well-Documented Examples to Emulate

**`src/auth/mod.rs:resolve_authentication()`**
```rust
/// Resolve authentication information for a provider
///
/// This function consolidates the common authentication resolution logic:
/// 1. Finding authentication for a specified provider
/// 2. Using config default provider if available
/// 3. Falling back to first available authentication
/// 4. Using environment variables as last resort
///
/// Returns: (api_key, base_url, provider_name, provider_display_name)
```

**Strengths**: Clear summary, numbered steps, explicit return value documentation

### Poorly-Documented Examples to Improve

**`src/commands/mod.rs:CommandResult`**
```rust
pub enum CommandResult {  // ← No documentation
    Continue,
    ContinueWithTranscriptFocus,
    ProcessAsMessage(String),
    // ... 7 more variants, all undocumented
}
```

**Impact**: Users cannot understand what each variant means without reading all command handler implementations.

**`src/character/service.rs:CharacterService`**
```rust
pub struct CharacterService {  // ← No documentation
    cache: CardCache,
    cards: HashMap<PathBuf, CachedCardEntry>,
    last_cache_key: Option<String>,
}
```

**Impact**: Users don't understand the caching strategy or when cache invalidation occurs.

---

## Conclusion

This sprint will elevate Chabeau's documentation from **~30% coverage** to **70%+ coverage**, making the codebase more accessible to contributors and users while following Rust ecosystem best practices. The phased approach allows for incremental progress with early wins in Phase 1, comprehensive coverage in Phase 2, and polish in Phase 3-4.

**Total Estimated Effort**: 12-16 hours (parallelizable across 3-5 contributors for ~3-5 hours per person)
