//! Call memory with Jev (calls spec `2026-10-25-calls-design.md` §8).
//!
//! Jev returns typed judgments, not text, so a call's memory is selection:
//! its own screened messages, verbatim, tagged and ranked by Jev. This module
//! is the pure part, shared by the relay and the memory eval: building the
//! tagging and recall requests, reading their answers, and composing the
//! bundle an agent receives.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use uuid::Uuid;

/// The most a tagging request's `state` may hold, as message safety's.
pub const STATE_BUDGET: usize = 24 * 1024;
/// A message's text as sent to Jev, and the previous message's.
const TEXT_BYTES: usize = 4 * 1024;
const PREVIOUS_BYTES: usize = 1024;
/// An open question as sent to Jev, and how many are sent.
const OPEN_CHARS: usize = 300;
pub const MAX_OPEN: usize = 32;
/// Recall's candidates, and their snippets (spec §8.4).
pub const MAX_CANDIDATES: usize = 64;
pub const SNIPPET_CHARS: usize = 300;
/// The bundle's budgets (spec §8.3).
const RECENT_BYTES: usize = 8 * 1024;
const RECENT_ONLY_BYTES: usize = 16 * 1024;
const PINNED_BYTES: usize = 6 * 1024;
const OPEN_BYTES: usize = 2 * 1024;
const ITEM_TEXT_BYTES: usize = 2 * 1024;
const MIN_RECENT: usize = 3;
const SHORT_CHARS: usize = 200;
const CONTEXT_CHARS: usize = 300;
const PREVIEW_BYTES: usize = 1024;

/// Which side of the call sent a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Caller,
    Callee,
}

/// A message to tag.
#[derive(Debug, Clone)]
pub struct Message {
    pub seq: i32,
    pub from: Side,
    pub text: String,
    /// The message before it, for what a short reply refers to.
    pub previous: Option<String>,
}

/// A question asked earlier in the call and not yet answered.
#[derive(Debug, Clone)]
pub struct Open {
    pub seq: i32,
    pub text: String,
}

/// What Jev said about a message.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Tags {
    pub decision: f64,
    pub commitment: f64,
    pub open_question: f64,
    pub fact: f64,
    /// 0 (chit-chat) to 3 (an agreed term or deadline).
    pub weight: f64,
    /// The earlier question it answers, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolves: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolves_p: Option<f64>,
}

impl Tags {
    /// Worth pinning: a decision, a commitment or a fact.
    pub fn pinned(&self, threshold: f64) -> bool {
        self.decision >= threshold || self.commitment >= threshold || self.fact >= threshold
    }

    pub fn asks(&self, threshold: f64) -> bool {
        self.open_question >= threshold
    }

    /// The earlier question it answers, when Jev is sure enough.
    pub fn answers(&self, threshold: f64) -> Option<i32> {
        self.resolves
            .filter(|_| self.resolves_p.unwrap_or(0.0) >= threshold)
    }
}

fn slice_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    &s[..cut]
}

fn slice_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn noul(instructions: String, yes: &str, no: &str) -> Value {
    json!({
        "type": "noul",
        "instructions": instructions,
        "criteria": { "true": yes, "false": no },
    })
}

// ─── Tagging (spec §8.2) ─────────────────────────────────

fn tag_state(topic: Option<&str>, messages: &[Message], open: &[Open]) -> Value {
    json!({
        "call": { "topic": topic },
        "messages": messages.iter().map(|m| json!({
            "id": format!("m{}", m.seq),
            "from": m.from,
            "text": slice_bytes(&m.text, TEXT_BYTES),
            "previous": m.previous.as_deref().map(|p| slice_bytes(p, PREVIOUS_BYTES)),
        })).collect::<Vec<_>>(),
        "open_questions": open.iter().take(MAX_OPEN).map(|o| json!({
            "id": format!("q{}", o.seq),
            "text": slice_chars(&o.text, OPEN_CHARS),
        })).collect::<Vec<_>>(),
    })
}

