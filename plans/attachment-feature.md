# Attachment feature implementation plan

## Goals
- Support attaching local files in chat sessions with provider-aware handling.
- Preserve a consistent transcript message type for attachments regardless of provider capability.
- Provide user feedback about pending attachments prior to message send.
- Lay groundwork for interactive file selection via `/attach` with no arguments.

## Provider capability mapping
1. Extend provider definitions with upload capability metadata:
   - Enumerate which providers accept file uploads and the MIME types or file extensions they support.
   - Provide a list of constraints (max file size, per-request limits) when available.
2. Add a capability probing hook (e.g., `--probe` flag or subcommand) that:
   - Queries the configured provider for advertised upload support.
   - Falls back to local defaults if the provider cannot be queried.
   - Surfaces results in a human-readable summary for diagnostics.
3. Store capability info in a centralized registry/module to keep command handling provider-agnostic.

## Command handling (`/attach` and `-a`)
1. Parse `/attach <path>` and `-a <path>` as attachment intents:
   - Resolve paths (expanding `~`, relative paths) and validate existence/readability.
   - Capture file metadata (name, size, type) up front for UI display.
2. If the active provider supports uploads and the file type is allowed:
   - Prepare an upload request but defer actual transmission until the next outbound message.
   - Represent the attachment as a queued message entry with state (pending → uploaded → sent).
3. If uploads are unsupported or file type is disallowed:
   - Read up to 32 KiB of the file and insert it into the conversation context as a special attachment message.
   - Truncate with clear notation when the file exceeds the inline limit.
4. Support `/attach` without arguments (second phase):
   - Trigger a file picker abstraction that feeds back a path selection to the same handler.
   - Keep the command interface consistent across providers.

## Transcript and message model
1. Introduce/extend a dedicated attachment message variant in the transcript model containing:
   - Source path, display name, size, file type, provider handling mode (upload vs inline), and state.
   - Optional remote reference/URL once uploaded for providers that return one.
2. Ensure transcript rendering treats attachments distinctly (icons/labels) and avoids duplicating text content in normal messages.
3. Update serialization/logging so transcripts preserve attachment entries for replay.

## UI behavior
1. Input area should display a single attachment status line per pending file showing:
   - File name, size, handling mode (upload/inline), and any truncation notice.
   - Current state (pending upload, ready to send, uploaded).
2. Keep attachments queued until the user sends a message; then:
   - Upload supported files before or alongside the message send flow.
   - Include inline attachments in the message payload as needed.
3. Allow users to remove/cancel queued attachments from the input state prior to send.

## Error handling and limits
- Surface clear errors for missing files, unreadable paths, unsupported types, and oversized files.
- Guard against exceeding provider limits (size/type) before attempting uploads.
- Ensure inline insertion respects the 32 KiB cap and does not block UI responsiveness.

## Testing and validation
- Add unit tests for command parsing, capability mapping, and inline truncation.
- Add integration-style tests (or golden transcripts) to verify transcript entries and UI rendering for both upload-capable and non-capable providers.
- Include probes in tests to confirm provider capability reporting does not regress.
