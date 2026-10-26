//! Run the labelled call transcripts against TypeSafe's Jev and print how
//! call memory does (calls spec `2026-10-25-calls-design.md` §8, §16).
//!
//! ```text
//! TYPESAFE_AI_KEY=… cargo run -p chakramcp-shared --example calls_memory_eval
//! TYPESAFE_AI_KEY=… cargo run -p chakramcp-shared --example calls_memory_eval -- --all
//! ```
//!
//! Each call is tagged in batches, as the relay's tagger does, and each
//! recall question is asked over the whole call. Tags are judged at several
//! thresholds from the same answers, so `CALLS_TAG_THRESHOLD` can be tuned
//! from one run. `--all` prints every message's labels and answers.
//!
//! Reads `TYPESAFE_AI_KEY` (and `TYPESAFE_AI_MODEL`, `TYPESAFE_AI_BASE_URL`)
//! from the environment and never prints them. The labelled set is built in,
//! so the binary runs anywhere the key is (e.g. inside the relay container).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use chakramcp_shared::calls_memory::{
    fit_batch, read_recall, read_tags, recall_request, tag_request, Candidate, Message, Open, Side,
    Tags,
};
use chakramcp_shared::system_one::{ComplianceChecker, DEFAULT_BASE_URL, DEFAULT_MODEL};
use serde_json::Value;

const TAGS: [&str; 4] = ["decision", "commitment", "open_question", "fact"];
const THRESHOLDS: [f64; 5] = [0.5, 0.6, 0.7, 0.8, 0.9];
const BATCH: usize = 16;
/// The memory price, as `CALLS_MEMORY_MC_PER_1K_TOKENS` defaults it.
const MC_PER_1K: i64 = 42;

struct Tagged {
    call: String,
    lang: String,
    seq: i32,
    labels: Vec<String>,
    resolves: Option<i32>,
    tags: Tags,
}

struct Recalled {
    call: String,
    question: String,
    expect: Vec<i32>,
    answered: Option<f64>,
    ranked: Vec<(i32, f64)>,
}

fn prob(t: &Tags, tag: &str) -> f64 {
    match tag {
        "decision" => t.decision,
        "commitment" => t.commitment,
        "open_question" => t.open_question,
        _ => t.fact,
    }
}

fn side(v: &Value) -> Side {
    if v.as_str() == Some("caller") {
        Side::Caller
    } else {
        Side::Callee
    }
}