/// How many messages from the front of `messages` fit one request's state
/// (at least one, when there are any: a single message is sliced to fit).
pub fn fit_batch(topic: Option<&str>, messages: &[Message], open: &[Open]) -> usize {
    let mut n = messages.len();
    while n > 1 && tag_state(topic, &messages[..n], open).to_string().len() > STATE_BUDGET {
        n -= 1;
    }
    n
}

/// The `state` and `questions` that tag `messages`: per message `m<seq>`,
/// four nouls, a `weight` score and, when there's anything it could answer,
/// a `resolves` choice over the open questions and the earlier messages in
/// the batch.
pub fn tag_request(topic: Option<&str>, messages: &[Message], open: &[Open]) -> (Value, Value) {
    let state = tag_state(topic, messages, open);
    let mut q = Map::new();
    for (i, m) in messages.iter().enumerate() {
        let id = format!("m{}", m.seq);
        let it = format!("`messages[{i}].text`");
        q.insert(
            format!("{id}_decision"),
            noul(
                format!(
                    "Does {it} itself make or confirm a decision between the two parties of \
                     this call? `messages[{i}].previous` is the message before it, there only \
                     to show what a short reply refers to."
                ),
                "It settles something: a choice, a plan, a price, a date. A short 'Agreed.' \
                 or 'Yes, let's do that.' accepting the previous message's proposal counts.",
                "It proposes, asks, informs, thanks or chats without settling anything. A \
                 question is never a decision, and saying it will do a task ('I'll send \
                 it') isn't one unless it also settles a choice.",
            ),
        );
        q.insert(
            format!("{id}_commitment"),
            noul(
                format!("Does {it} have someone commit to doing something?"),
                "Someone says they will do something, with or without a time: 'I'll send \
                 the contract by Friday'.",
                "No one takes on a task in this message. Questions, thanks and \
                 acknowledgements don't count, nor does a commitment made only in \
                 `previous`.",
            ),
        );
        q.insert(
            format!("{id}_open_question"),
            noul(
                format!("Does {it} ask the other party something they should answer?"),
                "It asks for information, a choice or a confirmation.",
                "It asks nothing, or only rhetorically.",
            ),
        );
        q.insert(
            format!("{id}_fact"),
            noul(
                format!("Does {it} state a fact the rest of the call depends on?"),
                "It gives a figure, date, constraint, requirement or detail that later \
                 messages rely on: a budget, a headcount, a deadline.",
                "It's chit-chat, thanks, an opinion, or nothing new.",
            ),
        );
        q.insert(
            format!("{id}_weight"),
            json!({
                "type": "score",
                "instructions": format!("How much will the rest of this call depend on {it}?"),
                "criteria": [
                    "Chit-chat, thanks or an acknowledgement.",
                    "Context: background that helps follow the conversation.",
                    "Something later messages will refer to: a proposal, a requirement or a question.",
                    "An agreed term, decision or deadline the parties will hold each other to.",
                ],
            }),
        );
        let mut options = Map::new();
        for o in open.iter().take(MAX_OPEN).filter(|o| o.seq < m.seq) {
            options.insert(
                format!("q{}", o.seq),
                json!(format!("The question `q{}` in `open_questions`.", o.seq)),
            );
        }
        for earlier in &messages[..i] {
            options.insert(
                format!("m{}", earlier.seq),
                json!(format!(
                    "The question asked in `m{}` in `messages`, if it asks one.",
                    earlier.seq
                )),
            );
        }
        if !options.is_empty() {
            options.insert("none".into(), json!("It answers none of them."));
            q.insert(
                format!("{id}_resolves"),
                json!({
                    "type": "choice",
                    "instructions": format!(
                        "Which earlier question does {it} answer? The questions are in \
                         `open_questions`, or asked in earlier entries of `messages`."
                    ),
                    "criteria": options,
                }),
            );
        }
    }
    (state, Value::Object(q))
}

