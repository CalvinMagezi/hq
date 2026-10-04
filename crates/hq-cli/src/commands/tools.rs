use anyhow::Result;
use hq_core::config::HqConfig;
use std::process::Command;

/// Check CLI tools status.
pub async fn run(_config: &HqConfig) -> Result<()> {
    println!("\nCLI Tools Setup");
    println!("===============\n");

    // LLM Router
    println!("── LLM Router ──");
    println!("  [OK] Built-in LLM router (external harnesses retired)");
    println!("       Supports: OpenRouter, Anthropic, Google AI, Ollama");

    // DrawIt
    println!("\n── DrawIt CLI ──");
    match Command::new("drawit").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            println!("  [OK] DrawIt: {}", version);
        }
        _ => {
            println!("  [--] DrawIt: not installed (optional, for diagrams)");
            println!("       Install: npm install -g @chamuka-labs/drawit-cli");
        }
    }

    // Bun
    println!("\n── Bun ──");
    match Command::new("bun").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            println!("  [OK] Bun: {}", version);
        }
        _ => {
            println!("  [--] Bun: not installed");
            println!("       Install: https://bun.sh");
        }
    }

    println!();
    Ok(())
}
