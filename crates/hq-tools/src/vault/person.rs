//! Source-grounded people notes (FR-074): a deterministic writer, no LLM step.
//!
//! Identity enforcement lives in `scoped.rs`, which gates this tool on the
//! person's private space like any other vault write. Ambiguous or unknown
//! audiences never reach `execute`.

use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_core::privacy::{Fact, PEOPLE_DIR, PersonId, Provenance, ProvenanceKind};
use hq_db::Database;
use hq_vault::VaultClient;
use serde_json::{Value, json};
use std::sync::Arc;

use super::sync_note_index_and_cache;
use crate::registry::HqTool;

const FACTS_HEADING: &str = "Facts";
const PROFILE_NOTE: &str = "profile";
const MAX_FACT_CHARS: usize = 300;
const MAX_FIELD_CHARS: usize = 80;
const FACT_PREFIX: &str = "- **";
const STRUCK_PREFIX: &str = "- ~~";
const TEXT_SOURCE_SEPARATOR: &str = " _(";

/// Whole-word matches; recording these needs an explicit owner decision, not an agent one.
const SENSITIVE_WORDS: &[&str] = &[
    "health",
    "medical",
    "medication",
    "diagnosis",
    "diagnosed",
    "illness",
    "disease",
    "cancer",
    "hiv",
    "pregnant",
    "pregnancy",
    "therapy",
    "therapist",
    "disability",
    "religion",
    "religious",
    "church",
    "mosque",
    "political",
    "politics",
    "party",
    "vote",
    "voted",
    "sexual",
    "sexuality",
    "gay",
    "lesbian",
    "orientation",
    "race",
    "racial",
    "ethnicity",
    "ethnic",
    "tribe",
    "immigration",
    "visa",
    "criminal",
    "arrested",
    "convicted",
    "lawsuit",
    "password",
    "passcode",
    "pin",
    "ssn",
    "passport",
    "salary",
    "debt",
    "bankruptcy",
    "iban",
    "divorce",
    "abuse",
    "addiction",
];
const SENSITIVE_PHRASES: &[&str] = &[
    "credit card",
    "bank account",
    "social security",
    "national id",
    "api key",
    "secret key",
];

pub struct PersonRecordFactTool {
    vault: Arc<VaultClient>,
    db: Option<Arc<Database>>,
    /// People known from config or the identity registry even without a folder yet.
    registered: Vec<PersonId>,
}

impl PersonRecordFactTool {
    pub fn new(vault: Arc<VaultClient>, db: Option<Arc<Database>>) -> Self {
        Self {
            vault,
            db,
            registered: Vec::new(),
        }
    }

    pub fn with_registered_people(mut self, registered: Vec<PersonId>) -> Self {
        self.registered = registered;
        self
    }

    fn is_registered(&self, person: &str) -> bool {
        self.registered.contains(&PersonId::from_name(person))
    }
}

/// Set by the scope wrapper once it has proven the caller is this registered person.
pub(super) const PERSON_REGISTERED_ARG: &str = "person_registered";
pub(super) const CREATE_PERSON_ARG: &str = "create_person";
pub(super) const CREATE_PERSON_OWNER_ONLY: &str =
    "Only the owner can create a new person profile. Ask the owner to add this person first.";

/// People named in relay config, so a family member without a folder yet still counts as existing.
pub(super) fn configured_people() -> Vec<PersonId> {
    let Ok(config) = hq_core::config::HqConfig::load() else {
        return Vec::new();
    };
    let family = config.relay.discord_family_users.iter().map(|u| &u.name);
    let telegram = config.relay.telegram_users.iter().map(|u| &u.name);
    family
        .chain(telegram)
        .map(|name| PersonId::from_name(name))
        .collect()
}

/// A path inside the person's folder, used by the scope wrapper to reuse its write gate.
pub(super) fn person_probe_path(person: &str) -> Option<String> {
    let slug = PersonId::from_name(person);
    (!slug.as_str().is_empty()).then(|| format!("{PEOPLE_DIR}/{}/{PROFILE_NOTE}.md", slug.as_str()))
}

struct FactRequest<'a> {
    person: &'a str,
    topic: &'a str,
    text: &'a str,
    kind: ProvenanceKind,
    source_ref: &'a str,
    corrects: bool,
    /// The owner deliberately allows a brand new profile.
    create_person: bool,
    /// The person is known from config or the identity registry.
    registered: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Recorded,
    Duplicate,
    Corrected(usize),
    Conflict(Vec<String>),
}

