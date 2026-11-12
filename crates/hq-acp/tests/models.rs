//! Choosing an agent's model: through `session/set_model` or a config option, and refusing an id
//! the agent did not offer.

use hq_acp::{ClientError, Connection, ModelChoice};
use std::process::Command;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(10);
const TICK: Duration = Duration::from_millis(25);

fn open(scenario: &str) -> Connection {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fake-acp-agent"));
    cmd.arg(scenario);
    let mut c = Connection::spawn(cmd).expect("fake agent starts");
    c.initialize("hq-test", WAIT).expect("initialize");
    c.new_session("/work", serde_json::json!([]), WAIT)
        .expect("session");
    c
}

fn model_after_a_turn(c: &mut Connection) -> String {
    c.prompt("which").expect("prompt");
    while !c.tracker.done() {
        c.pump(TICK).expect("agent alive");
    }
    c.tracker.text().to_string()
}

#[test]
fn a_model_is_chosen_through_set_model_or_a_config_option() {
    for (scenario, choice) in [
        ("model-set", ModelChoice::SetModel),
        (
            "model-config",
            ModelChoice::ConfigOption {
                config_id: "model".into(),
            },
        ),
    ] {
        let mut c = open(scenario);
        assert_eq!(c.models().unwrap().choice, choice);
        assert_eq!(c.models().unwrap().current.as_deref(), Some("m-old"));
        c.select_model("m-new", WAIT).expect("select");
        assert_eq!(model_after_a_turn(&mut c), "MODEL:m-new", "{scenario}");
    }
}

#[test]
fn the_model_it_is_on_already_is_left_alone() {
    let mut c = open("model-set");
    c.select_model("m-old", WAIT).expect("no-op");
    assert_eq!(model_after_a_turn(&mut c), "MODEL:m-old");
}

#[test]
fn an_id_the_agent_did_not_offer_lists_what_it_did() {
    let mut c = open("model-set");
    match c.select_model("m-missing", WAIT) {
        Err(ClientError::UnknownModel { wanted, available }) => {
            assert_eq!(wanted, "m-missing");
            assert_eq!(available, vec!["m-old", "m-new"]);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn an_agent_with_no_menu_cannot_be_asked() {
    let mut c = open("turn");
    assert!(c.models().is_none());
    assert!(matches!(
        c.select_model("anything", WAIT),
        Err(ClientError::NoModelChoice)
    ));
}
