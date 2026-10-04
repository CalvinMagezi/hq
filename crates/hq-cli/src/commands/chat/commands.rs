use crate::commands::start::common::resolve_model_alias;
use crate::render::{self, Theme};
use hq_agent::session::AgentSession;
use hq_core::config::HqConfig;
use std::path::Path;
use std::sync::Mutex;

/// What the REPL should do after a built-in slash command.
pub(super) enum Flow {
    Quit,
    Handled,
    Unhandled,
}

pub(super) async fn handle_builtin(
    input: &str,
    session: &mut AgentSession,
    config: &HqConfig,
    theme: &Theme,
    last_model: &Mutex<String>,
    total_tokens: &Mutex<(u64, u64)>,
) -> Flow {
    // ── Prefix-based slash commands (with arguments) ──
    if input.starts_with("/model ") {
        let name = input.trim_start_matches("/model ").trim();
        if name.is_empty() {
            println!(
                "\x1b[{dim}m  Usage: /model <name>  (e.g. deepseek/deepseek-chat, sonnet, kimi)\x1b[0m",
                dim = render::fg(&theme.text_dim),
            );
        } else {
            let resolved = resolve_model_alias(name);
            session.set_model(&resolved);
            let _ = hq_core::config::HqConfig::set_key("default_model", &resolved);
            println!(
                "\x1b[{success}m  Model switched to: {resolved} (saved to config)\x1b[0m",
                success = render::fg(&theme.success),
            );
        }
        return Flow::Handled;
    }

    // ── Slash commands ──
    match input {
        "/quit" | "/exit" => {
            println!(
                "\x1b[{dim}mGoodbye!\x1b[0m",
                dim = render::fg(&theme.text_dim),
            );
            return Flow::Quit;
        }
        "/help" => {
            print_help(theme, &config.vault_path);
            return Flow::Handled;
        }
        "/stash" => {
            println!(
                "\x1b[{muted}m  Stash works with in-progress input. Nothing was stashed.\x1b[0m",
                muted = render::fg(&theme.text_muted),
            );
            return Flow::Handled;
        }
        "/pop" => {
            println!(
                "\x1b[{muted}m  No stashed prompts in inline mode.\x1b[0m",
                muted = render::fg(&theme.text_muted),
            );
            return Flow::Handled;
        }
        "/export" => {
            let transcript = export_transcript(session);
            let filename = format!(
                "hq-transcript-{}.md",
                chrono::Utc::now().format("%Y%m%d-%H%M%S")
            );
            match std::fs::write(&filename, &transcript) {
                Ok(_) => println!(
                    "\x1b[{muted}m  Exported to {filename} ({} bytes)\x1b[0m",
                    transcript.len(),
                    muted = render::fg(&theme.text_muted),
                ),
                Err(e) => println!(
                    "\x1b[{err}m  Export failed: {e}\x1b[0m",
                    err = render::fg(&theme.error),
                ),
            }
            return Flow::Handled;
        }
        "/cost" => {
            let model_name = last_model.lock().map(|m| m.clone()).unwrap_or_default();
            print_cost(theme, session, &model_name);
            return Flow::Handled;
        }
        "/tools" => {
            println!(
                "\x1b[{dim}m  {count} tools registered\x1b[0m",
                dim = render::fg(&theme.text_dim),
                count = session.tool_count(),
            );
            return Flow::Handled;
        }
        "/codereview" | "/review" => {
            println!(
                "\x1b[{dim}m  Reviewing uncommitted changes…\x1b[0m",
                dim = render::fg(&theme.text_dim),
            );
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            match hq_agent::adversarial::review_uncommitted_changes(&cwd).await {
                Ok(Some((review, has_blockers))) => {
                    println!("{review}");
                    if has_blockers {
                        println!(
                            "\x1b[{warn}m  Adversarial review raised blocking concerns — read above before you commit.\x1b[0m",
                            warn = render::fg(&theme.warning),
                        );
                    } else {
                        println!(
                            "\x1b[{success}m  No blocking concerns.\x1b[0m",
                            success = render::fg(&theme.success),
                        );
                    }
                }
                Ok(None) => println!(
                    "\x1b[{muted}m  No uncommitted changes to review.\x1b[0m",
                    muted = render::fg(&theme.text_muted),
                ),
                Err(e) => println!(
                    "\x1b[{err}m  Review failed: {e}\x1b[0m",
                    err = render::fg(&theme.error),
                ),
            }
            return Flow::Handled;
        }
        "/model" => {
            println!(
                "\x1b[{dim}m  Current model: {}  (use /model <name> to switch)\x1b[0m",
                session.model(),
                dim = render::fg(&theme.text_dim),
            );
            println!(
                "\x1b[{muted}m  Aliases: sonnet, opus, haiku, gemini, kimi, qwen, gpt4\x1b[0m",
                muted = render::fg(&theme.text_muted),
            );
            println!(
                "\x1b[{muted}m  Cloud: deepseek/deepseek-chat  Local: ollama/granite4.1:3b\x1b[0m",
                muted = render::fg(&theme.text_muted),
            );
            return Flow::Handled;
        }
        "/tokens" => {
            let (inp, out) = *total_tokens.lock().unwrap_or_else(|e| e.into_inner());
            println!(
                "\x1b[{dim}m  Tokens this session: {inp}\u{2191} {out}\u{2193} ({total} total)\x1b[0m",
                dim = render::fg(&theme.text_dim),
                total = inp + out,
            );
            return Flow::Handled;
        }
        "/compact" => {
            session.compact().await;
            println!(
                "\x1b[{muted}m  Context compacted.\x1b[0m",
                muted = render::fg(&theme.text_muted),
            );
            return Flow::Handled;
        }
        "/save" => {
            let sessions_dir = config.vault_path.join("_sessions");
            let title = session
                .messages()
                .iter()
                .find(|m| m.role == hq_core::types::MessageRole::User)
                .map(|m| m.content.chars().take(60).collect::<String>())
                .unwrap_or_else(|| "Untitled".into());
            let saved = render::SavedSession {
                id: uuid::Uuid::new_v4().to_string(),
                model: session.model().to_string(),
                title: title.clone(),
                messages: session
                    .messages()
                    .iter()
                    .map(|m| render::SavedMessage {
                        role: format!("{:?}", m.role).to_lowercase(),
                        content: m.content.clone(),
                        ttft_ms: None,
                    })
                    .collect(),
                tokens_in: session.total_input_tokens(),
                tokens_out: session.total_output_tokens(),
                cost_usd: session.total_cost(),
                created_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
            };
            match saved.save(&sessions_dir) {
                Ok(path) => println!(
                    "\x1b[{muted}m  Session saved: {} ({})\x1b[0m",
                    title,
                    path.display(),
                    muted = render::fg(&theme.text_muted),
                ),
                Err(e) => println!(
                    "\x1b[{err}m  Save failed: {e}\x1b[0m",
                    err = render::fg(&theme.error),
                ),
            }
            return Flow::Handled;
        }
        _ if input.starts_with("/resume") => {
            let sessions_dir = config.vault_path.join("_sessions");
            let arg = input.strip_prefix("/resume").unwrap().trim();

            if arg.is_empty() {
                // List saved sessions
                match render::SavedSession::list_sessions(&sessions_dir) {
                    Ok(sessions) if sessions.is_empty() => {
                        println!(
                            "\x1b[{dim}m  No saved sessions.\x1b[0m",
                            dim = render::fg(&theme.text_dim),
                        );
                    }
                    Ok(sessions) => {
                        println!(
                            "\x1b[{dim}m  Saved sessions (newest first):",
                            dim = render::fg(&theme.text_dim)
                        );
                        for (i, (path, _name)) in sessions.iter().take(10).enumerate() {
                            if let Ok(s) = render::SavedSession::load(path) {
                                println!(
                                    "    {}: {} ({} msgs, {}in/{}out)",
                                    i + 1,
                                    s.title,
                                    s.messages.len(),
                                    format_tokens(s.tokens_in),
                                    format_tokens(s.tokens_out),
                                );
                            }
                        }
                        println!("  Use /resume <N> to load a session.\x1b[0m");
                    }
                    Err(e) => println!(
                        "\x1b[{err}m  Error: {e}\x1b[0m",
                        err = render::fg(&theme.error),
                    ),
                }
            } else if let Ok(n) = arg.parse::<usize>() {
                // Load session by number
                match render::SavedSession::list_sessions(&sessions_dir) {
                    Ok(sessions) if n > 0 && n <= sessions.len() => {
                        let (path, _) = &sessions[n - 1];
                        match render::SavedSession::load(path) {
                            Ok(saved) => {
                                // Inject saved messages into the session
                                for msg in &saved.messages {
                                    let role = match msg.role.as_str() {
                                        "user" => hq_core::types::MessageRole::User,
                                        "assistant" => hq_core::types::MessageRole::Assistant,
                                        _ => hq_core::types::MessageRole::System,
                                    };
                                    session.push_message(hq_core::types::ChatMessage {
                                        image_parts: Vec::new(),
                                        role,
                                        content: msg.content.clone(),
                                        tool_calls: Vec::new(),
                                        tool_call_id: None,
                                        reasoning_content: None,
                                    });
                                }
                                println!(
                                    "\x1b[{muted}m  Resumed: {} ({} messages)\x1b[0m",
                                    saved.title,
                                    saved.messages.len(),
                                    muted = render::fg(&theme.text_muted),
                                );
                            }
                            Err(e) => println!(
                                "\x1b[{err}m  Load failed: {e}\x1b[0m",
                                err = render::fg(&theme.error),
                            ),
                        }
                    }
                    Ok(_) => println!(
                        "\x1b[{err}m  Invalid session number.\x1b[0m",
                        err = render::fg(&theme.error),
                    ),
                    Err(e) => println!(
                        "\x1b[{err}m  Error: {e}\x1b[0m",
                        err = render::fg(&theme.error),
                    ),
                }
            }
            return Flow::Handled;
        }
        _ => {}
    }
    Flow::Unhandled
}

