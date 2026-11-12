//! A model asked for at spawn is chosen in the agent's session, or the spawn fails saying what the
//! agent offers.
#![cfg(unix)]

use hq_host::{Host, HostError, ReadSource, SpawnSpec, Transport};
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(25);

fn spec(name: &str, scenario: &str, model: Option<&str>) -> SpawnSpec {
    let bin = env!("CARGO_BIN_EXE_hq-host-fake-acp-agent");
    let mut spec = SpawnSpec::new(
        name,
        vec![bin.into(), scenario.into()],
        std::env::temp_dir(),
    );
    spec.transport = Transport::Acp;
    spec.model = model.map(str::to_string);
    spec
}

fn model_it_answers_on(host: &Host, name: &str) -> String {
    host.prompt(name, "which").unwrap();
    let deadline = Instant::now() + WAIT;
    while !host.info(name).unwrap().done {
        assert!(Instant::now() < deadline, "no answer in time");
        std::thread::sleep(POLL);
    }
    host.read(name, ReadSource::Recent, 0).unwrap()
}

#[test]
fn the_asked_for_model_is_chosen_in_either_menu_shape() {
    let host = Host::new();
    for (n, scenario) in ["model-set", "model-config"].into_iter().enumerate() {
        let name = format!("mdl{n}");
        host.spawn(spec(&name, scenario, Some("m-new"))).unwrap();
        assert!(model_it_answers_on(&host, &name).contains("MODEL:m-new"));
        host.kill(&name).unwrap();
    }
}

#[test]
fn no_model_asked_leaves_the_agents_own_default() {
    let host = Host::new();
    host.spawn(spec("mdl2", "model-set", None)).unwrap();
    assert!(model_it_answers_on(&host, "mdl2").contains("MODEL:m-old"));
    host.kill("mdl2").unwrap();
}

#[test]
fn a_model_the_agent_does_not_offer_fails_the_spawn_with_the_list() {
    let host = Host::new();
    let err = host
        .spawn(spec("mdl3", "model-set", Some("m-missing")))
        .unwrap_err();
    let HostError::Spawn { detail, .. } = &err else {
        panic!("unexpected {err:?}");
    };
    assert!(
        detail.contains("m-missing") && detail.contains("m-old, m-new"),
        "{detail}"
    );
    assert!(host.list().is_empty());
    let err = host.spawn(spec("mdl4", "turn", Some("m-new"))).unwrap_err();
    assert!(
        err.to_string().contains("no way to choose a model"),
        "{err}"
    );
}
