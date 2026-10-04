use crate::render::{self, Theme};
use anyhow::Result;
use hq_agent::builder::SessionBuilder;
use hq_agent::session::AgentSession;
use hq_agent::session_presets;
use hq_core::config::HqConfig;
use hq_core::middleware::unstructured_llm_error_looks_transient;
use hq_core::types::{SessionEvent, SessionResult};
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::commands::start::common::resolve_model_alias;

mod commands;
mod context;
mod events;
mod remote;

pub use remote::{ServerFlag, auto_from_env};

use commands::{format_tokens, print_cost};
use context::{
    augment_with_image_attachments, detect_git_branch, inject_project_context,
    looks_like_existing_path,
};

// ── Public entry points ──────────────────────────────────────────

/// Entry point for `hq chat`: picks the daemon's backend (remote) or builds
/// one in-process (local), says which, then runs the REPL or a one-shot turn.
pub async fn start(
    config: &HqConfig,
    model_override: Option<String>,
    tui: bool,
    permission_preset: Option<hq_core::types::PermissionPreset>,
    server: ServerFlag,
    prompt: Option<String>,
) -> Result<()> {
    let Some(target) = remote::resolve_target(config, &server).await? else {
        eprintln!("hq chat: local mode (in-process backend)");
        return match prompt {
            Some(p) => run_local_oneshot(config, model_override, permission_preset, &p).await,
            None if tui => run_fullscreen(config, model_override, permission_preset).await,
            None => run(config, model_override, permission_preset).await,
        };
    };
    eprintln!("hq chat: remote mode via {}", target.http_base);
    if model_override.is_some() || permission_preset.is_some() || tui {
        eprintln!("hq chat: --model, --permission-preset and --tui are ignored in remote mode (the daemon decides).");
    }
    eprintln!("hq chat: tools run on the daemon's host, in the daemon's working directory.");
    match prompt {
        Some(p) => remote::run_oneshot(&target, &p).await,
        None => remote::run_interactive(&target).await,
    }
}

/// One local turn: reply text on stdout, tool and error lines on stderr.
async fn run_local_oneshot(
    config: &HqConfig,
    model_override: Option<String>,
    permission_preset: Option<hq_core::types::PermissionPreset>,
    prompt: &str,
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let mut session = build_session(config, model_override, &cwd, permission_preset).await?;
    session.on_event(|event| match event {
        SessionEvent::TextDelta(text) => {
            print!("{text}");
            let _ = io::stdout().flush();
        }
        SessionEvent::ToolStart { tool_name, .. } => eprintln!("[{tool_name}]"),
        SessionEvent::Error(msg) => eprintln!("Error: {msg}"),
        _ => {}
    });
    let result = session.prompt_stream(prompt).await?;
    println!();
    record_cli_turn(config, prompt, result.text());
    match result.failure_reason() {
        Some(reason) => anyhow::bail!("turn failed: {reason}"),
        None => Ok(()),
    }
}

/// Run chat: TUI mode (default) or inline mode for pipes.
pub async fn run(
    config: &HqConfig,
    model_override: Option<String>,
    permission_preset: Option<hq_core::types::PermissionPreset>,
) -> Result<()> {
    let is_tty = crossterm::terminal::size().is_ok();
    if is_tty {
        run_agent_chat(config, model_override, permission_preset).await
    } else {
        run_inline_chat(config, model_override, permission_preset).await
    }
}

/// Fullscreen TUI mode (--tui flag). Delegates to the agent chat for now.
pub async fn run_fullscreen(
    config: &HqConfig,
    model_override: Option<String>,
    permission_preset: Option<hq_core::types::PermissionPreset>,
) -> Result<()> {
    run_agent_chat(config, model_override, permission_preset).await
}

// ── Shared session builder ───────────────────────────────────────

