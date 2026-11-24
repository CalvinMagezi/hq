//! `cargo run -p hq-finance --example redact -- <raw.json> <out.json>`: raw text stays in the files.
//! The salt is random per run and never stored, so scrambled numbers cannot be brute-forced back.
use std::collections::{HashMap, hash_map::RandomState};
use std::hash::{BuildHasher, Hasher};
use std::{env, fs};

const PER_SHAPE_CAP: usize = 3;

fn main() {
    let args: Vec<String> = env::args().collect();
    let raw: Vec<serde_json::Value> =
        serde_json::from_str(&fs::read_to_string(&args[1]).unwrap()).unwrap();
    let salt = RandomState::new().build_hasher().finish().to_string();
    let digits = regex::Regex::new(r"\d").unwrap();
    let mut per_shape: HashMap<String, usize> = HashMap::new();
    let (mut out, mut dropped) = (Vec::new(), 0);
    for m in &raw {
        let Some(body) = hq_finance::redact::redact(m["body"].as_str().unwrap_or(""), &salt) else {
            dropped += 1;
            continue;
        };
        let shape = format!("{}|{}", m["sender"], digits.replace_all(&body, "#"));
        let seen = per_shape.entry(shape).or_insert(0);
        *seen += 1;
        if *seen <= PER_SHAPE_CAP {
            out.push(serde_json::json!({"sender": m["sender"], "body": body}));
        }
    }
    fs::write(&args[2], serde_json::to_string_pretty(&out).unwrap()).unwrap();
    eprintln!(
        "{} kept of {} ({} dropped as one-time codes)",
        out.len(),
        raw.len(),
        dropped
    );
}
