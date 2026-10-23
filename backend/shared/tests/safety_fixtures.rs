//! The labelled message set (`tests/data/safety/messages.json`): every
//! entry's `code` expectations — the contact candidates, the unreadable
//! spans and the language class — hold for the code pass. The `expect`
//! field is for the live eval against Jev (S2), not checked here.

use std::time::Instant;

use chakramcp_shared::safety::{self, normalize::normalize, plan, Channel, Context};
use serde_json::Value;

fn entries() -> Vec<Value> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/safety/messages.json"
    );
    let text = std::fs::read_to_string(path).expect("messages.json");
    serde_json::from_str(&text).expect("messages.json parses")
}

fn channel(e: &Value) -> Channel {
    match e["channel"].as_str() {
        Some("reply") => Channel::Reply,
        Some("note") => Channel::Note,
        _ => Channel::Request,
    }
}

#[test]
fn the_code_pass_matches_every_labelled_message() {
    let mut failures = Vec::new();
    let all = entries();
    assert!(all.len() >= 100, "only {} messages", all.len());
    for e in &all {
        let id = e["id"].as_str().unwrap_or("?");
        let schema = e["capability"].get("input_schema");
        let (root, schema) = match channel(e) {
            Channel::Request => ("input", schema),
            _ => ("", None),
        };
        let scan = match safety::scan(&e["value"], root, schema, "") {
            Ok(s) => s,
            Err(_) => {
                failures.push(format!("{id}: too large"));
                continue;
            }
        };

        let mut got: Vec<(String, String)> = scan
            .candidates
            .iter()
            .map(|c| (c.kind.as_str().to_owned(), c.text.clone()))
            .collect();
        let expected = e["code"]["candidates"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for x in &expected {
            let kind = x["kind"].as_str().unwrap_or_default();
            let text = normalize(x["text"].as_str().unwrap_or_default()).text;
            let hit = got.iter().position(|(k, t)| {
                k == kind && (t == &text || t.contains(&text) || text.contains(t.as_str()))
            });
            match hit {
                Some(i) => {
                    got.remove(i);
                }
                None => failures.push(format!("{id}: missed {kind} {text:?}")),
            }
        }
        for (k, t) in got {
            failures.push(format!("{id}: unexpected {k} {t:?}"));
        }

        let opaque = e["code"]["opaque"].as_u64().unwrap_or(0) as usize;
        if scan.opaque.len() != opaque {
            failures.push(format!(
                "{id}: opaque {} (expected {opaque})",
                scan.opaque.len()
            ));
        }
        if let Some(english) = e["code"]["english"].as_bool() {
            if scan.language.english != english {
                failures.push(format!(
                    "{id}: english {} (expected {english}), detected {}",
                    scan.language.english, scan.language.code
                ));
            }
        }

        // Whatever the message, every Jev request fits.
        let p = plan(channel(e), &scan, &Context::default());
        for c in &p.chunks {
            let n = serde_json::to_string(&c.state).unwrap().len();
            if n > safety::STATE_BUDGET_BYTES {
                failures.push(format!("{id}: a chunk is {n} bytes"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} messages:\n{}",
        failures.len(),
        all.len(),
        failures.join("\n")
    );
}

/// `cargo test -p chakramcp-shared --test safety_fixtures -- --ignored --nocapture`
#[test]
#[ignore = "timing, for the PR description"]
fn timing() {
    let typical = serde_json::json!({
        "message": "Hi! Could we move Friday's sync to 4pm? Send the deck to sales@example.com \
                    or ping +91 98450 12345. Notes: ".repeat(12),
        "attendees": ["a@example.com", "b@example.org"],
    });
    let big = serde_json::json!({ "pad": "lorem ipsum dolor sit amet ".repeat(38_000) });
    for (name, v) in [("typical ~2 KiB", &typical), ("~1 MiB", &big)] {
        let started = Instant::now();
        let runs = if name.starts_with("typical") { 200 } else { 3 };
        for _ in 0..runs {
            let s = safety::scan(v, "input", None, "").unwrap();
            let _ = plan(Channel::Request, &s, &Context::default());
        }
        println!("{name}: {:?} per message", started.elapsed() / runs);
    }
}