/// Build a coding-mode session with the shared HQ context frame.
/// Uses the same session config and harness instructions as `hq code`.
async fn build_session(
    config: &HqConfig,
    model_override: Option<String>,
    cwd: &Path,
    permission_preset: Option<hq_core::types::PermissionPreset>,
) -> Result<AgentSession> {
    let effective_model = match model_override {
        Some(ref m) => resolve_model_alias(m),
        None => hq_core::config::resolve_session_model(config),
    };

    let mut session_config = session_presets::code_session_config();
    session_config.model = effective_model;
    // A live human is driving `hq chat` at the terminal right now.
    session_config.is_live_user_turn = true;

    let mut builder = SessionBuilder::from_config(config)
        .working_dir(cwd.to_path_buf())
        .allow_path(cwd.to_path_buf())
        .harness_instructions(session_presets::terminal_code_harness_instructions(cwd))
        .session_config(session_config)
        .with_identity(hq_core::identity::RequestIdentity::local());
    if let Some(preset) = permission_preset {
        builder = builder.permission_preset(preset);
    }
    let mut session = builder.build().await?;

    // Layer project context (git branch/status, CLAUDE.md) on top of the shared
    // soul/memory/thread context.
    inject_project_context(&mut session, cwd);

    Ok(session)
}

// ── Agent Chat (main interactive experience) ─────────────────────

/// Check whether the model string points at Ollama and, if so, whether Ollama
/// is reachable and the model is pulled. Prints actionable warnings; never
/// blocks startup (all errors are advisory only).
async fn check_provider_health(config: &HqConfig, model_override: Option<&str>, theme: &Theme) {
    let effective_model = model_override
        .map(resolve_model_alias)
        .unwrap_or_else(|| hq_core::config::resolve_session_model(config));

    if !effective_model.starts_with("ollama/") && effective_model != "code" {
        return; // cloud provider — no local health check needed
    }

    // For the "code" alias with local_only=true, it routes to an Ollama model.
    let is_local_code_alias = effective_model == "code" && config.local_only;
    if effective_model.starts_with("ollama/") || is_local_code_alias {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap_or_default();

        match client.get("http://localhost:11434/").send().await {
            Err(_) => {
                println!(
                    "\x1b[{warn}m  \u{26a0} Ollama unreachable at localhost:11434.\x1b[0m",
                    warn = render::fg(&theme.warning),
                );
                println!(
                    "\x1b[{dim}m  Start Ollama: open the Ollama app or run `ollama serve`\x1b[0m",
                    dim = render::fg(&theme.text_dim),
                );
                println!(
                    "\x1b[{dim}m  Or switch to cloud: `hq config set local_only false` then restart\x1b[0m",
                    dim = render::fg(&theme.text_dim),
                );
            }
            Ok(_) => {
                // Ollama is up — check if the specific model is pulled
                let model_name = effective_model
                    .strip_prefix("ollama/")
                    .unwrap_or(&effective_model);
                if let Ok(resp) = client
                    .get("http://localhost:11434/api/tags")
                    .send()
                    .await
                    && let Ok(json) = resp.json::<serde_json::Value>().await {
                        let pulled = json
                            .get("models")
                            .and_then(|m| m.as_array())
                            .map(|arr| {
                                arr.iter().any(|m| {
                                    m.get("name")
                                        .and_then(|n| n.as_str())
                                        .map(|n| n.starts_with(model_name))
                                        .unwrap_or(false)
                                })
                            })
                            .unwrap_or(false);

                        if !pulled {
                            println!(
                                "\x1b[{warn}m  \u{26a0} Model '{model_name}' not found in Ollama.\x1b[0m",
                                warn = render::fg(&theme.warning),
                            );
                            println!(
                                "\x1b[{dim}m  Pull it: `ollama pull {model_name}`\x1b[0m",
                                dim = render::fg(&theme.text_dim),
                            );
                            println!(
                                "\x1b[{dim}m  Or switch provider: /model deepseek/deepseek-chat\x1b[0m",
                                dim = render::fg(&theme.text_dim),
                            );
                        }
                    }
            }
        }
    }
}

