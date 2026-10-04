use anyhow::Result;
use hq_core::config::HqConfig;
use hq_core::hardware::{GpuType, detect_hardware, recommend_models};
use std::process::Command;

/// TurboQuant sits behind an hq-llm feature, so a default build has no local route on CUDA.
const CUDA_NOTE: &str = "CUDA GPU detected, so no Ollama models are recommended. Local inference \
here runs through TurboQuant, which needs hq built with `--features hq-llm/turboquant`.";

/// Manage local models for inference.
pub async fn run(_config: &HqConfig, sub: &str) -> Result<()> {
    match sub {
        "status" => status().await,
        "setup" => setup().await,
        "recommend" => recommend(),
        _ => {
            println!("Usage: hq models <status|setup|recommend>");
            println!();
            println!("  status     Show installed vs recommended models");
            println!("  setup      Pull recommended models for your hardware");
            println!("  recommend  Show what would be pulled (dry run)");
            Ok(())
        }
    }
}

/// Show installed models vs recommended.
async fn status() -> Result<()> {
    let profile = detect_hardware();
    println!(
        "Hardware: {} total, {}GB usable for models",
        format_hardware(&profile),
        profile.usable_for_models_gb,
    );
    println!();

    let recommended = recommend_models(&profile);
    if recommended.is_empty() {
        println!("{CUDA_NOTE}");
        return Ok(());
    }

    let installed = get_installed_models();

    println!("{:<10} {:<20} {:<8} STATUS", "ALIAS", "MODEL", "VRAM");
    println!("{}", "-".repeat(55));

    for rec in &recommended {
        let is_installed = installed
            .iter()
            .any(|m| models_match(&m.name, &rec.ollama_model));
        let status = if is_installed { "[OK]" } else { "[MISSING]" };
        println!(
            "{:<10} {:<20} {:<8} {}",
            rec.alias,
            rec.ollama_model,
            format!("~{}GB", rec.vram_gb),
            status,
        );
    }

    let missing: Vec<_> = recommended
        .iter()
        .filter(|r| {
            !installed
                .iter()
                .any(|m| models_match(&m.name, &r.ollama_model))
        })
        .collect();

    println!();
    if missing.is_empty() {
        println!("All recommended models installed.");
    } else {
        println!(
            "{} missing model(s). Run `hq models setup` to pull them.",
            missing.len()
        );
    }

    Ok(())
}

/// Pull recommended models via Ollama.
async fn setup() -> Result<()> {
    let profile = detect_hardware();
    let recommended = recommend_models(&profile);

    if recommended.is_empty() {
        println!("{CUDA_NOTE}");
        return Ok(());
    }

    // Check Ollama is running
    if !ollama_is_reachable() {
        println!("Ollama is not running. Start it with: ollama serve");
        return Ok(());
    }

    let installed = get_installed_models();

    // Deduplicate models (fast and bulk often share the same model)
    let mut to_pull: Vec<String> = Vec::new();
    for rec in &recommended {
        if !installed
            .iter()
            .any(|m| models_match(&m.name, &rec.ollama_model))
            && !to_pull.contains(&rec.ollama_model)
        {
            to_pull.push(rec.ollama_model.clone());
        }
    }

    if to_pull.is_empty() {
        println!("All recommended models already installed.");
        return Ok(());
    }

    println!(
        "Pulling {} model(s) for {} ({}GB usable)...",
        to_pull.len(),
        format_hardware(&profile),
        profile.usable_for_models_gb,
    );
    println!();

    for model in &to_pull {
        println!("Pulling {}...", model);
        let status = Command::new("ollama").args(["pull", model]).status();

        match status {
            Ok(s) if s.success() => println!("  {} pulled successfully.\n", model),
            Ok(s) => println!("  Failed to pull {} (exit code: {:?})\n", model, s.code()),
            Err(e) => println!("  Error pulling {}: {}\n", model, e),
        }
    }

    println!("Done. Run `hq models status` to verify.");
    Ok(())
}

/// Dry-run: show what would be pulled.
fn recommend() -> Result<()> {
    let profile = detect_hardware();
    println!("Hardware: {}", format_hardware(&profile));
    println!("Total memory: {}GB", profile.total_memory_gb);
    println!("Usable for models: {}GB", profile.usable_for_models_gb);
    match &profile.gpu_type {
        GpuType::AppleSilicon { chip } => println!("GPU: {}", chip),
        GpuType::NvidiaCuda { vram_gb } => println!("GPU: NVIDIA CUDA ({}GB VRAM)", vram_gb),
        GpuType::None => println!("GPU: None detected"),
    }
    println!();

    let recommended = recommend_models(&profile);
    if recommended.is_empty() {
        println!("CUDA GPU detected. Use TurboQuant for local inference.");
        return Ok(());
    }

    println!("Recommended models:");
    println!();
    println!("{:<10} {:<22} {:<8} ROLE", "ALIAS", "MODEL", "VRAM");
    println!("{}", "-".repeat(60));

    for rec in &recommended {
        println!(
            "{:<10} {:<22} {:<8} {}",
            rec.alias,
            rec.ollama_model,
            format!("~{}GB", rec.vram_gb),
            rec.role,
        );
    }

    println!();
    println!("Run `hq models setup` to pull these models.");
    Ok(())
}

/// Check if two model names match (handles tag variations).
fn models_match(installed: &str, recommended: &str) -> bool {
    // Exact match
    if installed == recommended {
        return true;
    }
    // installed might have ":latest" suffix
    let installed_base = installed.split(':').next().unwrap_or(installed);
    let rec_base = recommended.split(':').next().unwrap_or(recommended);
    if installed_base == rec_base {
        // Check if the size tag matches
        let installed_tag = installed.split(':').nth(1).unwrap_or("latest");
        let rec_tag = recommended.split(':').nth(1).unwrap_or("latest");
        return installed_tag == rec_tag;
    }
    false
}

fn format_hardware(profile: &hq_core::hardware::HardwareProfile) -> String {
    match &profile.gpu_type {
        GpuType::AppleSilicon { chip } => format!("{} ({}GB)", chip, profile.total_memory_gb),
        GpuType::NvidiaCuda { vram_gb } => {
            format!(
                "NVIDIA CUDA ({}GB VRAM, {}GB RAM)",
                vram_gb, profile.total_memory_gb
            )
        }
        GpuType::None => format!("CPU ({}GB RAM)", profile.total_memory_gb),
    }
}

struct InstalledModel {
    name: String,
}

/// Get installed Ollama models via `ollama list`.
fn get_installed_models() -> Vec<InstalledModel> {
    match Command::new("ollama").arg("list").output() {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            text.lines()
                .skip(1) // header
                .filter_map(|line| {
                    let name = line.split_whitespace().next()?;
                    Some(InstalledModel {
                        name: name.to_string(),
                    })
                })
                .collect()
        }
        _ => vec![],
    }
}

fn ollama_is_reachable() -> bool {
    use std::net::TcpStream;
    use std::time::Duration;
    TcpStream::connect_timeout(
        &"127.0.0.1:11434".parse().unwrap(),
        Duration::from_millis(500),
    )
    .is_ok()
}