fn p(answers: &Map<String, Value>, id: &str) -> f64 {
    answers
        .get(id)
        .and_then(|a| a.get("noul"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
}

/// Read a tagging request's answers. A message whose answers are missing
/// gets zeros, which tag nothing.
pub fn read_tags(answers: &Map<String, Value>, messages: &[Message]) -> Vec<(i32, Tags)> {
    messages
        .iter()
        .map(|m| {
            let id = format!("m{}", m.seq);
            let resolves = answers.get(&format!("{id}_resolves"));
            let choice = resolves
                .and_then(|r| r.get("choice"))
                .and_then(Value::as_str)
                .filter(|c| *c != "none");
            let seq = choice.and_then(|c| c.get(1..)).and_then(|n| n.parse().ok());
            let choice_p = choice.and_then(|c| resolves?.get("probabilities")?.get(c)?.as_f64());
            let tags = Tags {
                decision: p(answers, &format!("{id}_decision")),
                commitment: p(answers, &format!("{id}_commitment")),
                open_question: p(answers, &format!("{id}_open_question")),
                fact: p(answers, &format!("{id}_fact")),
                weight: answers
                    .get(&format!("{id}_weight"))
                    .and_then(|a| a.get("score"))
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0),
                resolves: seq,
                resolves_p: seq.and(choice_p),
            };
            (m.seq, tags)
        })
        .collect()
}

// ─── Recall (spec §8.4) ──────────────────────────────────

/// A message recall may choose.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub seq: i32,
    pub from: Side,
    pub snippet: String,
}

/// The `state` and `questions` that ask which candidate answers `question`.
pub fn recall_request(question: &str, candidates: &[Candidate]) -> (Value, Value) {
    let candidates = &candidates[..candidates.len().min(MAX_CANDIDATES)];
    let state = json!({
        "question": question,
        "messages": candidates.iter().map(|c| json!({
            "id": format!("m{}", c.seq),
            "from": c.from,
            "text": slice_chars(&c.snippet, SNIPPET_CHARS),
        })).collect::<Vec<_>>(),
    });
    let mut options = Map::new();
    for c in candidates {
        options.insert(format!("m{}", c.seq), Value::Null);
    }
    options.insert("none".into(), json!("None of them answers it."));
    let questions = json!({
        "best": {
            "type": "choice",
            "instructions": "Which message in `messages` (by `id`) best answers `question`?",
            "criteria": options,
        },
        "answered": noul(
            "Does any message in `messages` answer `question`?".into(),
            "One of them gives the answer, or most of it.",
            "None of them does; at most they mention the topic.",
        ),
    });
    (state, questions)
}

/// Read a recall request's answers: whether anything answers, and up to
/// five candidates by probability (at least 0.05), best first.
pub fn read_recall(answers: &Map<String, Value>) -> (Option<f64>, Vec<(i32, f64)>) {
    let answered = answers
        .get("answered")
        .and_then(|a| a.get("noul"))
        .and_then(Value::as_f64);
    let mut ranked: Vec<(i32, f64)> = answers
        .get("best")
        .and_then(|b| b.get("probabilities"))
        .and_then(Value::as_object)
        .map(|probs| {
            probs
                .iter()
                .filter(|(k, _)| k.as_str() != "none")
                .filter_map(|(k, v)| Some((k.get(1..)?.parse().ok()?, v.as_f64()?)))
                .filter(|(_, p)| *p >= 0.05)
                .collect()
        })
        .unwrap_or_default();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked.truncate(5);
    (answered, ranked)
}

// ─── The bundle (spec §8.3) ──────────────────────────────

/// An invocation made in the call, as the bundle shows it.
#[derive(Debug, Clone)]
pub struct Invocation {
    pub id: Uuid,
    pub capability: String,
    pub status: String,
    pub input: Option<Value>,
    pub output: Option<Value>,
}

/// A transcript entry the bundle may hold: a message (with its text) or an
/// invocation.
#[derive(Debug, Clone)]
pub struct BundleEntry {
    pub seq: i32,
    pub from: Option<Uuid>,
    pub at: DateTime<Utc>,
    pub text: Option<String>,
    pub tags: Option<Tags>,
    pub invocation: Option<Invocation>,
}

fn finished(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "rejected" | "timeout")
}

/// A preview if its JSON is small enough, else a note of its size.
fn preview(v: &Value) -> Value {
    let n = v.to_string().len();
    if n <= PREVIEW_BYTES {
        v.clone()
    } else {
        json!({ "truncated": true, "bytes": n })
    }
}

