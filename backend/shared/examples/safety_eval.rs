//! Run the labelled message set against TypeSafe's Jev and print how
//! message safety does (spec `2026-10-23-message-safety-design.md` §8).
//!
//! ```text
//! TYPESAFE_AI_KEY=… cargo run -p chakramcp-shared --example safety_eval
//! TYPESAFE_AI_KEY=… cargo run -p chakramcp-shared --example safety_eval -- --only offp-
//! TYPESAFE_AI_KEY=… cargo run -p chakramcp-shared --example safety_eval -- --all --today
//! ```
//!
//! `--all` prints every message's outcome and answers, not only the misses.
//! `--today` also runs each request through the compliance check alone (the
//! path with `SYSTEM_ONE_SAFETY` off), to compare the two.
//!
//! Reads `TYPESAFE_AI_KEY` (and `TYPESAFE_AI_MODEL`, `TYPESAFE_AI_BASE_URL`
//! when set) from the environment and never prints them. The labelled set
//! is built into the binary. Each message's outcome is `block` when screening blocked it, `redact` when it removed
//! anything, `flag` when it only flagged, and `deliver` otherwise; `error`
//! means TypeSafe failed for it (fail-open).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use chakramcp_shared::safety::{Action, Channel};
use chakramcp_shared::system_one::{
    ComplianceChecker, Subject, Verdict, DEFAULT_BASE_URL, DEFAULT_MODEL,
};
use serde_json::Value;

struct Outcome {
    id: String,
    group: String,
    lang: String,
    expect: String,
    got: String,
    answers: Vec<(String, f64)>,
    elapsed: Duration,
    /// With `--today`, for a request: the compliance check alone.
    today: Option<(&'static str, Vec<(String, f64)>)>,
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let only = args
        .iter()
        .position(|a| a == "--only")
        .and_then(|i| args.get(i + 1).cloned());
    let all = args.iter().any(|a| a == "--all");
    let today = args.iter().any(|a| a == "--today");

    let checker = ComplianceChecker::new(key, model.clone(), &base, Duration::from_secs(15))
        .with_safety(true);
    // Built in, so the binary runs anywhere the key is (e.g. beside the relay).
    let entries: Vec<Value> =
        serde_json::from_str(include_str!("../tests/data/safety/messages.json"))
            .expect("messages.json parses");
    let entries: Vec<Value> = entries
        .into_iter()
        .filter(|e| {
            only.as_deref()
                .is_none_or(|p| e["id"].as_str().unwrap_or_default().starts_with(p))
        })
        .collect();
    println!("{} messages against {model}\n", entries.len());

    let mut outcomes = Vec::new();
    for e in &entries {
        outcomes.push(run(&checker, e, today).await);
    }
    report(&outcomes, all);
}

async fn run(checker: &ComplianceChecker, e: &Value, today: bool) -> Outcome {
    let id = e["id"].as_str().unwrap_or("?").to_owned();
    let (channel, root) = match e["channel"].as_str() {
        Some("reply") => (Channel::Reply, ""),
        Some("note") => (Channel::Note, "note"),
        _ => (Channel::Request, "input"),
    };
    let cap = &e["capability"];
    let subject = Subject {
        capability_name: cap["name"].as_str().unwrap_or("(note)"),
        capability_description: cap["description"].as_str(),
        grant_purpose: None,
        relationship: None,
        input: &e["value"],
    };
    let schema = (channel == Channel::Request)
        .then(|| cap.get("input_schema"))
        .flatten();
    let started = Instant::now();
    let screened = checker.screen(channel, &subject, root, schema, "").await;
    let elapsed = started.elapsed();
    let (got, answers) = match screened {
        Err(_) => ("too_large".to_owned(), Vec::new()),
        Ok(s) => {
            let got = if s.report["decision"] == "error" {
                "error"
            } else if s.block.is_some() {
                "block"
            } else if s.findings.iter().any(|f| f.action == Action::Redacted) {
                "redact"
            } else if !s.findings.is_empty() {
                "flag"
            } else {
                "deliver"
            };
            (got.to_owned(), sorted_answers(&s.report))
        }
    };
    let today = if today && channel == Channel::Request {
        Some(match checker.check(&subject).await {
            Verdict::Allow { report } if report["decision"] == "error" => ("error", Vec::new()),
            Verdict::Allow { report } => ("allow", sorted_answers(&report)),
            Verdict::Deny { report, .. } => ("deny", sorted_answers(&report)),
        })
    } else {
        None
    };
    Outcome {
        group: id.split('-').next().unwrap_or("?").to_owned(),
        lang: e["lang"].as_str().unwrap_or("?").to_owned(),
        expect: e["expect"].as_str().unwrap_or("?").to_owned(),
        id,
        got,
        answers,
        elapsed,
        today,
    }
}

/// A report's answers, most probable first.
fn sorted_answers(report: &Value) -> Vec<(String, f64)> {
    let mut answers: Vec<(String, f64)> = report["answers"]
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_f64()?)))
                .collect()
        })
        .unwrap_or_default();
    answers.sort_by(|a, b| b.1.total_cmp(&a.1));
    answers
}