// ── Display helpers ──────────────────────────────────────────────

fn print_help(theme: &Theme, vault_path: &Path) {
    let dim = render::fg(&theme.text_dim);
    println!("\x1b[{dim}m");
    println!("  Commands:");
    println!("  /quit, /exit       Exit the session");
    println!("  /cost              Show token usage, cost, and model");
    println!("  /model             Show current model (+ how to switch)");
    println!("  /model <name>      Switch model (sonnet, kimi, deepseek/deepseek-chat, ...)");
    println!("  /tokens            Show session token totals");
    println!("  /tools             Show registered tool count");
    println!("  /codereview        Adversarial review of uncommitted changes (alias /review)");
    println!("  /compact           Compact conversation context");
    println!("  /save              Save this session to vault");
    println!("  /resume            List/browse saved sessions");
    println!("  /export            Export conversation to markdown file");
    println!("  /split             Toggle tool activity panel");
    println!("  /stash             Save current input to stash");
    println!("  /pop               Restore last stashed input");
    println!("  /help              Show this help");
    println!(
        "  <image path>       Paste/type a png/jpg/gif/webp/bmp/tiff/heic path to attach it (OCR'd on-device)"
    );

    let custom = hq_tools::slash_commands::load_custom_commands(vault_path);
    if !custom.is_empty() {
        println!();
        println!("  Custom commands (_commands/ in vault):");
        for cmd in &custom {
            println!("  /{:<18} {}", cmd.name, cmd.description);
        }
    }
    println!();
    println!("  Keybindings (TUI mode):");
    println!("  Shift+Tab          Cycle mode: Agent > Code > Plan");
    println!("  Ctrl+K             Command palette");
    println!("  Ctrl+T             Toggle split-pane (tool activity)");
    println!("  Ctrl+R             Reverse history search");
    println!("  Shift+Enter        Insert newline");
    println!("  Ctrl+D / Ctrl+C    Exit / Cancel");
    println!("\x1b[0m");
}