fn round(p: f64) -> f64 {
    (p * 100.0).round() / 100.0
}

fn item(e: &BundleEntry, previous: Option<&str>, with_tags: bool) -> Value {
    let mut v = json!({ "seq": e.seq, "from": e.from, "at": e.at });
    if let Some(inv) = &e.invocation {
        let mut i = json!({ "id": inv.id, "capability": inv.capability, "status": inv.status });
        if finished(&inv.status) {
            if let Some(input) = &inv.input {
                i["input"] = preview(input);
            }
            if let Some(output) = &inv.output {
                i["output"] = preview(output);
            }
        }
        v["invocation"] = i;
        return v;
    }
    let text = e.text.as_deref().unwrap_or_default();
    v["text"] = json!(slice_bytes(text, ITEM_TEXT_BYTES));
    if with_tags {
        if let Some(t) = &e.tags {
            v["tags"] = json!({
                "decision": round(t.decision),
                "commitment": round(t.commitment),
                "open_question": round(t.open_question),
                "fact": round(t.fact),
                "weight": round(t.weight),
            });
        }
        if text.chars().count() < SHORT_CHARS {
            if let Some(prev) = previous {
                v["context"] = json!(slice_chars(prev, CONTEXT_CHARS));
            }
        }
    }
    v
}

fn size(v: &Value) -> usize {
    v.to_string().len()
}

