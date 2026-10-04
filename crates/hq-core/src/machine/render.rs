use super::{BinaryStatus, MachineProfile};

fn named<'a>(p: &'a MachineProfile, name: &str) -> Option<&'a BinaryStatus> {
    p.binaries.iter().find(|b| b.name == name)
}

fn labelled(p: &MachineProfile, names: &[&str]) -> String {
    names
        .iter()
        .filter_map(|n| named(p, n))
        .map(|b| match &b.version {
            Some(v) => format!("{} {v}", b.name),
            None => b.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Render the prompt-facing block. Kept under ~900 bytes: this ships in
/// every system prompt on every surface.
pub fn render_markdown(p: &MachineProfile) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str("# Machine Profile\n");
    out.push_str(&format!(
        "_Probed {} by hq-daemon. Call `system_info` for live checks._\n\n",
        p.generated_at.format("%Y-%m-%d %H:%M UTC")
    ));

    let host_kind = if p.in_container {
        "container"
    } else {
        "bare metal"
    };
    out.push_str(&format!(
        "- **Host**: {} — {} {}, {} cores, {} GB, {host_kind}\n",
        p.hostname, p.os, p.arch, p.cpu_cores, p.memory_gb
    ));
    out.push_str(&format!("- **Home**: {}", p.home.display()));
    if let Some(v) = &p.vault_path {
        out.push_str(&format!(" · **Vault**: {}", v.display()));
    }
    out.push('\n');

    if let Some(git) = named(p, "git") {
        let version = git.version.clone().unwrap_or_else(|| "present".into());
        match &p.git_user {
            Some(u) => out.push_str(&format!("- **Git**: {version} ({u})\n")),
            None => out.push_str(&format!("- **Git**: {version}\n")),
        }
    }

    match (&named(p, "gh"), &p.gh_auth) {
        (Some(gh), Some(auth)) => {
            let version = gh.version.clone().unwrap_or_else(|| "present".into());
            out.push_str(&format!(
                "- **GitHub**: gh {version} — authenticated as {auth}\n"
            ));
        }
        (Some(gh), None) => {
            let version = gh.version.clone().unwrap_or_else(|| "present".into());
            let state = if p.deep_probed {
                "installed but NOT authenticated — run `gh auth login`"
            } else {
                "installed; auth not yet checked — call `system_info` with check `gh_auth`"
            };
            out.push_str(&format!("- **GitHub**: gh {version} {state}\n"));
        }
        (None, _) => {}
    }

    if let Some(docker) = named(p, "docker") {
        let version = docker.version.clone().unwrap_or_else(|| "present".into());
        let state = match (p.deep_probed, p.docker_running) {
            (_, true) => "daemon running",
            (true, false) => "daemon not reachable",
            (false, false) => "daemon state not yet checked",
        };
        out.push_str(&format!("- **Containers**: docker {version}, {state}\n"));
    }

    let langs = labelled(p, &["cargo", "rustc", "node", "bun", "python3", "uv"]);
    if !langs.is_empty() {
        out.push_str(&format!("- **Languages**: {langs}\n"));
    }

    let utils: Vec<&str> = [
        "gws", "rg", "jq", "fd", "herdr", "sqlite3", "ffmpeg", "curl", "ollama", "npm", "pnpm",
        "gcloud", "aws", "vercel", "psql",
    ]
    .into_iter()
    .filter(|n| named(p, n).is_some())
    .collect();
    if !utils.is_empty() {
        out.push_str(&format!("- **Utilities**: {}\n", utils.join(", ")));
    }

    if !p.missing.is_empty() {
        out.push_str(&format!("- **Missing**: {}\n", p.missing.join(", ")));
    }
    out.push_str(&render_web_search(p));

    out.push_str(
        "\nShell out to any listed binary with `bash`. Never claim a capability listed \
         under **Missing** — install it first or say it is unavailable.\n",
    );
    out
}

/// Computed on both probe paths, so unlike `gh_auth` it needs no
/// `deep_probed` gate. Only configured backends are named, each with what
/// was actually verified, so a bare API key never reads as a working search.
fn render_web_search(p: &MachineProfile) -> String {
    let usable: Vec<&str> = p
        .web_search
        .iter()
        .filter(|s| s.usable())
        .map(|s| s.detail.as_str())
        .collect();
    if !usable.is_empty() {
        return format!(
            "- **Web search**: {}. Each `web_search` result names the backend that answered.\n",
            usable.join("; ")
        );
    }
    if p.web_search.is_empty()
        && let Some(backend) = &p.web_search_backend
    {
        return format!("- **Web search**: available via {backend}\n");
    }
    if p.can_build_self {
        return "- **Web search**: UNAVAILABLE: no backend reachable. Run \
             `scripts/setup-searxng.sh` or set `brave_api_key` (or `HQ_BRAVE_API_KEY`).\n"
            .into();
    }
    "- **Web search**: UNAVAILABLE: no backend reachable. Ask the operator to \
     provision one (a SearxNG instance or a Brave Search API key), since this host has \
     no source checkout to run the setup script from.\n"
        .into()
}