fn parse_kind(raw: &str) -> Result<ProvenanceKind> {
    match raw {
        "user_stated" => Ok(ProvenanceKind::UserStated),
        "quoted" => Ok(ProvenanceKind::Quoted),
        "inferred" => Ok(ProvenanceKind::Inferred),
        other => bail!("source_kind must be user_stated, quoted or inferred, got '{other}'"),
    }
}

fn kind_label(kind: ProvenanceKind) -> &'static str {
    match kind {
        ProvenanceKind::Config => "config",
        ProvenanceKind::Platform => "platform",
        ProvenanceKind::Vault => "vault",
        ProvenanceKind::UserStated => "user_stated",
        ProvenanceKind::Quoted => "quoted",
        ProvenanceKind::Inferred => "inferred",
    }
}

fn is_sensitive(text: &str) -> bool {
    let lower = text.to_lowercase();
    if SENSITIVE_PHRASES.iter().any(|p| lower.contains(p)) {
        return true;
    }
    lower
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| SENSITIVE_WORDS.contains(&w))
}

fn validate_field(name: &str, value: &str, max: usize) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{name} must not be empty");
    }
    if value.contains('\n') || value.contains('\r') {
        bail!("{name} must be a single line");
    }
    if value.chars().count() > max {
        bail!("{name} is longer than {max} characters");
    }
    if is_sensitive(value) {
        bail!("{name} touches a sensitive category and is not recorded by default");
    }
    Ok(())
}

fn validate(req: &FactRequest<'_>) -> Result<()> {
    if PersonId::from_name(req.person).as_str().is_empty() {
        bail!("person must name someone");
    }
    validate_field("topic", req.topic, MAX_FIELD_CHARS)?;
    validate_field("fact", req.text, MAX_FACT_CHARS)?;
    if req.source_ref.trim().is_empty() || req.source_ref.contains('\n') {
        bail!("source_ref must be a non-empty single line naming where the fact came from");
    }
    if req.corrects && req.kind == ProvenanceKind::Inferred {
        bail!("an inference cannot correct a recorded fact");
    }
    Ok(())
}

fn normalize(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Topic and text of an active (not struck through) fact line.
fn parse_active(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix(FACT_PREFIX)?;
    let (topic, rest) = rest.split_once("**: ")?;
    let text = rest
        .rsplit_once(TEXT_SOURCE_SEPARATOR)
        .map_or(rest, |(t, _)| t);
    Some((normalize(topic), normalize(text)))
}

fn render_line(req: &FactRequest<'_>, date: &str, conflict: bool) -> String {
    let fact = Fact::new(
        req.text.trim(),
        Provenance::new(req.kind, req.source_ref.trim()),
    );
    let trust = if fact.verified {
        "verified"
    } else {
        "unverified"
    };
    let tag = if conflict {
        " [conflicts with an earlier entry]"
    } else {
        ""
    };
    format!(
        "{FACT_PREFIX}{}**: {}{TEXT_SOURCE_SEPARATOR}{}, {trust}: {}; recorded {date})_{tag}",
        req.topic.trim(),
        fact.text,
        kind_label(fact.provenance.kind),
        fact.provenance.reference,
    )
}

/// Applies the request to the fact lines; returns the new lines and what happened.
fn apply(lines: &[String], req: &FactRequest<'_>, date: &str) -> (Vec<String>, Outcome) {
    let topic = normalize(req.topic);
    let text = normalize(req.text);
    let same_topic: Vec<(usize, String)> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            parse_active(l)
                .filter(|(t, _)| *t == topic)
                .map(|(_, x)| (i, x))
        })
        .collect();
    if same_topic.iter().any(|(_, t)| *t == text) {
        return (lines.to_vec(), Outcome::Duplicate);
    }
    let mut out = lines.to_vec();
    if req.corrects && !same_topic.is_empty() {
        for (i, _) in &same_topic {
            let body = out[*i].trim_start_matches("- ");
            out[*i] = format!("{STRUCK_PREFIX}{body}~~ superseded {date}");
        }
        out.push(render_line(req, date, false));
        return (out, Outcome::Corrected(same_topic.len()));
    }
    let conflict = !same_topic.is_empty();
    out.push(render_line(req, date, conflict));
    if conflict {
        let earlier = same_topic.into_iter().map(|(_, t)| t).collect();
        return (out, Outcome::Conflict(earlier));
    }
    (out, Outcome::Recorded)
}