pub(super) fn print_cost(theme: &Theme, session: &AgentSession, model: &str) {
    let model_display = if model.is_empty() {
        session.model().to_string()
    } else {
        // Shorten long model names (e.g., "deepseek-chat" from "deepseek/deepseek-chat")
        model.rsplit('/').next().unwrap_or(model).to_string()
    };
    println!(
        "\x1b[{dim}m  {in_t}in / {out_t}out | ${cost:.4} | {model}\x1b[0m",
        dim = render::fg(&theme.text_dim),
        in_t = format_tokens(session.total_input_tokens()),
        out_t = format_tokens(session.total_output_tokens()),
        cost = session.total_cost(),
        model = model_display,
    );
}

pub(super) fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M ", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K ", n as f64 / 1_000.0)
    } else {
        format!("{} ", n)
    }
}

/// Export session messages to a markdown string.
fn export_transcript(session: &AgentSession) -> String {
    let mut out = String::from("# HQ Agent Transcript\n\n");
    out.push_str(&format!(
        "Model: {} | Date: {}\n\n---\n\n",
        session.model(),
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"),
    ));

    for msg in session.messages() {
        match msg.role {
            hq_core::types::MessageRole::User => {
                out.push_str(&format!("**You:**\n\n{}\n\n", msg.content));
            }
            hq_core::types::MessageRole::Assistant => {
                out.push_str(&format!("**HQ:**\n\n{}\n\n", msg.content));
            }
            hq_core::types::MessageRole::System => {
                // Skip system messages in export
            }
            _ => {}
        }
    }

    out.push_str(&format!(
        "---\n\nTokens: {}in / {}out | Cost: ${:.4}\n",
        format_tokens(session.total_input_tokens()),
        format_tokens(session.total_output_tokens()),
        session.total_cost(),
    ));

    out
}
