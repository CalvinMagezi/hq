use anyhow::Result;
use hq_core::config::HqConfig;
use std::io::{self, BufRead, IsTerminal, Write};

/// One provider key that `hq env` and `hq onboard` can prompt for.
pub(crate) struct ApiKey {
    label: &'static str,
    url: &'static str,
    get: fn(&HqConfig) -> &Option<String>,
    set: fn(&mut HqConfig, String),
}

pub(crate) const OPENROUTER: ApiKey = ApiKey {
    label: "OpenRouter API Key (routes to any model)",
    url: "https://openrouter.ai/keys",
    get: |c| &c.openrouter_api_key,
    set: |c, k| c.openrouter_api_key = Some(k),
};
pub(crate) const ANTHROPIC: ApiKey = ApiKey {
    label: "Anthropic API Key (direct Claude access)",
    url: "https://console.anthropic.com/settings/keys",
    get: |c| &c.anthropic_api_key,
    set: |c, k| c.anthropic_api_key = Some(k),
};
pub(crate) const GOOGLE_AI: ApiKey = ApiKey {
    label: "Google AI API Key (Gemini access)",
    url: "https://aistudio.google.com/apikey",
    get: |c| &c.google_ai_api_key,
    set: |c, k| c.google_ai_api_key = Some(k),
};
const CEREBRAS: ApiKey = ApiKey {
    label: "Cerebras API Key (free, ~1M tokens/day, fastest free option)",
    url: "https://cloud.cerebras.ai",
    get: |c| &c.cerebras_api_key,
    set: |c, k| c.cerebras_api_key = Some(k),
};
const GROQ: ApiKey = ApiKey {
    label: "Groq API Key (free, ~500K tokens/day, llama-3.3-70b)",
    url: "https://console.groq.com",
    get: |c| &c.groq_api_key,
    set: |c, k| c.groq_api_key = Some(k),
};
const DEEPSEEK: ApiKey = ApiKey {
    label: "DeepSeek API Key (budget, ~$0.14/M input, strong coding model)",
    url: "https://platform.deepseek.com",
    get: |c| &c.deepseek_api_key,
    set: |c, k| c.deepseek_api_key = Some(k),
};
const OPENAI: ApiKey = ApiKey {
    label: "OpenAI API Key (gpt-4o-mini, o4-mini, gpt-5)",
    url: "https://platform.openai.com",
    get: |c| &c.openai_api_key,
    set: |c, k| c.openai_api_key = Some(k),
};

const ALL_KEYS: [&ApiKey; 7] = [
    &OPENROUTER,
    &ANTHROPIC,
    &GOOGLE_AI,
    &CEREBRAS,
    &GROQ,
    &DEEPSEEK,
    &OPENAI,
];

const MASK_PREFIX_CHARS: usize = 8;

fn mask(key: &Option<String>) -> String {
    match key.as_deref() {
        Some(k) if !k.is_empty() => {
            format!(
                "{}...",
                k.chars().take(MASK_PREFIX_CHARS).collect::<String>()
            )
        }
        _ => "(not set)".to_string(),
    }
}

pub(crate) fn is_set(key: &ApiKey, config: &HqConfig) -> bool {
    (key.get)(config).as_deref().is_some_and(|k| !k.is_empty())
}

/// Prompt for one key with hidden input so the secret is never echoed.
pub(crate) fn prompt_key(
    n: usize,
    key: &ApiKey,
    config: &HqConfig,
    indent: &str,
) -> Result<String> {
    println!("{indent}{n}. {}", key.label);
    println!("{indent}   Get one at: {}", key.url);
    println!("{indent}   Current: {}", mask((key.get)(config)));
    print!("{indent}   Enter key (or press Enter to skip): ");
    io::stdout().flush()?;
    // Hidden input needs a terminal; piped or non-tty runs read stdin as before.
    let typed = if io::stdin().is_terminal() {
        rpassword::read_password()?
    } else {
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        line
    };
    println!();
    Ok(typed.trim().to_string())
}

/// Write the typed keys (and an optional new default model) to the config file.
/// Returns the saved config and whether any cloud key is now configured.
pub(crate) fn save_keys(
    config: &HqConfig,
    typed: Vec<(&'static ApiKey, String)>,
    model: Option<String>,
) -> Result<(HqConfig, bool)> {
    // Counts keys from any source, env included, but only typed values are
    // written, so an env-supplied key is never baked into the plaintext file.
    let has_any_cloud =
        typed.iter().any(|(_, v)| !v.is_empty()) || ALL_KEYS.iter().any(|k| is_set(k, config));
    let updated = HqConfig::save_patch(|c| {
        for (key, value) in typed.into_iter().filter(|(_, v)| !v.is_empty()) {
            (key.set)(c, value);
        }
        if let Some(model) = model {
            c.default_model = model;
        }
        c.apply_cloud_key_flip(has_any_cloud);
    })?;
    Ok((updated, has_any_cloud))
}

/// Interactive API key setup.
pub async fn run(config: &HqConfig) -> Result<()> {
    println!("\nAgent-HQ Environment Setup");
    println!("==========================\n");

    let mut typed = Vec::with_capacity(ALL_KEYS.len());
    for (i, key) in ALL_KEYS.into_iter().enumerate() {
        typed.push((key, prompt_key(i + 1, key, config, "")?));
    }

    println!("{}. Default LLM Model", ALL_KEYS.len() + 1);
    println!("   Current: {}", config.default_model);
    print!("   Enter model ID (or press Enter to keep current): ");
    io::stdout().flush()?;
    let mut model = String::new();
    io::stdin().lock().read_line(&mut model)?;
    let model = Some(model.trim().to_string()).filter(|m| !m.is_empty());
    println!();

    let (updated, has_any_cloud) = save_keys(config, typed, model)?;

    println!("Updated: {}\n", HqConfig::config_file_path().display());
    if has_any_cloud {
        println!("Cloud providers enabled (local_only: false).");
        println!("Default model: {}", updated.default_model);
    }
    println!("Run `hq health` to verify, or `hq` to start chatting.\n");

    Ok(())
}
