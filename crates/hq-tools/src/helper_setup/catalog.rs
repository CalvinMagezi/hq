//! The helpers HQ can set up for the owner, and exactly how. Every command here is fixed in this
//! file: a model, a chat or a web request names a helper by id and never supplies a command. The
//! install lines are the vendors' own documented installers; they download and run vendor code on
//! the owner's computer, which is why setting up needs the owner's one approval (`docs/HELPER_SETUP.md`).

/// One way to install a helper. The first whose `needs` are all on the computer is used.
#[derive(Debug)]
pub struct Install {
    /// Programs this recipe runs.
    pub needs: &'static [&'static str],
    /// Run by `bash -c`, from a fixed string.
    pub script: &'static str,
    /// What the owner is told it does.
    pub label: &'static str,
}

/// How a helper's sign-in is asked about.
#[derive(Debug)]
pub struct Status {
    /// Exit status zero and no "not signed in" wording means signed in.
    pub argv: &'static [&'static str],
}

#[derive(Debug)]
pub struct Helper {
    /// The harness name HQ starts sessions with (`harness_session_spawn`).
    pub id: &'static str,
    pub title: &'static str,
    /// One line in the owner's words: what it is for.
    pub blurb: &'static str,
    /// What is known about starting free. Never a promise: the vendor sets and changes the limits.
    pub free_note: &'static str,
    /// Program names to look for, the first being the one HQ starts.
    pub programs: &'static [&'static str],
    pub installs: &'static [Install],
    /// Starts the sign-in on the computer: it opens the vendor's page in the browser there, or
    /// prints a link when there is none. Exit status zero means the sign-in worked.
    pub login: &'static [&'static str],
    pub status: Option<Status>,
}

pub const CATALOG: &[Helper] = &[
    Helper {
        id: "cursor",
        title: "Cursor",
        blurb: "Coding helper from Cursor. Good for building and fixing a project end to end.",
        free_note: "You can sign in with a free Cursor account to try it. Cursor sets the limits and can change them.",
        programs: &["cursor-agent", "agent"],
        installs: &[Install {
            needs: &["curl", "bash"],
            script: "curl https://cursor.com/install -fsS | bash",
            label: "Cursor's own installer (cursor.com/install)",
        }],
        login: &["cursor-agent", "login"],
        status: Some(Status {
            argv: &["cursor-agent", "status"],
        }),
    },
    Helper {
        id: "codex",
        title: "Codex",
        blurb: "Coding helper from OpenAI. Signs in with a ChatGPT account.",
        free_note: "It works with a ChatGPT account. What a free account includes is up to OpenAI and changes.",
        programs: &["codex"],
        installs: &[
            Install {
                needs: &["brew"],
                script: "brew install --cask codex",
                label: "Homebrew (brew install --cask codex)",
            },
            Install {
                needs: &["npm"],
                script: "npm install -g --prefix \"$HOME/.local\" @openai/codex",
                label: "npm, into your home folder (@openai/codex)",
            },
        ],
        login: &["codex", "login"],
        status: Some(Status {
            argv: &["codex", "login", "status"],
        }),
    },
    Helper {
        id: "antigravity",
        title: "Antigravity",
        blurb: "Coding helper from Google. Signs in with a Google account.",
        free_note: "You can sign in with a free Google account. Google sets the limits and can change them.",
        programs: &["agy"],
        installs: &[Install {
            needs: &["curl", "bash"],
            script: "curl -fsSL https://antigravity.google/cli/install.sh | bash",
            label: "Google's own installer (antigravity.google/cli)",
        }],
        // It has no sign-in command: it asks the first time something needs the model.
        login: &["agy", "-p", "say hi"],
        status: None,
    },
];

pub fn find(id: &str) -> Option<&'static Helper> {
    CATALOG.iter().find(|h| h.id == id)
}

/// Every program name the catalog looks for.
pub fn all_programs() -> Vec<&'static str> {
    let mut names: Vec<&str> = CATALOG
        .iter()
        .flat_map(|h| {
            h.programs
                .iter()
                .chain(h.installs.iter().flat_map(|i| i.needs))
        })
        .copied()
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_helper_is_a_harness_hq_can_start() {
        for h in CATALOG {
            let spec = crate::harness_session::spec::SPECS
                .iter()
                .find(|s| s.harness == h.id)
                .unwrap_or_else(|| panic!("{} is not a known harness", h.id));
            assert!(
                h.programs.contains(&spec.binary()),
                "{}: HQ starts `{}`, which the catalog must look for",
                h.id,
                spec.binary()
            );
        }
    }

    #[test]
    fn recipes_are_fixed_strings_with_no_placeholders() {
        for h in CATALOG {
            for i in h.installs {
                assert!(
                    !i.script.contains("{}") && !i.script.contains("{{"),
                    "{}",
                    h.id
                );
                assert!(!i.needs.is_empty(), "{}", h.id);
            }
            assert!(!h.login.is_empty());
            assert!(!h.free_note.to_lowercase().contains("guarantee"));
        }
    }
}