fn top(answers: &[(String, f64)], n: usize) -> String {
    answers
        .iter()
        .take(n)
        .map(|(k, p)| format!("{k}={p:.2}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn report(outcomes: &[Outcome], all: bool) {
    if all {
        println!("every message");
        for o in outcomes {
            let mark = if o.got == o.expect { " " } else { "✗" };
            println!(
                "{mark} {:<28} {:<7} expected {:<8} got {:<8} {}",
                o.id,
                o.lang,
                o.expect,
                o.got,
                top(&o.answers, 5)
            );
            if let Some((decision, answers)) = &o.today {
                println!("    today: {decision:<6} {}", top(answers, 3));
            }
        }
        println!();
    }

    let table = |title: &str, key: fn(&Outcome) -> &str| {
        let mut rows: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
        for o in outcomes {
            let r = rows.entry(key(o)).or_default();
            r.1 += 1;
            if o.got == o.expect {
                r.0 += 1;
            }
        }
        println!("{title:<10} {:>6} {:>6}", "pass", "total");
        for (k, (pass, total)) in rows {
            println!("{k:<10} {pass:>6} {total:>6}");
        }
        println!();
    };
    table("group", |o| &o.group);
    table("lang", |o| &o.lang);

    let mut confusion: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for o in outcomes {
        *confusion.entry((&o.expect, &o.got)).or_default() += 1;
    }
    println!("expected → got");
    for ((e, g), n) in &confusion {
        println!("  {e:<8} → {g:<8} {n}");
    }
    println!();

    let misses: Vec<&Outcome> = outcomes.iter().filter(|o| o.got != o.expect).collect();
    println!("{} misses", misses.len());
    for o in misses {
        println!(
            "  {:<28} expected {:<8} got {:<8} {}",
            o.id,
            o.expect,
            o.got,
            top(&o.answers, 3)
        );
    }

    let mut times: Vec<Duration> = outcomes.iter().map(|o| o.elapsed).collect();
    times.sort();
    if !times.is_empty() {
        let at = |q: f64| times[((times.len() - 1) as f64 * q).round() as usize];
        println!("\nlatency: median {:?}, p95 {:?}", at(0.5), at(0.95));
    }
    let today: Vec<&Outcome> = outcomes.iter().filter(|o| o.today.is_some()).collect();
    if !today.is_empty() {
        let denied = today
            .iter()
            .filter(|o| o.today.as_ref().is_some_and(|t| t.0 == "deny"))
            .count();
        println!(
            "compliance check alone (today): {denied} of {} requests denied",
            today.len()
        );
    }
    let pass = outcomes.iter().filter(|o| o.got == o.expect).count();
    println!("passed {pass} of {}", outcomes.len());
}
