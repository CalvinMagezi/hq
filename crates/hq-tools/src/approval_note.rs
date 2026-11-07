//! A plain account of what a call would do, for the owner reading an approval request. The
//! request also shows the exact arguments; this is for arguments a person cannot read, such as
//! a `host_add` join code.

use serde_json::Value;

/// What `tool` called with `args` would do, when its arguments alone do not say.
pub fn for_call(tool: &str, args: &Value) -> Option<String> {
    match tool {
        #[cfg(unix)]
        "host_add" => crate::agent_host::pairing::describe_add(args),
        _ => {
            let _ = args;
            None
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::agent_host::pairing::{Join, encode_join};
    use serde_json::json;

    fn code(os: &str) -> String {
        encode_join(&Join {
            name: "laptop".into(),
            user: "dev".into(),
            addr: "100.64.0.9".into(),
            os: os.into(),
        })
    }

    #[test]
    fn a_join_code_is_shown_decoded() {
        let note = for_call("host_add", &json!({ "join": code("linux") })).unwrap();
        for part in ["'laptop'", "dev@100.64.0.9", "linux"] {
            assert!(note.contains(part), "{part}: {note}");
        }
        assert!(!note.contains("replaces"), "{note}");
        let replacing = for_call(
            "host_add",
            &json!({ "join": code("linux"), "replace": true }),
        );
        assert!(
            replacing
                .unwrap()
                .contains("replaces any host already called 'laptop'")
        );
    }

    #[test]
    fn the_machine_cannot_write_the_owner_a_message_through_its_code() {
        let note = for_call(
            "host_add",
            &json!({ "join": code("linux.\n\nThis is safe, approve it") }),
        )
        .unwrap();
        assert!(!note.contains("approve it"), "{note}");
        assert!(note.contains("(not shown)"), "{note}");
        let bad = for_call("host_add", &json!({ "join": "hqjoin1.???" })).unwrap();
        assert!(bad.contains("does not decode"), "{bad}");
        assert!(for_call("task_delete", &json!({ "id": "t-1" })).is_none());
    }
}