async fn run_agent_chat(
    config: &HqConfig,
    model_override: Option<String>,
    permission_preset: Option<hq_core::types::PermissionPreset>,
) -> Result<()> {
    let theme = Theme::dark();
    let cwd = std::env::current_dir()?;

    check_provider_health(config, model_override.as_deref(), &theme).await;
    let mut session = build_session(config, model_override, &cwd, permission_preset).await?;

    // ── Wire memory extraction (once per completed prompt) ──
    hq_agent::native_hq::wire_post_turn_ingestion(
        &mut session,
        &config.vault_path,
        &config.db_path(),
        "cli",
    );

    // ── Print startup header ──
    let project_name = cwd
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| cwd.display().to_string());
    let git_branch = detect_git_branch(&cwd);

    println!(
        "\n\x1b[{primary}m\x1b[1m HQ Agent Session\x1b[0m",
        primary = render::fg(&theme.primary),
    );
    println!(
        " \x1b[{dim}mProject:\x1b[0m {project}  \x1b[{dim}mBranch:\x1b[0m {branch}",
        dim = render::fg(&theme.text_dim),
        project = project_name,
        branch = git_branch,
    );
    println!(
        " \x1b[{dim}mModel:\x1b[0m {model}  \x1b[{dim}mTools:\x1b[0m {tools}",
        dim = render::fg(&theme.text_dim),
        model = session.model(),
        tools = session.tool_count(),
    );
    println!(
        " \x1b[{dim}mVault:\x1b[0m {vault}  \x1b[{dim}mDir:\x1b[0m {cwd}",
        dim = render::fg(&theme.text_dim),
        vault = config.vault_path.display(),
        cwd = cwd.display(),
    );
    println!(
        " \x1b[{muted}mType /help for commands. /quit to exit. Ctrl+D for EOF.\x1b[0m\n",
        muted = render::fg(&theme.text_muted),
    );

    // ── Wire event rendering ──
    let first_token_flag = Arc::new(AtomicBool::new(true));
    let last_model = Arc::new(std::sync::Mutex::new(String::new()));
    let total_tokens = Arc::new(std::sync::Mutex::new((0u64, 0u64)));
    let turn_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
    events::wire_event_rendering(
        &mut session,
        theme.clone(),
        first_token_flag.clone(),
        last_model.clone(),
        total_tokens.clone(),
    );
    // ── Interactive REPL loop ──
    let mut stdout = io::stdout();

    loop {
        first_token_flag.store(true, Ordering::Relaxed);

        print!(
            "\x1b[{primary}myou> \x1b[0m",
            primary = render::fg(&theme.primary),
        );
        stdout.flush()?;

        // Read the line on a blocking thread so Ctrl+C at the idle prompt is still
        // observed via `tokio::select!` — a bare `read_line()` call here blocks the
        // whole task, and since `ctrl_c()` is used elsewhere in this loop it has
        // already installed tokio's signal handler in place of the OS default, so
        // an un-polled Ctrl+C would otherwise be silently swallowed with no way to exit.
        let read = tokio::select! {
            r = tokio::task::spawn_blocking(|| {
                let mut buf = String::new();
                let n = io::stdin().lock().read_line(&mut buf)?;
                Ok::<_, std::io::Error>((n, buf))
            }) => r,
            _ = tokio::signal::ctrl_c() => {
                println!(
                    "\n\x1b[{dim}mGoodbye!\x1b[0m",
                    dim = render::fg(&theme.text_dim),
                );
                // The `read_line` blocking task above is parked in a blocking syscall
                // on stdin with no way to cancel it; a plain `return` would leave the
                // tokio runtime waiting forever for it to join during shutdown, hanging
                // the process instead of exiting. Exit directly instead.
                std::process::exit(0);
            }
        };
        let (n, input) = match read {
            Ok(Ok(pair)) => pair,
            Ok(Err(e)) => return Err(e.into()),
            Err(e) => return Err(e.into()),
        };
        if n == 0 {
            println!(
                "\n\x1b[{dim}mGoodbye!\x1b[0m",
                dim = render::fg(&theme.text_dim),
            );
            break;
        }

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        match commands::handle_builtin(
            input,
            &mut session,
            config,
            &theme,
            &last_model,
            &total_tokens,
        )
        .await
        {
            commands::Flow::Quit => break,
            commands::Flow::Handled => continue,
            commands::Flow::Unhandled => {}
        }

        // ── Custom slash commands (vault-defined, _commands/*.md) ──
        // Any `/name` that reached here matched none of the built-ins above.
        // Expand it against a vault-defined template if one exists; otherwise
        // it is an unrecognized command, not a message, so it must not be
        // silently forwarded to the model as a literal prompt.
        //
        // Guard: an absolute path (e.g. a pasted image path) also starts with
        // `/`, so treat it as a slash command only when it isn't an existing
        // file on disk — otherwise `/Users/you/screenshot.png explain this`
        // used to be misread as an unknown command instead of a message.
        let expanded_command;
        let augmented_input;
        let input: &str = if input.starts_with('/') && !looks_like_existing_path(input) {
            let rest = input.strip_prefix('/').unwrap();
            let mut parts = rest.splitn(2, ' ');
            let cmd_name = parts.next().unwrap_or("");
            let cmd_args = parts.next().unwrap_or("").trim();
            match hq_tools::slash_commands::find_custom_command(&config.vault_path, cmd_name) {
                Some(cmd) => {
                    expanded_command =
                        hq_tools::slash_commands::render_custom_command(&cmd, cmd_args);
                    expanded_command.as_str()
                }
                None => {
                    println!(
                        "\x1b[{err}m  Unknown command '/{cmd_name}'. Type /help for the list.\x1b[0m",
                        err = render::fg(&theme.error),
                    );
                    continue;
                }
            }
        } else {
            augmented_input = augment_with_image_attachments(input).await;
            augmented_input.as_str()
        };

        // ── Run the full agent loop ──
        // Animated spinner runs on a blocking thread; stops when first_token_flag goes false.
        let flag_for_spinner = first_token_flag.clone();
        let theme_sp = theme.clone();
        let spinner_handle = tokio::task::spawn_blocking(move || {
            const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let start = std::time::Instant::now();
            let mut i = 0usize;
            while flag_for_spinner.load(Ordering::Relaxed) {
                let s = start.elapsed().as_secs();
                let t = if s < 60 {
                    format!("{}s", s)
                } else {
                    format!("{}m{}s", s / 60, s % 60)
                };
                print!(
                    "\r\x1b[{muted}m{f} thinking... ({t})\x1b[0m   ",
                    muted = render::fg(&theme_sp.text_muted),
                    f = FRAMES[i % FRAMES.len()],
                );
                let _ = io::stdout().flush();
                i += 1;
                std::thread::sleep(std::time::Duration::from_millis(80));
            }
        });

        let result = tokio::select! {
            r = session.prompt_stream(input) => r,
            _ = tokio::signal::ctrl_c() => {
                first_token_flag.store(false, Ordering::Relaxed);
                let _ = spinner_handle.await;
                let w = crossterm::terminal::size().map(|(w, _)| w as usize).unwrap_or(80);
                print!("\r{}\r", " ".repeat(w));
                println!(
                    "\n\x1b[{muted}m  [cancelled]\x1b[0m",
                    muted = render::fg(&theme.text_muted),
                );
                continue;
            }
        };
        // Ensure spinner exits before we write any post-turn output.
        first_token_flag.store(false, Ordering::Relaxed);
        let _ = spinner_handle.await;

        match result {
            Ok(ref sr) => {
                // Empty response means all providers were rate-limited. Auto-retry once
                // after a short backoff so the router can pick a recovered provider.
                if sr.text().is_empty() {
                    println!(
                        "\n\x1b[{warn}m  Providers rate-limited. Retrying in 5s...\x1b[0m",
                        warn = render::fg(&theme.warning),
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    let retry = tokio::select! {
                        r = session.prompt_stream(input) => r,
                        _ = tokio::signal::ctrl_c() => {
                            println!(
                                "\n\x1b[{muted}m  [cancelled]\x1b[0m",
                                muted = render::fg(&theme.text_muted),
                            );
                            continue;
                        }
                    };
                    match retry {
                        Ok(ref rsr) if rsr.text().is_empty() => {
                            println!(
                                "\x1b[{warn}m  Still rate-limited. Wait a moment and try again.\x1b[0m\n",
                                warn = render::fg(&theme.warning),
                            );
                            continue;
                        }
                        Ok(_) => {} // fall through to normal handling below with retry result
                        Err(e) => {
                            println!(
                                "\n\x1b[{err}m  Error: {e}\x1b[0m\n",
                                err = render::fg(&theme.error),
                            );
                            continue;
                        }
                    }
                    println!();
                    let model_name = last_model.lock().map(|m| m.clone()).unwrap_or_default();
                    print_cost(&theme, &session, &model_name);
                    println!();
                    continue;
                }
                println!();
                record_cli_turn(config, input, sr.text());
                match sr {
                    SessionResult::BudgetExhausted(_) => {
                        println!(
                            "\x1b[{err}m  [budget exhausted]\x1b[0m",
                            err = render::fg(&theme.error),
                        );
                    }
                    SessionResult::TimeLimitReached(_) => {
                        println!(
                            "\x1b[{warn}m  [time limit reached]\x1b[0m",
                            warn = render::fg(&theme.warning),
                        );
                    }
                    SessionResult::Cancelled(_) => {
                        println!(
                            "\x1b[{warn}m  [cancelled]\x1b[0m",
                            warn = render::fg(&theme.warning),
                        );
                    }
                    SessionResult::Failed { .. } => {
                        println!(
                            "\x1b[{err}m  [failed: backend error]\x1b[0m",
                            err = render::fg(&theme.error),
                        );
                    }
                    SessionResult::Complete(_) => {}
                }
                // Prefer the actual resolved model name from the session (e.g. "deepseek/deepseek-chat")
                // over the router alias (e.g. "code") so the user can see which model was used.
                let model_name = session
                    .last_resolved_model
                    .clone()
                    .or_else(|| last_model.lock().map(|m| m.clone()).ok())
                    .unwrap_or_else(|| session.model().to_string());
                print_cost(&theme, &session, &model_name);
                let turn = turn_count.fetch_add(1, Ordering::Relaxed) + 1;
                let (inp, out) = *total_tokens.lock().unwrap_or_else(|e| e.into_inner());
                println!(
                    "\x1b[{dim}m  [turn {turn} \u{00b7} {inp}\u{2191} {out}\u{2193} tokens]\x1b[0m",
                    dim = render::fg(&theme.text_dim),
                );
                println!();
            }
            Err(e) => {
                let msg = e.to_string();
                if unstructured_llm_error_looks_transient(&msg) || msg.contains("Too Many Requests")
                {
                    println!(
                        "\n\x1b[{warn}m  Rate limited. Wait a moment and try again.\x1b[0m\n",
                        warn = render::fg(&theme.warning),
                    );
                } else if msg.contains("No provider found") {
                    println!(
                        "\n\x1b[{err}m  No provider available for model '{}'. Check your API keys.\x1b[0m\n",
                        session.model(),
                        err = render::fg(&theme.error),
                    );
                } else {
                    println!(
                        "\n\x1b[{err}m  Error: {msg}\x1b[0m\n",
                        err = render::fg(&theme.error),
                    );
                }
            }
        }
    }
    Ok(())
}