fn person_has_notes(vault: &VaultClient, person: &PersonId) -> bool {
    vault
        .list_notes(&format!("{PEOPLE_DIR}/{}", person.as_str()))
        .is_ok_and(|notes| !notes.is_empty())
}

/// Reuse the person's existing note; only create one when their folder is empty.
fn choose_note(vault: &VaultClient, person: &PersonId) -> Result<String> {
    let slug = person.as_str();
    let dir = format!("{PEOPLE_DIR}/{slug}");
    let existing = vault.list_notes(&dir)?;
    let named = |stem: &str| {
        existing
            .iter()
            .find(|p| p.ends_with(&format!("/{stem}.md")))
    };
    if let Some(p) = named(slug).or_else(|| named(PROFILE_NOTE)) {
        return Ok(p.clone());
    }
    match existing.as_slice() {
        [] => Ok(format!("{dir}/{PROFILE_NOTE}.md")),
        [only] => Ok(only.clone()),
        _ => bail!(
            "several notes exist under {dir} and none is the profile; name one {slug}.md or {PROFILE_NOTE}.md"
        ),
    }
}

fn new_person_note(vault: &VaultClient, path: &str, person: &str) -> Result<()> {
    let mut frontmatter = std::collections::HashMap::new();
    frontmatter.insert(
        "title".into(),
        serde_yaml::Value::String(person.trim().into()),
    );
    frontmatter.insert("type".into(), serde_yaml::Value::String("person".into()));
    let note = hq_core::types::Note {
        title: person.trim().into(),
        content: format!("# {}\n\n## {FACTS_HEADING}\n", person.trim()),
        path: path.into(),
        frontmatter,
        note_type: None,
        tags: vec![],
        pinned: false,
        source: None,
        embedding_status: None,
        created_at: None,
        updated_at: None,
        modified_at: chrono::Utc::now(),
    };
    vault.write_note(path, &note)
}

fn fact_lines(vault: &VaultClient, path: &str) -> Result<Vec<String>> {
    let Some(section) = vault.read_section(path, FACTS_HEADING)? else {
        return Ok(Vec::new());
    };
    let lines = section.content.lines().skip(1);
    Ok(lines
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect())
}

fn write_lines(vault: &VaultClient, path: &str, lines: &[String]) -> Result<()> {
    let body = lines.join("\n");
    let has_section = vault.read_section(path, FACTS_HEADING)?.is_some();
    if has_section {
        return vault.patch_section(path, FACTS_HEADING, &body);
    }
    vault.append_to_note(path, &format!("## {FACTS_HEADING}\n{body}"), None)
}

/// Writes the fact, then re-reads the note to confirm it landed.
fn record_fact(
    vault: &VaultClient,
    req: &FactRequest<'_>,
    date: &str,
) -> Result<(String, Outcome)> {
    validate(req)?;
    let person = PersonId::from_name(req.person);
    if !req.create_person && !req.registered && !person_has_notes(vault, &person) {
        bail!(
            "No existing person matches '{}': there is no {PEOPLE_DIR}/{}/ folder with notes and they are not a registered person. \
             Either use the name of someone who already exists (check the spelling), or, if you are the owner and \
             really mean to add a new person, call again with create_person=true.",
            req.person.trim(),
            person.as_str()
        );
    }
    let path = choose_note(vault, &person)?;
    if !vault.note_exists(&path) {
        new_person_note(vault, &path, req.person)?;
    }
    let before = fact_lines(vault, &path)?;
    let (after, outcome) = apply(&before, req, date);
    if outcome == Outcome::Duplicate {
        return Ok((path, outcome));
    }
    write_lines(vault, &path, &after)?;
    if fact_lines(vault, &path)? != after {
        bail!("verification failed: {path} does not contain the recorded fact");
    }
    Ok((path, outcome))
}

fn str_arg<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(Value::as_str).unwrap_or_default()
}

#[async_trait]
impl HqTool for PersonRecordFactTool {
    fn name(&self) -> &str {
        "vault_person_record_fact"
    }