/// Compose a call's memory from its entries in `seq` order: `recent` (the
/// latest, verbatim), `pinned` (decisions, commitments and facts from
/// earlier) and `open` (questions not yet answered). Without tags, it's
/// `recent` alone, with a larger budget.
pub fn bundle(entries: &[BundleEntry], threshold: f64, tags_on: bool) -> Value {
    let previous = |i: usize| entries[..i].iter().rev().find_map(|e| e.text.as_deref());

    // Recent: from the end, within the budget, at least the last three.
    let budget = if tags_on {
        RECENT_BYTES
    } else {
        RECENT_ONLY_BYTES
    };
    let mut recent: Vec<(usize, Value)> = Vec::new();
    let mut used = 0;
    for (i, e) in entries.iter().enumerate().rev() {
        let v = item(e, None, false);
        let n = size(&v);
        if used + n > budget && recent.len() >= MIN_RECENT {
            break;
        }
        used += n;
        recent.push((i, v));
    }
    recent.reverse();
    let in_recent = |i: usize| recent.iter().any(|(j, _)| *j == i);
    if !tags_on {
        return json!({
            "pinned": [],
            "open": [],
            "recent": recent.into_iter().map(|(_, v)| v).collect::<Vec<_>>(),
        });
    }

    let answered: Vec<i32> = entries
        .iter()
        .filter_map(|e| e.tags.as_ref()?.answers(threshold))
        .collect();

    // Pinned: by weight, then newest first.
    let mut pinnable: Vec<usize> = (0..entries.len())
        .filter(|&i| !in_recent(i))
        .filter(|&i| {
            entries[i]
                .tags
                .as_ref()
                .is_some_and(|t| t.pinned(threshold))
        })
        .collect();
    pinnable.sort_by(|&a, &b| {
        let w = |i: usize| entries[i].tags.as_ref().map_or(0.0, |t| t.weight);
        w(b).total_cmp(&w(a))
            .then(entries[b].seq.cmp(&entries[a].seq))
    });
    let mut pinned = Vec::new();
    let mut used = 0;
    for i in pinnable {
        let v = item(&entries[i], previous(i), true);
        let n = size(&v);
        if used + n > PINNED_BYTES {
            continue;
        }
        used += n;
        pinned.push(v);
    }

    // Open: asked, not answered, newest first.
    let mut open = Vec::new();
    let mut used = 0;
    for (i, e) in entries.iter().enumerate().rev() {
        let asks = e.tags.as_ref().is_some_and(|t| t.asks(threshold));
        if !asks || in_recent(i) || answered.contains(&e.seq) {
            continue;
        }
        let v = item(e, previous(i), true);
        let n = size(&v);
        if used + n > OPEN_BYTES {
            continue;
        }
        used += n;
        open.push(v);
    }

    json!({
        "pinned": pinned,
        "open": open,
        "recent": recent.into_iter().map(|(_, v)| v).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(seq: i32, text: &str) -> Message {
        Message {
            seq,
            from: if seq % 2 == 1 {
                Side::Caller
            } else {
                Side::Callee
            },
            text: text.into(),
            previous: None,
        }
    }

    #[test]
    fn tagging_asks_four_nouls_a_weight_and_what_it_resolves() {
        let messages = [msg(4, "What's the budget?"), msg(5, "About 4,000.")];
        let open = [Open {
            seq: 2,
            text: "Which venue?".into(),
        }];
        let (state, q) = tag_request(Some("Venue"), &messages, &open);
        assert_eq!(state["messages"][1]["id"], "m5");
        assert_eq!(state["open_questions"][0]["id"], "q2");
        for suffix in ["decision", "commitment", "open_question", "fact"] {
            assert_eq!(q[format!("m4_{suffix}")]["type"], "noul");
        }
        assert_eq!(q["m4_weight"]["type"], "score");
        assert_eq!(q["m4_weight"]["criteria"].as_array().unwrap().len(), 4);
        // m4 can answer only q2; m5 can answer q2 or what m4 asked.
        let m4: Vec<&String> = q["m4_resolves"]["criteria"]
            .as_object()
            .unwrap()
            .keys()
            .collect();
        assert_eq!(m4, ["none", "q2"]);
        let m5: Vec<&String> = q["m5_resolves"]["criteria"]
            .as_object()
            .unwrap()
            .keys()
            .collect();
        assert_eq!(m5, ["m4", "none", "q2"]);
        // Nothing to answer: no choice.
        let (_, q) = tag_request(None, &[msg(1, "Hi")], &[]);
        assert!(q.get("m1_resolves").is_none());
    }

    #[test]
    fn batches_fit_the_state_budget() {
        let long = "x".repeat(4000);
        let many: Vec<Message> = (1..=20).map(|s| msg(s, &long)).collect();
        let n = fit_batch(None, &many, &[]);
        assert!((1..20).contains(&n), "{n}");
        let (state, _) = tag_request(None, &many[..n], &[]);
        assert!(state.to_string().len() <= STATE_BUDGET);
        // One huge message still goes, sliced.
        let huge = [msg(1, &"y".repeat(100_000))];
        assert_eq!(fit_batch(None, &huge, &[]), 1);
        let (state, _) = tag_request(None, &huge, &[]);
        assert!(state.to_string().len() < STATE_BUDGET);
    }

    #[test]
    fn tags_are_read_from_the_answers() {
        let answers = json!({
            "m5_decision": { "type": "noul", "noul": 0.9 },
            "m5_fact": { "type": "noul", "noul": 0.4 },
            "m5_weight": { "type": "score", "score": 2.6 },
            "m5_resolves": { "type": "choice", "choice": "q2", "probabilities": { "q2": 0.8, "none": 0.2 } },
        });
        let tags = read_tags(answers.as_object().unwrap(), &[msg(5, "Hall A, agreed.")]);
        assert_eq!(tags[0].0, 5);
        let t = &tags[0].1;
        assert_eq!(
            (t.decision, t.commitment, t.fact, t.weight),
            (0.9, 0.0, 0.4, 2.6)
        );
        assert_eq!((t.resolves, t.resolves_p), (Some(2), Some(0.8)));
        assert!(t.pinned(0.7));
        assert_eq!(t.answers(0.7), Some(2));
        assert_eq!(t.answers(0.9), None);
    }

    #[test]
    fn recall_asks_one_choice_and_reads_the_best_five() {
        let candidates: Vec<Candidate> = (1..=8)
            .map(|s| Candidate {
                seq: s,
                from: Side::Caller,
                snippet: format!("message {s}"),
            })
            .collect();
        let (state, q) = recall_request("Which venue?", &candidates);
        assert_eq!(state["messages"].as_array().unwrap().len(), 8);
        assert_eq!(q["best"]["criteria"].as_object().unwrap().len(), 9);
        let answers = json!({
            "answered": { "noul": 0.93 },
            "best": { "choice": "m3", "probabilities": {
                "m3": 0.5, "m7": 0.2, "m1": 0.1, "m2": 0.06, "m4": 0.05, "m5": 0.04, "none": 0.05
            } },
        });
        let (answered, ranked) = read_recall(answers.as_object().unwrap());
        assert_eq!(answered, Some(0.93));
        assert_eq!(
            ranked.iter().map(|r| r.0).collect::<Vec<_>>(),
            [3, 7, 1, 2, 4]
        );
    }

    fn entry(seq: i32, text: &str, tags: Option<Tags>) -> BundleEntry {
        BundleEntry {
            seq,
            from: None,
            at: Utc::now(),
            text: Some(text.into()),
            tags,
            invocation: None,
        }
    }

    fn tagged(decision: f64, open_question: f64, weight: f64) -> Option<Tags> {
        Some(Tags {
            decision,
            open_question,
            weight,
            ..Tags::default()
        })
    }

    fn seqs(v: &Value) -> Vec<i64> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|i| i["seq"].as_i64().unwrap())
            .collect()
    }

    #[test]
    fn the_bundle_pins_decisions_keeps_open_questions_and_the_latest() {
        let filler = "x".repeat(1500);
        let mut entries = vec![
            entry(1, "Which venue should we book?", tagged(0.0, 0.9, 2.0)),
            entry(2, "Can you also confirm the date?", tagged(0.0, 0.9, 2.0)),
            entry(3, "Hall A, agreed.", tagged(0.95, 0.0, 3.0)),
            entry(4, "Agreed.", tagged(0.9, 0.0, 2.0)),
        ];
        // Hall A answers the venue question.
        entries[2].tags.as_mut().unwrap().resolves = Some(1);
        entries[2].tags.as_mut().unwrap().resolves_p = Some(0.9);
        for s in 5..=14 {
            entries.push(entry(s, &filler, tagged(0.0, 0.0, 0.0)));
        }
        let b = bundle(&entries, 0.7, true);
        // The latest fill recent; at least three.
        let recent = seqs(&b["recent"]);
        assert!(
            recent.len() >= 3 && *recent.last().unwrap() == 14,
            "{recent:?}"
        );
        // Decisions are pinned, heaviest first; a short one carries context.
        assert_eq!(seqs(&b["pinned"]), [3, 4]);
        assert_eq!(b["pinned"][1]["context"], "Hall A, agreed.");
        // The venue question was answered; the date question wasn't.
        assert_eq!(seqs(&b["open"]), [2]);
    }

    #[test]
    fn without_tags_the_bundle_is_recent_only_with_a_larger_budget() {
        let filler = "x".repeat(1500);
        let entries: Vec<BundleEntry> = (1..=20).map(|s| entry(s, &filler, None)).collect();
        let b = bundle(&entries, 0.7, false);
        assert!(b["pinned"].as_array().unwrap().is_empty());
        let n = b["recent"].as_array().unwrap().len();
        assert!(n > 5 && n < 20, "{n}");
        assert!(b["recent"].to_string().len() <= RECENT_ONLY_BYTES + 2048);
        let with_tags = bundle(&entries, 0.7, true);
        assert!(with_tags["recent"].as_array().unwrap().len() < n);
    }

    #[test]
    fn finished_invocations_carry_small_previews() {
        let mut e = entry(3, "", None);
        e.text = None;
        e.invocation = Some(Invocation {
            id: Uuid::nil(),
            capability: "book_venue".into(),
            status: "succeeded".into(),
            input: Some(json!({ "venue": "Hall A" })),
            output: Some(json!({ "notes": "y".repeat(5000) })),
        });
        let b = bundle(&[e], 0.7, true);
        let inv = &b["recent"][0]["invocation"];
        assert_eq!(inv["capability"], "book_venue");
        assert_eq!(inv["input"]["venue"], "Hall A");
        assert_eq!(inv["output"]["truncated"], true);
    }
}
