//! Protocol version 1, pinned line by line.
//!
//! The host and its clients ship separately: a phone on the stable channel can drive a laptop
//! that follows `main`, and the reverse. A request or reply that changes shape without a bump of
//! `PROTOCOL_VERSION` breaks one of them with nothing in a test to say so, because every other
//! test runs the current client against the current host. This one sends version 1's request
//! lines as text and checks the reply lines for exactly the keys a version 1 client reads.
//!
//! If this fails, either the change is backward compatible (add a method or a key, and add it to
//! the list here) or the shape changed and `PROTOCOL_VERSION` must be bumped with a way for the
//! old client to be told. Do not edit a line below to make it pass.
#![cfg(unix)]

use hq_host::{Host, Server, load_or_create_token, socket_path};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Duration;

fn keys(value: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap_or_else(|| panic!("not an object: {value}"))
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

struct Wire {
    reader: BufReader<UnixStream>,
}

impl Wire {
    fn send(&mut self, line: &str) -> Value {
        let stream = self.reader.get_mut();
        stream.write_all(line.as_bytes()).unwrap();
        stream.write_all(b"\n").unwrap();
        let mut reply = String::new();
        self.reader.read_line(&mut reply).unwrap();
        serde_json::from_str(&reply).unwrap_or_else(|e| panic!("not one JSON line ({e}): {reply}"))
    }
}

#[test]
fn version_1_request_and_reply_lines_keep_their_shape() {
    let dir = tempfile::tempdir().unwrap();
    let run = dir.path().join("run");
    let server = Server::bind(&run, Arc::new(Host::new())).unwrap();
    let stop = server.stop_handle();
    let thread = std::thread::spawn(move || server.serve());
    let token = load_or_create_token(&run).unwrap();

    let stream = UnixStream::connect(socket_path(&run)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut wire = Wire {
        reader: BufReader::new(stream),
    };

    // Before hello, nothing but hello is answered.
    let early = wire.send(r#"{"id":1,"method":"host.status","params":{}}"#);
    assert_eq!(keys(&early), ["error", "id"]);
    assert_eq!(early["id"], 1);
    assert_eq!(keys(&early["error"]), ["code", "message"]);
    assert_eq!(early["error"]["code"], "unauthenticated");

    // A wrong version is refused with a code the old client can read.
    let stale = wire.send(&format!(
        r#"{{"id":"a","method":"hello","params":{{"protocol_version":0,"token":"{}"}}}}"#,
        token.trim()
    ));
    assert_eq!(stale["id"], "a");
    assert_eq!(stale["error"]["code"], "protocol_mismatch");

    // Hello: a number or a string id comes back as sent, the reply holds exactly these keys.
    let hello = wire.send(&format!(
        r#"{{"id":2,"method":"hello","params":{{"protocol_version":1,"token":"{}"}}}}"#,
        token.trim()
    ));
    assert_eq!(keys(&hello), ["id", "result"]);
    assert_eq!(hello["id"], 2);
    assert_eq!(keys(&hello["result"]), ["host_version", "protocol_version"]);
    assert_eq!(hello["result"]["protocol_version"], 1);
    assert!(hello["result"]["host_version"].is_string());

    // `params` may be left out of a request.
    let status = wire.send(r#"{"id":3,"method":"host.status"}"#);
    assert_eq!(keys(&status), ["id", "result"]);
    assert_eq!(
        keys(&status["result"]),
        [
            "agents",
            "agents_blocked",
            "agents_unsandboxed",
            "agents_working",
            "binary_stale",
            "host_version",
            "lockdown",
            "pid",
            "protocol_version",
        ]
    );

    // The agent calls a v1 client makes, on an empty host.
    let list = wire.send(r#"{"id":4,"method":"agent.list","params":{}}"#);
    assert_eq!(keys(&list["result"]), ["agents"]);
    assert!(list["result"]["agents"].is_array());

    // An error on a real method has the same two keys, and an unknown method is one too.
    let missing = wire.send(r#"{"id":5,"method":"agent.get","params":{"name":"nobody"}}"#);
    assert_eq!(keys(&missing), ["error", "id"]);
    assert_eq!(keys(&missing["error"]), ["code", "message"]);
    let unknown = wire.send(r#"{"id":6,"method":"no.such.method","params":{}}"#);
    assert_eq!(keys(&unknown), ["error", "id"]);

    // The spawn reply and the info a client reads from it.
    let spawn = wire.send(
        &json!({
            "id": 7,
            "method": "agent.spawn",
            "params": {"name": "wire", "argv": ["sh", "-c", "sleep 30"], "cwd": std::env::temp_dir()},
        })
        .to_string(),
    );
    assert_eq!(keys(&spawn), ["id", "result"], "{spawn}");
    let info = &spawn["result"];
    for key in ["name", "status", "state", "sandbox"] {
        assert!(info.get(key).is_some(), "agent info lost {key}: {info}");
    }
    assert_eq!(info["name"], "wire");
    assert_eq!(info["status"], "running");
    let removed = wire.send(r#"{"id":8,"method":"agent.remove","params":{"name":"wire"}}"#);
    assert_eq!(keys(&removed), ["id", "result"], "{removed}");

    stop.stop();
    let _ = thread.join();
}