// ── Inline chat (non-TTY / piped) ────────────────────────────────

async fn run_inline_chat(
    config: &HqConfig,
    model_override: Option<String>,
    permission_preset: Option<hq_core::types::PermissionPreset>,
) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let mut session = build_session(config, model_override, &cwd, permission_preset).await?;

    println!(
        "HQ Agent (model: {}, tools: {})",
        session.model(),
        session.tool_count(),
    );
    println!("Type your message. Press Ctrl+D to exit.\n");

    // Wire simple event rendering for inline mode
    session.on_event(move |event| match event {
        SessionEvent::TextDelta(text) => {
            print!("{}", text);
            let _ = io::stdout().flush();
        }
        SessionEvent::ToolStart { tool_name, .. } => {
            print!("[{tool_name}] ");
            let _ = io::stdout().flush();
        }
        SessionEvent::ToolEnd { result, .. } => {
            if result.len() > 100 {
                println!("...({} bytes)", result.len());
            }
        }
        SessionEvent::Error(msg) => {
            eprintln!("Error: {msg}");
        }
        _ => {}
    });
    let mut stdout = io::stdout();

    loop {
        print!("you> ");
        stdout.flush()?;

        // Same rationale as the TTY loop above: read on a blocking thread and race it
        // against Ctrl+C so an idle prompt can still be interrupted/exited.
        let read = tokio::select! {
            r = tokio::task::spawn_blocking(|| {
                let mut buf = String::new();
                let n = io::stdin().lock().read_line(&mut buf)?;
                Ok::<_, std::io::Error>((n, buf))
            }) => r,
            _ = tokio::signal::ctrl_c() => {
                println!("\nGoodbye!");
                // See the matching comment in the TTY loop: the parked read_line task
                // can't be cancelled, so a plain `return` would hang the runtime shutdown.
                std::process::exit(0);
            }
        };
        let (n, input) = match read {
            Ok(Ok(pair)) => pair,
            Ok(Err(e)) => return Err(e.into()),
            Err(e) => return Err(e.into()),
        };
        if n == 0 {
            println!("\nGoodbye!");
            break;
        }
        let input = input.trim();
        if input.is_empty() {
            continue;
        }
        match input {
            "/quit" | "/exit" => {
                println!("Goodbye!");
                break;
            }
            "/cost" => {
                println!(
                    "  Tokens: {}in / {}out | Cost: ${:.4}",
                    format_tokens(session.total_input_tokens()),
                    format_tokens(session.total_output_tokens()),
                    session.total_cost(),
                );
                continue;
            }
            "/codereview" | "/review" => {
                let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                match hq_agent::adversarial::review_uncommitted_changes(&cwd).await {
                    Ok(Some((review, has_blockers))) => {
                        println!("{review}");
                        if has_blockers {
                            println!("  [blocking concerns raised]");
                        }
                    }
                    Ok(None) => println!("  No uncommitted changes to review."),
                    Err(e) => println!("  Review failed: {e}"),
                }
                continue;
            }
            _ => {}
        }

        let augmented_input = augment_with_image_attachments(input).await;
        let input = augmented_input.as_str();

        match session.prompt_stream(input).await {
            Ok(sr) => {
                record_cli_turn(config, input, sr.text());
                match sr.failure_reason() {
                    // The partial text already streamed to stdout; flag the failure
                    // on stderr rather than treating the truncated turn as success.
                    Some(reason) => eprintln!("\n[turn failed: {reason}]\n"),
                    None => println!("\n"),
                }
            }
            Err(e) => eprintln!("Error: {e}\n"),
        }
    }

    Ok(())
}

/// Append a completed CLI turn to `_threads/cli.jsonl` for cross-interface
/// continuity. Best-effort; a failure never disturbs the REPL.
fn record_cli_turn(config: &HqConfig, input: &str, reply: &str) {
    let identity = hq_core::identity::RequestIdentity::local();
    for (role, text) in [("user", input), ("assistant", reply)] {
        if let Err(e) =
            hq_agent::threads::append_thread_entry(&config.vault_path, &identity, role, text)
        {
            tracing::warn!(%e, role, "cli chat: failed to append thread entry");
        }
    }
}