#[tokio::main]
async fn main() {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let Some(key) = var("TYPESAFE_AI_KEY") else {
        eprintln!("TYPESAFE_AI_KEY is not set");
        std::process::exit(2);
    };
    let model = var("TYPESAFE_AI_MODEL").unwrap_or_else(|| DEFAULT_MODEL.into());
    let base = var("TYPESAFE_AI_BASE_URL").unwrap_or_else(|| DEFAULT_BASE_URL.into());
    let all = std::env::args().any(|a| a == "--all");
    let client = ComplianceChecker::new(key, model.clone(), &base, Duration::from_secs(30));
    let calls: Vec<Value> =
        serde_json::from_str(include_str!("../tests/data/calls/transcripts.json"))
            .expect("transcripts.json parses");
    println!("{} calls against {model}\n", calls.len());

    let mut tagged = Vec::new();
    let mut recalled = Vec::new();
    let mut tag_times = Vec::new();
    let mut recall_times = Vec::new();
    let mut tokens = 0i64;
    let mut errors = 0;

    for call in &calls {
        let id = call["id"].as_str().unwrap_or("?").to_owned();
        let lang = call["lang"].as_str().unwrap_or("?").to_owned();
        let topic = call["topic"].as_str();
        let raw = call["messages"].as_array().cloned().unwrap_or_default();
        let messages: Vec<Message> = raw
            .iter()
            .enumerate()
            .map(|(i, m)| Message {
                seq: m["seq"].as_i64().unwrap_or(0) as i32,
                from: side(&m["from"]),
                text: m["text"].as_str().unwrap_or_default().to_owned(),
                previous: i
                    .checked_sub(1)
                    .and_then(|p| raw[p]["text"].as_str())
                    .map(str::to_owned),
            })
            .collect();

        // Tag in batches, carrying open questions forward as the tagger does.
        let mut results: Vec<(i32, Tags)> = Vec::new();
        let mut rest = &messages[..];
        while !rest.is_empty() {
            let open: Vec<Open> = results
                .iter()
                .filter(|(_, t)| t.asks(0.7))
                .filter(|(s, _)| !results.iter().any(|(_, a)| a.answers(0.7) == Some(*s)))
                .map(|(s, _)| Open {
                    seq: *s,
                    text: messages[(*s - 1) as usize].text.clone(),
                })
                .collect();
            let n = fit_batch(topic, &rest[..rest.len().min(BATCH)], &open);
            let (state, questions) = tag_request(topic, &rest[..n], &open);
            let started = Instant::now();
            match client.ask_questions(&state, &questions).await {
                Ok(asked) => {
                    tag_times.push(started.elapsed());
                    tokens += asked.input_tokens;
                    results.extend(read_tags(&asked.answers, &rest[..n]));
                }
                Err(e) => {
                    errors += 1;
                    eprintln!("{id}: tagging failed: {e}");
                    results.extend(rest[..n].iter().map(|m| (m.seq, Tags::default())));
                }
            }
            rest = &rest[n..];
        }
        for (seq, tags) in results {
            let m = &raw[(seq - 1) as usize];
            tagged.push(Tagged {
                call: id.clone(),
                lang: lang.clone(),
                seq,
                labels: m["tags"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|t| t.as_str().map(str::to_owned))
                    .collect(),
                resolves: m["resolves"].as_i64().map(|r| r as i32),
                tags,
            });
        }

        // Recall over the whole call.
        let candidates: Vec<Candidate> = messages
            .iter()
            .map(|m| Candidate {
                seq: m.seq,
                from: m.from,
                snippet: m.text.clone(),
            })
            .collect();
        for r in call["recall"].as_array().into_iter().flatten() {
            let question = r["question"].as_str().unwrap_or_default();
            let (state, questions) = recall_request(question, &candidates);
            let started = Instant::now();
            let (answered, ranked) = match client.ask_questions(&state, &questions).await {
                Ok(asked) => {
                    recall_times.push(started.elapsed());
                    tokens += asked.input_tokens;
                    read_recall(&asked.answers)
                }
                Err(e) => {
                    errors += 1;
                    eprintln!("{id}: recall failed: {e}");
                    (None, Vec::new())
                }
            };
            recalled.push(Recalled {
                call: id.clone(),
                question: question.to_owned(),
                expect: r["expect"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|s| s.as_i64().map(|s| s as i32))
                    .collect(),
                answered,
                ranked,
            });
        }
    }

    if all {
        print_all(&tagged, &recalled);
    }
    report_tags(&tagged);
    report_recall(&recalled);

    let latency = |times: &mut Vec<Duration>| {
        times.sort();
        if times.is_empty() {
            return "n/a".to_owned();
        }
        let at = |q: f64| times[((times.len() - 1) as f64 * q).round() as usize];
        format!("median {:?}, p95 {:?}", at(0.5), at(0.95))
    };
    println!("tagging latency: {}", latency(&mut tag_times));
    println!("recall latency:  {}", latency(&mut recall_times));
    println!(
        "input tokens: {tokens} (about {} credits at {MC_PER_1K} mc per 1,000)",
        (tokens * MC_PER_1K) as f64 / 1_000_000.0
    );
    println!("failed requests: {errors}");
}

fn f(n: f64) -> String {
    format!("{n:.2}")
}

