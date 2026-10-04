use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use hq_core::config::HqConfig;
use hq_core::types::{ChatMessage, MessageRole};
use hq_llm::openrouter::OpenRouterProvider;
use hq_llm::provider::{ChatRequest, LlmProvider};
use std::io::{self, Write};

#[derive(Args, Debug)]
pub struct ProfileArgs {
    #[command(subcommand)]
    pub sub: ProfileSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum ProfileSubcommand {
    /// Generate a quality profile (SKILL.md + SUMMARY.md) for a domain
    Generate {
        /// The domain to generate a profile for (e.g. "technical documentation")
        #[arg(long)]
        domain: String,
        /// Number of samples to generate/analyze
        #[arg(long, default_value = "10")]
        samples: usize,
    },
}

pub async fn run(config: &HqConfig, args: &ProfileArgs) -> Result<()> {
    match &args.sub {
        ProfileSubcommand::Generate { domain, samples } => generate(config, domain, *samples).await,
    }
}

async fn generate(config: &HqConfig, domain: &str, samples_count: usize) -> Result<()> {
    let api_key = config
        .openrouter_api_key
        .as_ref()
        .context("OpenRouter API key not set. Run `hq setup` or set HQ_OPENROUTER_API_KEY")?;

    let provider = OpenRouterProvider::new(api_key);
    let model = config.default_model.clone();

    println!("Generating quality profile for domain: {}", domain);
    println!("Model: {}", model);
    println!("Samples: {}", samples_count);
    println!();

    // 1. Generate Prompt Ideas
    print!("Step 1/4: Generating prompt ideas... ");
    io::stdout().flush()?;

    let prompt_ideas_req = ChatRequest {
        model: model.clone(),
        messages: vec![ChatMessage {
            image_parts: Vec::new(),
            role: MessageRole::User,
            content: format!(
                "Generate {samples_count} diverse, realistic prompts that a user might give to an AI to produce content in the domain of '{domain}'. \
                 Return ONLY the prompts, one per line, no numbers or bullets."
            ),
            tool_calls: vec![],
            tool_call_id: None,
            reasoning_content: None,
        }],
        tools: vec![],
        temperature: Some(0.8),
        max_tokens: Some(2048),
    };

    let prompts_res = provider.chat(&prompt_ideas_req).await?;
    let prompts_raw = prompts_res.message.content;
    let prompts: Vec<String> = prompts_raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.to_string())
        .collect();

    println!("Done ({})", prompts.len());

    // 2. Generate Samples
    print!("Step 2/4: Generating AI samples... ");
    io::stdout().flush()?;

    let mut samples = Vec::new();
    for p in prompts.iter().take(samples_count) {
        let sample_req = ChatRequest {
            model: model.clone(),
            messages: vec![ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::User,
                content: format!(
                    "Profoundly fulfill this request in the domain of '{domain}':\n\n{p}"
                ),
                tool_calls: vec![],
                tool_call_id: None,
                reasoning_content: None,
            }],
            tools: vec![],
            temperature: Some(0.7),
            max_tokens: Some(1024),
        };

        if let Ok(res) = provider.chat(&sample_req).await {
            samples.push(res.message.content);
            print!(".");
            io::stdout().flush()?;
        }
    }
    println!(" Done");

    // 3. Analyze Patterns
    print!("Step 3/4: Analyzing repetitive anti-patterns... ");
    io::stdout().flush()?;

    let samples_block = samples
        .iter()
        .enumerate()
        .map(|(i, s)| format!("### SAMPLE {}:\n\n{}\n\n", i + 1, s))
        .collect::<String>();

    let analysis_req = ChatRequest {
        model: model.clone(),
        messages: vec![ChatMessage {
            image_parts: Vec::new(),
            role: MessageRole::User,
            content: format!(
                "I am providing {samples_count} samples of AI output for the domain '{domain}'. \
                 Analyze these samples for recurring patterns that make them feel like 'AI slop'. \
                 Identify: \
                 - Tired clichés and repetitive openers \
                 - Overused metaphors or 'corporate' word choices \
                 - Predictable structural patterns (e.g. perfect balance, 'Not just X, but Y') \
                 - Tonal tells (over-politeness, eager-to-help-ness) \
                 \
                 ONLY report patterns appearing in >30% of samples. \
                 CRITICAL: Focus ONLY on what to AVOID. Do NOT prescribe alternatives. \
                 \
                 SAMPLES:\n\n{samples_block}"
            ),
            tool_calls: vec![],
            tool_call_id: None,
            reasoning_content: None,
        }],
        tools: vec![],
        temperature: Some(0.3),
        max_tokens: Some(4096),
    };

    let analysis_res = provider.chat(&analysis_req).await?;
    let analysis = analysis_res.message.content;
    println!("Done");

    // 4. Generate Skill Files
    print!("Step 4/4: Formatting skill files... ");
    io::stdout().flush()?;

    let skill_req = ChatRequest {
        model: model.clone(),
        messages: vec![ChatMessage {
            image_parts: Vec::new(),
            role: MessageRole::User,
            content: format!(
                "Convert the following anti-pattern analysis into two HQ skill files. \
                 \
                 FILE 1: SKILL.md \
                 Format: Markdown with YAML frontmatter. \
                 Frontmatter fields: description (1 sentence), autoLoad: true, hints (5-10 keywords). \
                 Content: Structured anti-patterns with checkboxes. \
                 \
                 FILE 2: SUMMARY.md \
                 Format: A concise list of the Top 8-10 most impactful 'Do not...' rules. \
                 \
                 ANALYSIS:\n\n{analysis}\n\n\
                 Output both files separated by '---FILE: SUMMARY.md---'."
            ),
            tool_calls: vec![],
            tool_call_id: None,
            reasoning_content: None,
        }],
        tools: vec![],
        temperature: Some(0.3),
        max_tokens: Some(4096),
    };

    let skill_res = provider.chat(&skill_req).await?;
    let output = skill_res.message.content;

    let parts: Vec<&str> = output.split("---FILE: SUMMARY.md---").collect();
    let skill_md = parts.first().unwrap_or(&"").trim();
    let summary_md = parts.get(1).unwrap_or(&"").trim();

    // Write to vault
    let domain_slug = domain.to_lowercase().replace(' ', "-");
    let skill_name = format!("quality-{}", domain_slug);
    let target_dir = hq_core::skills_dir(&config.vault_path).join(&skill_name);

    std::fs::create_dir_all(&target_dir)?;
    std::fs::write(target_dir.join("SKILL.md"), skill_md)?;
    std::fs::write(target_dir.join("SUMMARY.md"), summary_md)?;

    println!("Done");
    println!();
    println!(
        "Success! Quality profile generated at: {}",
        target_dir.display()
    );
    println!(
        "The skill '{}' is set to autoLoad and will apply to relevant tasks.",
        skill_name
    );

    Ok(())
}
