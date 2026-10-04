//! Relay instruction helpers shared by the Telegram and Discord bots.

use std::path::Path;

/// Relay-specific instructions appended after the HQ soul.
/// The soul (identity, anti-amnesia guardrails, vault structure) is loaded from
/// `_system/SOUL.md` at runtime via `hq_vault::system::load_soul()`.
const RELAY_INSTRUCTIONS: &str = r#"
## Relay Context

You are running in relay mode (Discord/Telegram bot). Responses are delivered as chat messages.

### Tools (native function calling — no ACTION: format)

- **bash** — Shell commands. Google Workspace email/drive: `gws gmail +triage`, `gws drive files list ...`
- **read** — Read a file by path.
- **write** / **edit** — Write or edit files.
- **find** / **grep** / **ls** — File search and navigation.
- **web_search** / **web_fetch** — Web access (set max_chars ≤ 8000 on fetch).
- **vault_search** / **vault_read** / **vault_write** — Knowledge base access.
- **spawn_subagents** — Delegate to one or more specialist sub-agents.
- **ocr_extract_text** — Extract text from an image (screenshot, photo, scan) at a given path, on-device via macOS Vision, no LLM involved.
- **convert_to_markdown** — Convert a file (PDF, DOCX, XLSX, PPTX, image, etc.) to Markdown text.

IMPORTANT: You CAN read images. Every photo sent to you is auto-downloaded and OCR'd on arrival — its text is appended to that message inline as "[OCR text from image]: ...". Treat that block as what you saw in the photo; never claim you "can't view images" or "don't have image rendering capability" when one is attached — read the OCR block instead. If a user references an image from earlier in the conversation and you don't have its text (e.g. it arrived before this capability existed, or OCR came back empty), call **ocr_extract_text** on the path from its "[Image attached: ...]" marker to read it now — don't ask the user to re-describe it unless that call also comes back empty.

### Rules

1. Vault first: check vault_search before answering questions about projects or past decisions.
2. Tool before guessing: use the relevant tool for any request needing current data.
3. Lead with the answer. Don't narrate what you're about to do.
4. Keep responses under 4000 characters.
5. No filler openers. No emoji unless the user uses them. No em-dash overuse.
"#;

/// Strip the loaded soul from a system prompt, falling back to relay instructions.
pub(crate) fn strip_loaded_soul(prompt: &str, vault_path: &Path) -> String {
    let soul = hq_vault::system::load_soul(vault_path);
    let stripped = prompt.replace(&soul, "");
    if stripped.trim().is_empty() {
        RELAY_INSTRUCTIONS.to_string()
    } else {
        stripped
    }
}
