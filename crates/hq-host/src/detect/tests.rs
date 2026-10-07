use super::*;

const SAMPLE: &str = r#"
id = "demo"
aliases = ["demo-cli"]

[[rules]]
id = "busy"
state = "working"
priority = 100
region = "bottom_non_empty_lines(3)"
contains = ["Esc to Interrupt"]

[[rules]]
id = "asks"
state = "blocked"
priority = 200
regex = ['(?i)allow\s+this\?']
not = [{ contains = ["never mind"] }]

[[rules]]
id = "tie-first"
state = "unknown"
priority = 50
contains = ["tie"]

[[rules]]
id = "tie-second"
state = "working"
priority = 50
contains = ["tie"]

[[rules]]
id = "title"
state = "working"
priority = 300
region = "osc_title"
regex = ['^[⠋⠙] ']
"#;

fn demo() -> Detector {
    let mut d = Detector {
        by_id: HashMap::new(),
        aliases: HashMap::new(),
    };
    d.add(SAMPLE).unwrap();
    d
}

fn state(d: &Detector, screen: &str, title: &str) -> Option<(AgentState, Option<String>)> {
    d.detect(
        "demo",
        Input {
            screen,
            osc_title: title,
        },
    )
    .map(|x| (x.state, x.rule))
}

#[test]
fn an_agent_with_rules_and_no_match_is_idle() {
    assert_eq!(state(&demo(), "hello", ""), Some((AgentState::Idle, None)));
}

#[test]
fn an_agent_without_rules_is_not_detected() {
    assert!(
        demo()
            .detect(
                "other",
                Input {
                    screen: "x",
                    osc_title: ""
                }
            )
            .is_none()
    );
    assert!(!demo().knows("other"));
}

#[test]
fn aliases_reach_the_same_rules() {
    assert!(demo().knows("demo-cli"));
}

#[test]
fn phrases_ignore_case_and_regions_limit_where_they_count() {
    let d = demo();
    assert_eq!(
        state(&d, "x\nesc to interrupt", "").unwrap().0,
        AgentState::Working
    );
    let far = format!("esc to interrupt\n{}", "line\n".repeat(10));
    assert_eq!(state(&d, &far, "").unwrap().0, AgentState::Idle);
}

#[test]
fn the_highest_priority_wins_and_an_earlier_rule_wins_a_tie() {
    let d = demo();
    let both = "Esc to interrupt\nAllow this?";
    assert_eq!(
        state(&d, both, "").unwrap(),
        (AgentState::Blocked, Some("asks".into()))
    );
    assert_eq!(
        state(&d, "tie", "").unwrap(),
        (AgentState::Unknown, Some("tie-first".into()))
    );
}

#[test]
fn not_gates_veto_a_rule_and_the_title_is_its_own_region() {
    let d = demo();
    assert_eq!(
        state(&d, "Allow this? never mind", "").unwrap().0,
        AgentState::Idle
    );
    assert_eq!(
        state(&d, "calm", "⠋ working").unwrap(),
        (AgentState::Working, Some("title".into()))
    );
}

#[test]
fn broken_files_are_refused_with_a_reason() {
    let mut d = demo();
    assert!(
        d.add("id = 'x'\n[[rules]]\nid='r'\nregex=['(']")
            .unwrap_err()
            .contains("rule r")
    );
    assert!(d.add("id = 'x'\nbogus = 1").is_err());
    assert!(
        d.add("id = 'x'\nmin_engine_version = 99")
            .unwrap_err()
            .contains("engine version")
    );
}

#[test]
fn builtin_rule_files_load() {
    let d = Detector::builtin();
    assert!(d.knows("claude") && d.knows("claude-code") && d.knows("codex"));
}

#[test]
fn overrides_replace_built_ins_and_bad_files_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("claude.toml"), "id = \"claude\"\n[[rules]]\nid = \"always\"\nstate = \"blocked\"\npriority = 1\ncontains = [\"\"]\n").unwrap();
    std::fs::write(dir.path().join("broken.toml"), "id = ").unwrap();
    std::fs::write(dir.path().join("notes.txt"), "ignored").unwrap();
    let mut d = Detector::builtin();
    let problems = d.load_overrides(dir.path());
    assert_eq!(problems.len(), 1, "{problems:?}");
    let got = d
        .detect(
            "claude",
            Input {
                screen: "anything",
                osc_title: "",
            },
        )
        .unwrap();
    assert_eq!(got.state, AgentState::Blocked);
}
