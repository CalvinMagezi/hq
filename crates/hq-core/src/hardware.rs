//! Hardware detection and model recommendation for local inference.
//!
//! Detects available memory and GPU type, then recommends optimal local
//! models that fit the hardware. Used by the LLM router and CLI.

use serde::{Deserialize, Serialize};
use std::process::Command;

/// GPU type detected on the system.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum GpuType {
    AppleSilicon { chip: String },
    NvidiaCuda { vram_gb: u64 },
    None,
}

/// Hardware profile for the current machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareProfile {
    pub total_memory_gb: u64,
    pub gpu_type: GpuType,
    /// Memory available for model loading (total minus OS overhead).
    pub usable_for_models_gb: u64,
}

/// A recommended model for a specific role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendedModel {
    /// Router alias (e.g. "fast", "mid", "memory").
    pub alias: String,
    /// Ollama model tag (e.g. "gemma4:e4b").
    pub ollama_model: String,
    /// Human-readable role description.
    pub role: String,
    /// Approximate VRAM usage in GB at Q4_K_M.
    pub vram_gb: f64,
}

/// Detect hardware profile for the current machine.
pub fn detect_hardware() -> HardwareProfile {
    let total_memory_gb = detect_total_memory_gb();
    let gpu_type = detect_gpu_type();

    // Reserve ~5GB for OS and apps on macOS, ~3GB on Linux
    let os_overhead = if cfg!(target_os = "macos") { 5 } else { 3 };
    let usable = total_memory_gb.saturating_sub(os_overhead);

    HardwareProfile {
        total_memory_gb,
        gpu_type,
        usable_for_models_gb: usable,
    }
}

/// Recommend models that fit the detected hardware.
pub fn recommend_models(profile: &HardwareProfile) -> Vec<RecommendedModel> {
    // CUDA machines: preserve TurboQuant path, don't recommend Ollama models
    if matches!(profile.gpu_type, GpuType::NvidiaCuda { .. }) {
        return vec![];
    }

    let usable = profile.usable_for_models_gb;

    if usable >= 26 {
        // 32GB+ Mac: Qwen3-14B relay/mid (native tool calling), Qwen3-8B fast
        vec![
            RecommendedModel {
                alias: "fast".into(),
                ollama_model: "qwen3:8b".into(),
                role: "Fast inference".into(),
                vram_gb: 5.2,
            },
            RecommendedModel {
                alias: "bulk".into(),
                ollama_model: "qwen3:8b".into(),
                role: "Bulk processing".into(),
                vram_gb: 5.2,
            },
            RecommendedModel {
                alias: "mid".into(),
                ollama_model: "qwen3:14b".into(),
                role: "Balanced quality/speed — native tool calling".into(),
                vram_gb: 9.0,
            },
            RecommendedModel {
                alias: "memory".into(),
                ollama_model: "qwen3:8b".into(),
                role: "Memory operations".into(),
                vram_gb: 5.2,
            },
        ]
    } else if usable >= 18 {
        // 24GB Mac (M4): Qwen3-14B relay/mid (~9GB), Qwen3-8B fast (~5GB)
        // Qwen3 has native OpenAI function calling — no prompted_tools shim needed.
        vec![
            RecommendedModel {
                alias: "fast".into(),
                ollama_model: "qwen3:8b".into(),
                role: "Fast inference".into(),
                vram_gb: 5.2,
            },
            RecommendedModel {
                alias: "bulk".into(),
                ollama_model: "qwen3:8b".into(),
                role: "Bulk processing".into(),
                vram_gb: 5.2,
            },
            RecommendedModel {
                alias: "mid".into(),
                ollama_model: "qwen3:14b".into(),
                role: "Balanced quality/speed — native tool calling".into(),
                vram_gb: 9.0,
            },
            RecommendedModel {
                alias: "memory".into(),
                ollama_model: "qwen3:8b".into(),
                role: "Memory operations".into(),
                vram_gb: 5.2,
            },
        ]
    } else if usable >= 10 {
        // 16GB Mac: Qwen3-8B mid, Qwen3-1.7B fast
        vec![
            RecommendedModel {
                alias: "fast".into(),
                ollama_model: "qwen3:1.7b".into(),
                role: "Fast inference".into(),
                vram_gb: 1.3,
            },
            RecommendedModel {
                alias: "bulk".into(),
                ollama_model: "qwen3:1.7b".into(),
                role: "Bulk processing".into(),
                vram_gb: 1.3,
            },
            RecommendedModel {
                alias: "mid".into(),
                ollama_model: "qwen3:8b".into(),
                role: "Balanced quality/speed".into(),
                vram_gb: 5.2,
            },
            RecommendedModel {
                alias: "memory".into(),
                ollama_model: "qwen3:1.7b".into(),
                role: "Memory operations".into(),
                vram_gb: 1.3,
            },
        ]
    } else {
        // 8GB or less: Qwen3-1.7B only
        vec![
            RecommendedModel {
                alias: "fast".into(),
                ollama_model: "qwen3:1.7b".into(),
                role: "Fast inference".into(),
                vram_gb: 1.3,
            },
            RecommendedModel {
                alias: "bulk".into(),
                ollama_model: "qwen3:1.7b".into(),
                role: "Bulk processing".into(),
                vram_gb: 1.3,
            },
            RecommendedModel {
                alias: "mid".into(),
                ollama_model: "qwen3:1.7b".into(),
                role: "Balanced quality/speed".into(),
                vram_gb: 1.3,
            },
            RecommendedModel {
                alias: "memory".into(),
                ollama_model: "qwen3:1.7b".into(),
                role: "Memory operations".into(),
                vram_gb: 1.3,
            },
        ]
    }
}