fn report_tags(tagged: &[Tagged]) {
    println!("tags: precision / recall (F1) by threshold");
    print!("{:<15}", "tag");
    for t in THRESHOLDS {
        print!("{:>20}", format!("@{t}"));
    }
    println!();
    for tag in TAGS {
        print!("{tag:<15}");
        for t in THRESHOLDS {
            let (mut tp, mut fp, mut fn_) = (0.0, 0.0, 0.0);
            for m in tagged {
                let said = prob(&m.tags, tag) >= t;
                let is = m.labels.iter().any(|l| l == tag);
                match (said, is) {
                    (true, true) => tp += 1.0,
                    (true, false) => fp += 1.0,
                    (false, true) => fn_ += 1.0,
                    _ => {}
                }
            }
            let p = if tp + fp > 0.0 { tp / (tp + fp) } else { 0.0 };
            let r = if tp + fn_ > 0.0 { tp / (tp + fn_) } else { 0.0 };
            let f1 = if p + r > 0.0 {
                2.0 * p * r / (p + r)
            } else {
                0.0
            };
            print!("{:>20}", format!("{} / {} ({})", f(p), f(r), f(f1)));
        }
        println!();
    }
    // What each message answers.
    for t in THRESHOLDS {
        let labelled: Vec<&Tagged> = tagged.iter().filter(|m| m.resolves.is_some()).collect();
        let right = labelled
            .iter()
            .filter(|m| m.tags.answers(t) == m.resolves)
            .count();
        let spurious = tagged
            .iter()
            .filter(|m| m.resolves.is_none() && m.tags.answers(t).is_some())
            .count();
        println!(
            "resolves @{t}: {right} of {} right, {spurious} spurious",
            labelled.len()
        );
    }
    // Weight: what's pinned should weigh more than what isn't.
    let mean = |ms: Vec<&Tagged>| {
        let n = ms.len().max(1) as f64;
        ms.iter().map(|m| m.tags.weight).sum::<f64>() / n
    };
    let decided = mean(
        tagged
            .iter()
            .filter(|m| {
                m.labels
                    .iter()
                    .any(|l| l == "decision" || l == "commitment")
            })
            .collect(),
    );
    let plain = mean(tagged.iter().filter(|m| m.labels.is_empty()).collect());
    println!(
        "weight: decisions and commitments {}, untagged {}",
        f(decided),
        f(plain)
    );
    // F1 at 0.7 by language.
    let mut by_lang: BTreeMap<&str, (f64, f64, f64)> = BTreeMap::new();
    for m in tagged {
        let e = by_lang.entry(&m.lang).or_default();
        for tag in TAGS {
            let said = prob(&m.tags, tag) >= 0.7;
            let is = m.labels.iter().any(|l| l == tag);
            match (said, is) {
                (true, true) => e.0 += 1.0,
                (true, false) => e.1 += 1.0,
                (false, true) => e.2 += 1.0,
                _ => {}
            }
        }
    }
    println!("all tags @0.7 by language (precision / recall):");
    for (lang, (tp, fp, fn_)) in by_lang {
        let p = if tp + fp > 0.0 { tp / (tp + fp) } else { 0.0 };
        let r = if tp + fn_ > 0.0 { tp / (tp + fn_) } else { 0.0 };
        println!("  {lang:<8} {} / {}", f(p), f(r));
    }
    println!();
}

fn report_recall(recalled: &[Recalled]) {
    let answerable: Vec<&Recalled> = recalled.iter().filter(|r| !r.expect.is_empty()).collect();
    let none: Vec<&Recalled> = recalled.iter().filter(|r| r.expect.is_empty()).collect();
    let top = |r: &Recalled, k: usize| r.ranked.iter().take(k).any(|(s, _)| r.expect.contains(s));
    let count = |k| answerable.iter().filter(|r| top(r, k)).count();
    println!(
        "recall: top-1 {} of {}, top-3 {} of {}",
        count(1),
        answerable.len(),
        count(3),
        answerable.len()
    );
    let mean = |rs: &[&Recalled]| {
        let n = rs.len().max(1) as f64;
        rs.iter().map(|r| r.answered.unwrap_or(0.0)).sum::<f64>() / n
    };
    println!(
        "answered: mean {} when answerable, {} when not ({} unanswerable)",
        f(mean(&answerable)),
        f(mean(&none)),
        none.len()
    );
    let misses: Vec<&&Recalled> = answerable.iter().filter(|r| !top(r, 3)).collect();
    if !misses.is_empty() {
        println!("recall misses (top 3):");
        for r in misses {
            println!(
                "  {:<28} {:<50} expected {:?} got {:?}",
                r.call,
                r.question.chars().take(50).collect::<String>(),
                r.expect,
                r.ranked.iter().take(3).map(|(s, _)| *s).collect::<Vec<_>>()
            );
        }
    }
    println!();
}

fn print_all(tagged: &[Tagged], recalled: &[Recalled]) {
    println!("every message (label → answers)");
    for m in tagged {
        let said: Vec<String> = TAGS
            .iter()
            .map(|t| (t, prob(&m.tags, t)))
            .filter(|(_, p)| *p >= 0.3)
            .map(|(t, p)| format!("{t}={p:.2}"))
            .collect();
        let miss = TAGS
            .iter()
            .any(|t| (prob(&m.tags, t) >= 0.7) != m.labels.iter().any(|l| l == t));
        println!(
            "{} {:<26}#{:<3} {:<36} → {} w={:.1} r={:?}",
            if miss { "✗" } else { " " },
            m.call,
            m.seq,
            m.labels.join(","),
            said.join(" "),
            m.tags.weight,
            m.tags.resolves
        );
    }
    println!("\nevery recall question");
    for r in recalled {
        println!(
            "  {:<26} {:<50} expected {:?} answered {:?} ranked {:?}",
            r.call,
            r.question.chars().take(50).collect::<String>(),
            r.expect,
            r.answered.map(|a| (a * 100.0).round() / 100.0),
            r.ranked
        );
    }
    println!();
}