    fn description(&self) -> &str {
        "Record one durable, relevant fact about a person in their people note under Notebooks/People/<slug>/. \
         Needs a source. Facts are stored with provenance and date; inferences are labelled, a differing fact on \
         the same topic is kept as a visible conflict unless corrects=true, and sensitive categories (health, \
         religion, politics, finances, credentials and similar) are refused. The person must already exist \
         (a people folder with notes, or a registered person); an unknown name is refused, never created. Only the \
         owner can create a new profile, and only by passing create_person=true deliberately. Family members can \
         only record to their own note. Only mention recorded facts in a conversation when they are relevant to it."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "person": { "type": "string", "description": "Person's name or slug; must match an existing person, never guessed" },
                "topic": { "type": "string", "description": "Short label such as 'employer' or 'preferred name'" },
                "fact": { "type": "string", "description": "One factual statement, single line" },
                "source_kind": { "type": "string", "enum": ["user_stated", "quoted", "inferred"] },
                "source_ref": { "type": "string", "description": "Where it came from, e.g. a channel and message reference" },
                "corrects": { "type": "boolean", "description": "True when this replaces an earlier fact on the same topic", "default": false },
                "create_person": { "type": "boolean", "description": "Owner only: create a brand new person profile when no such person exists. Leave false unless the owner explicitly asked to add a new person", "default": false }
            },
            "required": ["person", "topic", "fact", "source_kind", "source_ref"]
        })
    }

    fn category(&self) -> &str {
        "vault"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let kind = parse_kind(str_arg(&args, "source_kind"))?;
        let corrects = args
            .get("corrects")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let (vault, db) = (self.vault.clone(), self.db.clone());
        let registered = self.is_registered(str_arg(&args, "person"))
            || args.get(PERSON_REGISTERED_ARG).and_then(Value::as_bool) == Some(true);
        let create_person = args.get(CREATE_PERSON_ARG).and_then(Value::as_bool) == Some(true);
        let args_owned = args.clone();
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        tokio::task::spawn_blocking(move || {
            let req = FactRequest {
                person: str_arg(&args_owned, "person"),
                topic: str_arg(&args_owned, "topic"),
                text: str_arg(&args_owned, "fact"),
                kind,
                source_ref: str_arg(&args_owned, "source_ref"),
                corrects,
                create_person,
                registered,
            };
            let (path, outcome) = record_fact(&vault, &req, &date)?;
            if let Ok(note) = vault.read_note(&path) {
                sync_note_index_and_cache(
                    db.as_ref(),
                    &path,
                    &note.content,
                    &note.title,
                    &note.tags,
                );
            }
            Ok(outcome_json(&path, outcome))
        })
        .await?
    }
}

