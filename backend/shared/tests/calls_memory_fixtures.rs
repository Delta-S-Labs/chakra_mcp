//! The labelled call transcripts (`tests/data/calls/transcripts.json`) are
//! well formed, and each call's tagging and recall requests fit Jev's
//! budget. The labels themselves are judged by the live memory eval
//! (`cargo run -p chakramcp-shared --example calls_memory_eval`).

use std::collections::HashSet;

use chakramcp_shared::calls_memory::{
    fit_batch, recall_request, tag_request, Candidate, Message, Side, STATE_BUDGET,
};
use serde_json::Value;

const TAGS: [&str; 4] = ["decision", "commitment", "open_question", "fact"];

fn transcripts() -> Vec<Value> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/calls/transcripts.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("transcripts.json"))
        .expect("transcripts.json parses")
}

fn side(v: &Value) -> Side {
    match v.as_str() {
        Some("caller") => Side::Caller,
        Some("callee") => Side::Callee,
        other => panic!("from must be caller or callee, got {other:?}"),
    }
}

#[test]
fn the_labelled_calls_are_well_formed_and_fit_the_budget() {
    let calls = transcripts();
    assert!(calls.len() >= 12, "{} calls", calls.len());
    let mut ids = HashSet::new();
    let mut languages = HashSet::new();
    for call in &calls {
        let id = call["id"].as_str().expect("id");
        assert!(ids.insert(id), "{id}: duplicate id");
        languages.insert(call["lang"].as_str().expect("lang"));
        let messages = call["messages"].as_array().expect("messages");
        let mut parsed = Vec::new();
        for (i, m) in messages.iter().enumerate() {
            let seq = m["seq"].as_i64().unwrap() as i32;
            assert_eq!(seq, i as i32 + 1, "{id}: seqs run from 1");
            for t in m["tags"].as_array().unwrap() {
                assert!(TAGS.contains(&t.as_str().unwrap()), "{id}#{seq}: tag {t}");
            }
            if let Some(r) = m["resolves"].as_i64() {
                assert!(r >= 1 && r < seq as i64, "{id}#{seq}: resolves {r}");
            }
            parsed.push(Message {
                seq,
                from: side(&m["from"]),
                text: m["text"].as_str().unwrap().to_owned(),
                previous: i
                    .checked_sub(1)
                    .map(|p| messages[p]["text"].as_str().unwrap().to_owned()),
            });
        }
        let recall = call["recall"].as_array().unwrap();
        assert_eq!(recall.len(), 3, "{id}: three recall questions");
        for r in recall {
            for s in r["expect"].as_array().unwrap() {
                let s = s.as_i64().unwrap() as usize;
                assert!(s >= 1 && s <= messages.len(), "{id}: recall expects #{s}");
            }
        }

        // Tagging goes in batches that fit; recall over the whole call fits.
        let topic = call["topic"].as_str();
        let mut rest = &parsed[..];
        while !rest.is_empty() {
            let n = fit_batch(topic, &rest[..rest.len().min(16)], &[]);
            let (state, questions) = tag_request(topic, &rest[..n], &[]);
            assert!(state.to_string().len() <= STATE_BUDGET, "{id}");
            assert!(questions.as_object().unwrap().len() >= 5 * n);
            rest = &rest[n..];
        }
        let candidates: Vec<Candidate> = parsed
            .iter()
            .map(|m| Candidate {
                seq: m.seq,
                from: m.from,
                snippet: m.text.clone(),
            })
            .collect();
        let (state, _) = recall_request(recall[0]["question"].as_str().unwrap(), &candidates);
        assert!(
            state.to_string().len() <= STATE_BUDGET,
            "{id}: recall state"
        );
    }
    for lang in ["en", "hi", "bn", "hi-Latn", "es"] {
        assert!(languages.contains(lang), "no {lang} call");
    }
}
