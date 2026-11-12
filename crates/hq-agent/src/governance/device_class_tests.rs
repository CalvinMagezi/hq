//! A made-up device domain (`hq_tools::device_actions::fake`) goes through governance and the
//! agent wrapper with no edit to either: the risk class alone decides what each tool does.

use std::path::PathBuf;
use std::sync::Arc;

use hq_tools::device_actions::fake::*;
use hq_tools::device_actions::store::DeviceActionStore;
use hq_tools::device_actions::{DeviceLink, PhoneClock, spec_for};
use serde_json::{Value, json};

use super::tainted_calls::{always_ask_reason, tainted_reason, unasked_denial};
use super::{LiveUserTurn, SecurityProfile, TaintTracker, ToolGuardian, is_untrusted_source};
use crate::device_tool::DeviceActionTool;
use crate::tools::AgentTool;

fn tainted() -> TaintTracker {
    let taint = TaintTracker::new();
    taint.mark("web_search");
    taint
}

fn args() -> Value {
    json!({"target": "Alex"})
}

#[test]
fn a_read_of_any_domain_taints_the_session_and_nothing_else_does() {
    register_fake();
    assert!(is_untrusted_source(FAKE_READ, "device"));
    for tool in [FAKE_ADD, FAKE_DELETE, FAKE_SEND] {
        assert!(!is_untrusted_source(tool, "device"), "{tool}");
    }
    let taint = TaintTracker::new();
    assert_eq!(tainted_reason(FAKE_ADD, &args(), &taint), None);
    taint.mark(FAKE_READ);
    assert!(tainted_reason(FAKE_ADD, &args(), &taint).is_some());
}

#[test]
fn a_create_asks_only_when_tainted_and_never_runs_unasked_once_tainted_and_unattended() {
    register_fake();
    assert_eq!(always_ask_reason(FAKE_ADD, &args()), None);
    assert_eq!(
        unasked_denial(FAKE_ADD, &args(), &TaintTracker::new(), false),
        None
    );
    assert_eq!(unasked_denial(FAKE_ADD, &args(), &tainted(), false), None);
    let unattended = tainted();
    unattended.set_unattended();
    assert!(unasked_denial(FAKE_ADD, &args(), &unattended, false).is_some());
}

#[test]
fn a_destructive_or_outbound_call_always_asks_and_never_runs_with_nobody_to_ask() {
    register_fake();
    for tool in [FAKE_DELETE, FAKE_SEND] {
        let reason = always_ask_reason(tool, &args()).expect(tool);
        assert!(reason.contains("Alex"), "{reason}");
        // Clean, tainted and unattended alike.
        assert_eq!(tainted_reason(tool, &args(), &TaintTracker::new()), None);
        for taint in [TaintTracker::new(), tainted()] {
            let denial = unasked_denial(tool, &args(), &taint, false).expect(tool);
            assert!(denial.contains("nobody is here to approve"), "{denial}");
        }
    }
    assert_eq!(always_ask_reason(FAKE_READ, &args()), None);
}

fn link() -> Arc<DeviceLink> {
    let phone = PhoneClock::parse("Europe/London", "2026-01-10T12:00:00Z").unwrap();
    Arc::new(DeviceLink::new(
        "thread-1",
        Arc::new(DeviceActionStore::default()),
        Arc::new(|_| {}),
        phone,
    ))
}

fn tool(name: &str) -> Box<dyn AgentTool> {
    Box::new(DeviceActionTool::new(spec_for(name).expect(name), link()))
}

#[test]
fn an_unattended_session_is_never_offered_a_device_tool_of_any_class() {
    register_fake();
    let guardian =
        ToolGuardian::with_default_mode(vec![PathBuf::from(".")], SecurityProfile::Guarded);
    let tools = [FAKE_READ, FAKE_ADD, FAKE_DELETE, FAKE_SEND]
        .map(tool)
        .into();
    let registry = guardian.build_registry(tools, LiveUserTurn::unattended());
    assert_eq!(registry.len(), 0, "outbound included: it needs a live turn");
}

#[test]
fn the_wrapper_reads_name_schema_and_read_only_from_the_spec() {
    register_fake();
    let read = tool(FAKE_READ);
    assert_eq!(read.name(), FAKE_READ);
    assert!(read.is_read_only());
    assert!(read.requires_live_user_turn());
    assert!(
        read.description()
            .starts_with("A fake device tool for tests. ")
    );
    assert!(read.description().contains("Europe/London"));
    for name in [FAKE_ADD, FAKE_DELETE, FAKE_SEND] {
        assert!(!tool(name).is_read_only(), "{name}");
    }
}

#[tokio::test]
async fn the_wrapper_fails_closed_without_the_skill_and_sends_nothing() {
    register_fake();
    let (tx, mut frames) = tokio::sync::mpsc::unbounded_channel();
    let phone = PhoneClock::parse("Europe/London", "2026-01-10T12:00:00Z").unwrap();
    let emit = Arc::new(move |frame: Value| {
        let _ = tx.send(frame);
    });
    let store = Arc::new(DeviceActionStore::default());
    let link = Arc::new(DeviceLink::new("thread-1", store, emit, phone));
    let wrapper = DeviceActionTool::new(spec_for(FAKE_SEND).unwrap(), link);
    let result = wrapper.execute("call-1", json!({})).await.unwrap();
    let outcome: Value = serde_json::from_str(&result.content[0].text).unwrap();
    assert_eq!(outcome["outcome"], "fake_unavailable");
    assert_eq!(outcome["reason"], "skill_not_active");
    assert!(frames.try_recv().is_err());
}