fn outcome_json(path: &str, outcome: Outcome) -> Value {
    match outcome {
        Outcome::Recorded => json!({ "ok": true, "status": "recorded", "path": path }),
        Outcome::Duplicate => json!({ "ok": true, "status": "duplicate", "path": path }),
        Outcome::Corrected(n) => {
            json!({ "ok": true, "status": "corrected", "path": path, "superseded": n })
        }
        Outcome::Conflict(earlier) => {
            json!({ "ok": true, "status": "conflict", "path": path, "earlier": earlier })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::scope_vault_tools;
    use hq_core::privacy::{
        Binding, Confidence, DisclosureScope, IdentityClaim, PersonRegistry, Role, resolve,
    };

    const DATE: &str = "2026-09-30";

    fn vault() -> (tempfile::TempDir, Arc<VaultClient>) {
        let dir = tempfile::tempdir().unwrap();
        let v = Arc::new(VaultClient::new(dir.path().to_path_buf()).unwrap());
        (dir, v)
    }

    fn req<'a>(topic: &'a str, text: &'a str) -> FactRequest<'a> {
        FactRequest {
            person: "Bob",
            topic,
            text,
            kind: ProvenanceKind::UserStated,
            source_ref: "discord msg 42",
            corrects: false,
            create_person: true,
            registered: false,
        }
    }

    fn body(v: &VaultClient, path: &str) -> String {
        v.read_note(path).unwrap().content
    }

    #[test]
    fn records_fact_with_provenance_and_creates_one_profile() {
        let (_d, v) = vault();
        let (path, out) = record_fact(&v, &req("employer", "Works at Acme"), DATE).unwrap();
        assert_eq!(out, Outcome::Recorded);
        assert_eq!(path, "Notebooks/People/bob/profile.md");
        let text = body(&v, &path);
        assert!(text.contains("- **employer**: Works at Acme _(user_stated, unverified: discord msg 42; recorded 2026-09-30)_"));
    }

    #[test]
    fn inference_is_labelled_and_cannot_correct() {
        let (_d, v) = vault();
        let mut r = req("hobby", "Probably enjoys chess");
        r.kind = ProvenanceKind::Inferred;
        let (path, _) = record_fact(&v, &r, DATE).unwrap();
        assert!(body(&v, &path).contains("inferred, unverified"));
        r.corrects = true;
        assert!(record_fact(&v, &r, DATE).is_err());
    }

    #[test]
    fn correction_strikes_old_fact_and_keeps_it() {
        let (_d, v) = vault();
        record_fact(&v, &req("employer", "Works at Acme"), DATE).unwrap();
        let mut r = req("employer", "Works at Globex");
        r.corrects = true;
        let (path, out) = record_fact(&v, &r, DATE).unwrap();
        assert_eq!(out, Outcome::Corrected(1));
        let text = body(&v, &path);
        assert!(text.contains("- ~~**employer**: Works at Acme"));
        assert!(text.contains("superseded 2026-09-30"));
        assert!(text.contains("- **employer**: Works at Globex"));
    }

    #[test]
    fn conflicting_fact_is_kept_and_flagged() {
        let (_d, v) = vault();
        record_fact(&v, &req("employer", "Works at Acme"), DATE).unwrap();
        let (path, out) = record_fact(&v, &req("employer", "Works at Globex"), DATE).unwrap();
        assert_eq!(out, Outcome::Conflict(vec!["works at acme".into()]));
        let text = body(&v, &path);
        assert!(text.contains("Works at Acme"));
        assert!(text.contains("Works at Globex _(user_stated, unverified: discord msg 42; recorded 2026-09-30)_ [conflicts with an earlier entry]"));
    }

    #[test]
    fn same_fact_twice_is_a_duplicate_and_writes_nothing() {
        let (_d, v) = vault();
        let (path, _) = record_fact(&v, &req("employer", "Works at Acme"), DATE).unwrap();
        let before = body(&v, &path);
        let (_, out) = record_fact(&v, &req("Employer", "works  at acme"), DATE).unwrap();
        assert_eq!(out, Outcome::Duplicate);
        assert_eq!(body(&v, &path), before);
    }

    #[test]
    fn existing_note_is_updated_in_place_preserving_structure() {
        let (d, v) = vault();
        let dir = d.path().join("Notebooks/People/bob");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("bob.md"),
            "---\ntitle: Bob\ntags: [family]\n---\n# Bob\n\nSee [[Trip]].\n\n## Goals\n- Run a marathon\n",
        )
        .unwrap();
        let (path, _) = record_fact(&v, &req("employer", "Works at Acme"), DATE).unwrap();
        assert_eq!(path, "Notebooks/People/bob/bob.md");
        let text = body(&v, &path);
        assert!(text.contains("See [[Trip]]."));
        assert!(text.contains("## Goals\n- Run a marathon"));
        assert!(text.contains("## Facts\n- **employer**"));
        assert_eq!(v.read_note(&path).unwrap().tags, vec!["family".to_string()]);
        assert_eq!(v.list_notes("Notebooks/People/bob").unwrap().len(), 1);
    }

    #[test]
    fn ambiguous_note_folder_is_refused_rather_than_duplicated() {
        let (d, v) = vault();
        let dir = d.path().join("Notebooks/People/bob");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.md"), "# A").unwrap();
        std::fs::write(dir.join("b.md"), "# B").unwrap();
        assert!(record_fact(&v, &req("employer", "Works at Acme"), DATE).is_err());
        assert_eq!(v.list_notes("Notebooks/People/bob").unwrap().len(), 2);
    }

    #[test]
    fn sensitive_and_malformed_input_is_refused() {
        let (d, v) = vault();
        for (topic, text) in [
            ("condition", "Was diagnosed with asthma"),
            ("faith", "Attends church weekly"),
            ("money", "Salary is high"),
            ("bank", "Bank account ends 1234"),
            ("employer", "line one\nline two"),
            ("", "Works at Acme"),
        ] {
            assert!(record_fact(&v, &req(topic, text), DATE).is_err(), "{topic}");
        }
        assert!(!d.path().join("Notebooks/People/bob").exists());
        assert!(!is_sensitive("Loves to embrace change"));
    }

    fn scoped_tool(
        v: &Arc<VaultClient>,
        registry: PersonRegistry,
        account: &str,
    ) -> Vec<Box<dyn HqTool>> {
        let scope =
            DisclosureScope::for_person(&resolve(&registry, &IdentityClaim::discord(account)));
        let tools: Vec<Box<dyn HqTool>> =
            vec![Box::new(PersonRecordFactTool::new(v.clone(), None))];
        scope_vault_tools(tools, v.clone(), &scope)
    }

    fn binding(account: &str, name: &str, role: Role) -> Binding {
        Binding {
            platform: "discord".into(),
            account_id: account.into(),
            person: PersonId::from_name(name),
            name: name.into(),
            role,
            confidence: Confidence::Verified,
            provenance: Provenance::new(ProvenanceKind::Config, "test"),
        }
    }

    fn call_args(person: &str) -> Value {
        json!({ "person": person, "topic": "employer", "fact": "Works at Acme",
                "source_kind": "user_stated", "source_ref": "msg 1" })
    }

    #[tokio::test]
    async fn identity_gate_owner_member_ambiguous_and_unknown() {
        let (d, v) = vault();
        let registry = PersonRegistry::new(vec![
            binding("1", "owner", Role::Owner),
            binding("2", "Bob", Role::Member),
            binding("3", "Bob", Role::Member),
            binding("3", "Carol", Role::Member),
        ]);
        let run = |account: &'static str, person: &'static str| {
            let tools = scoped_tool(&v, registry.clone(), account);
            async move { tools[0].execute(call_args(person)).await }
        };
        assert!(run("2", "Bob").await.is_ok(), "member writes own note");
        assert!(
            run("2", "Carol").await.is_err(),
            "member cannot write another person"
        );
        assert!(run("3", "Bob").await.is_err(), "ambiguous identity refused");
        assert!(run("999", "Bob").await.is_err(), "unknown identity refused");
        assert!(
            run("1", "Carol").await.is_err(),
            "owner cannot write for a person with no profile unless creating deliberately"
        );
        let owner = scoped_tool(&v, registry.clone(), "1");
        let mut create = call_args("Carol");
        create["create_person"] = json!(true);
        assert!(owner[0].execute(create).await.is_ok(), "owner may create");
        assert!(d.path().join("Notebooks/People/bob/profile.md").exists());
        assert!(d.path().join("Notebooks/People/carol/profile.md").exists());
        assert_eq!(
            std::fs::read_dir(d.path().join("Notebooks/People"))
                .unwrap()
                .count(),
            2
        );
    }

    fn strict_req<'a>() -> FactRequest<'a> {
        FactRequest {
            create_person: false,
            ..req("employer", "Works at Acme")
        }
    }

    #[test]
    fn nonexistent_person_is_refused_with_both_ways_forward() {
        let (d, v) = vault();
        let mut r = strict_req();
        r.person = "Zzyzx Synthetic";
        let msg = record_fact(&v, &r, DATE).unwrap_err().to_string();
        assert!(msg.contains("No existing person matches 'Zzyzx Synthetic'"));
        assert!(msg.contains("check the spelling"));
        assert!(msg.contains("create_person=true"));
        assert!(!d.path().join("Notebooks/People").exists());
    }

    #[test]
    fn existing_person_folder_or_registered_person_is_accepted() {
        let (d, v) = vault();
        let dir = d.path().join("Notebooks/People/bob");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bob.md"), "# Bob\n").unwrap();
        assert!(record_fact(&v, &strict_req(), DATE).is_ok());

        let mut registered = strict_req();
        registered.person = "Carol";
        registered.registered = true;
        assert!(record_fact(&v, &registered, DATE).is_ok());
    }

    #[test]
    fn create_person_creates_a_profile_for_the_owner() {
        let (d, v) = vault();
        let mut r = strict_req();
        r.person = "New Person";
        r.create_person = true;
        record_fact(&v, &r, DATE).unwrap();
        assert!(
            d.path()
                .join("Notebooks/People/new-person/profile.md")
                .exists()
        );
    }

    #[tokio::test]
    async fn family_member_cannot_create_or_write_for_someone_else() {
        let (d, v) = vault();
        let registry = PersonRegistry::new(vec![
            binding("2", "Bob", Role::Member),
            binding("3", "Carol", Role::Member),
        ]);
        let tools = scoped_tool(&v, registry, "2");
        let mut create = call_args("Brand New");
        create["create_person"] = json!(true);
        let err = tools[0].execute(create).await.unwrap_err();
        assert_eq!(err.to_string(), CREATE_PERSON_OWNER_ONLY);
        let mut own = call_args("Bob");
        own["create_person"] = json!(true);
        assert!(
            tools[0].execute(own).await.is_err(),
            "flag is refused even for self"
        );
        assert!(tools[0].execute(call_args("Carol")).await.is_err());
        assert!(tools[0].execute(call_args("Bob")).await.is_ok());
        assert!(!d.path().join("Notebooks/People/brand-new").exists());
        assert!(!d.path().join("Notebooks/People/carol").exists());
    }
}
