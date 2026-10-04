//! External CLI harness runner: one strict subprocess invocation.

use anyhow::Result;
use std::path::Path;

/// Run an external CLI as a single invocation, rejecting a non-zero process
/// exit even when the process wrote text to stdout or stderr, so the caller
/// can drive retry or provider fallback from the exit status.
pub async fn run_external_cli_harness_strict(
    binary: &std::path::Path,
    extra_args: &[&str],
    prompt_arg: &str,
    cwd: Option<&Path>,
    timeout_secs: u64,
) -> Result<String> {
    use tokio::process::Command;

    let mut cmd = Command::new(binary);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    for arg in extra_args {
        cmd.arg(arg);
    }
    cmd.arg(prompt_arg);
    cmd.kill_on_drop(true);

    let output = match tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        cmd.output(),
    )
    .await
    {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => anyhow::bail!(
            "external harness: failed to spawn {}: {e}",
            binary.display()
        ),
        Err(_) => anyhow::bail!(
            "external harness: {} timed out after {timeout_secs}s",
            binary.display()
        ),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        anyhow::bail!(
            "external harness: {} exited with {}{}{}",
            binary.display(),
            output.status,
            if stderr.is_empty() { "" } else { ": stderr: " },
            if stderr.is_empty() {
                stdout.as_str()
            } else {
                stderr.as_str()
            },
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn strict_runner_rejects_nonzero_exit_even_with_output() {
        let err = run_external_cli_harness_strict(
            Path::new("/bin/sh"),
            &[
                "-c",
                "echo partial-output; echo authentication failed >&2; exit 7",
            ],
            "prompt",
            None,
            30,
        )
        .await
        .expect_err("non-zero CLI exit must be an error");

        let message = err.to_string();
        assert!(message.contains("exited with"));
        assert!(message.contains("authentication failed"));
    }
}