/// Detect total system memory in GB.
fn detect_total_memory_gb() -> u64 {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = Command::new("sysctl").args(["-n", "hw.memsize"]).output()
            && output.status.success()
        {
            let bytes_str = String::from_utf8_lossy(&output.stdout);
            if let Ok(bytes) = bytes_str.trim().parse::<u64>() {
                return bytes / (1024 * 1024 * 1024);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
            for line in content.lines() {
                if line.starts_with("MemTotal:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if let Some(kb_str) = parts.get(1)
                        && let Ok(kb) = kb_str.parse::<u64>()
                    {
                        return kb / (1024 * 1024);
                    }
                }
            }
        }
    }

    // Fallback: assume 16GB
    16
}

/// Detect GPU type.
fn detect_gpu_type() -> GpuType {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
            && output.status.success()
        {
            let brand = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if brand.contains("Apple") {
                // Extract chip name from brand string (e.g. "Apple M4 Pro" → "M4 Pro")
                let chip = brand.replace("Apple ", "");
                let chip = if chip.is_empty() {
                    "Apple Silicon".into()
                } else {
                    chip
                };
                return GpuType::AppleSilicon { chip };
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Check for NVIDIA GPU via nvidia-smi
        if let Ok(output) = Command::new("nvidia-smi")
            .args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"])
            .output()
            && output.status.success()
        {
            let vram_str = String::from_utf8_lossy(&output.stdout);
            if let Ok(vram_mb) = vram_str.trim().lines().next().unwrap_or("0").parse::<u64>() {
                return GpuType::NvidiaCuda {
                    vram_gb: vram_mb / 1024,
                };
            }
        }
    }

    GpuType::None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recommend_24gb() {
        let profile = HardwareProfile {
            total_memory_gb: 24,
            gpu_type: GpuType::AppleSilicon { chip: "M4".into() },
            usable_for_models_gb: 19,
        };
        let models = recommend_models(&profile);
        assert!(!models.is_empty());
        let fast = models.iter().find(|m| m.alias == "fast").unwrap();
        assert_eq!(fast.ollama_model, "qwen3:8b");
        let mid = models.iter().find(|m| m.alias == "mid").unwrap();
        assert_eq!(mid.ollama_model, "qwen3:14b");
    }

    #[test]
    fn test_recommend_32gb() {
        let profile = HardwareProfile {
            total_memory_gb: 32,
            gpu_type: GpuType::AppleSilicon {
                chip: "M4 Pro".into(),
            },
            usable_for_models_gb: 27,
        };
        let models = recommend_models(&profile);
        let mid = models.iter().find(|m| m.alias == "mid").unwrap();
        assert_eq!(mid.ollama_model, "qwen3:14b");
    }

    #[test]
    fn test_cuda_returns_empty() {
        let profile = HardwareProfile {
            total_memory_gb: 64,
            gpu_type: GpuType::NvidiaCuda { vram_gb: 24 },
            usable_for_models_gb: 61,
        };
        let models = recommend_models(&profile);
        assert!(models.is_empty());
    }

    #[test]
    fn test_detect_hardware_runs() {
        // Just verify it doesn't panic
        let profile = detect_hardware();
        assert!(profile.total_memory_gb > 0);
    }
}
