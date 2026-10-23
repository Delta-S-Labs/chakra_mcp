//! Message safety screening (spec `2026-10-23-message-safety-design.md`
//! §4–§4.3): the code pass, Jev requests that always fit, and the policy
//! that turns the answers into block, redact or flag.

use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

use serde_json::{json, Map, Value};
use tokio::task::JoinSet;

use super::questions::{
    self, act_threshold, Shape, ACT, ACT_HARM_MINORS, ACT_NON_ENGLISH, BLOCKING_COMPLIANCE,
    BLOCKING_SAFETY, CONTACT_PREFIX, OFF_PLATFORM,
};
use super::{post, ComplianceChecker, Subject};
use crate::safety::{
    self, Action, Channel, Chunk, Context, Finding, FindingKind, Plan, Scan, TooLarge,
};

/// Why a message was blocked: the reason the sender sees, and its code.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub reason: String,
    /// `chk.policy.compliance_denied` (today's questions) or
    /// `chk.policy.safety_blocked` (`harm_*`).
    pub code: &'static str,
}

/// A screened message.
#[derive(Debug, Clone)]
pub struct Screened {
    /// The message as delivered: redacted where contacts were removed.
    pub value: Value,
    /// What screening found and did, for the `safety` field and metrics.
    pub findings: Vec<Finding>,
    /// Set when the message must not be delivered.
    pub block: Option<Block>,
    /// For `trust_snapshot`: probabilities, language, decision, findings.
    /// Never holds a removed value.
    pub report: Value,
}

impl ComplianceChecker {
    /// Screen `subject.input` (spec §4). `root` prefixes paths; `schema`
    /// is the capability's schema for the part of the message at
    /// `schema_root`. Refuses (never cuts) a message over 1 MiB of text.
    /// Fail-open: an outage delivers the message unchanged.
    pub async fn screen(
        &self,
        channel: Channel,
        subject: &Subject<'_>,
        root: &str,
        schema: Option<&Value>,
        schema_root: &str,
    ) -> Result<Screened, TooLarge> {
        let started = Instant::now();
        let scan = safety::scan(subject.input, root, schema, schema_root)?;
        let rel = subject.relationship;
        let context = Context {
            capability_name: Some(subject.capability_name.to_owned()),
            capability_description: subject.capability_description.map(str::to_owned),
            grant_purpose: subject.grant_purpose.map(str::to_owned),
            consent_purpose: None,
            proposer_message: rel.and_then(|r| r.proposer_message).map(str::to_owned),
            response_message: rel.and_then(|r| r.response_message).map(str::to_owned),
        };
        let plan = safety::plan(channel, &scan, &context);
        let answered = self.ask(channel, subject, &plan).await;
        let decided = decide(channel, &scan, &plan, &answered);
        let value = if decided.block.is_some() {
            subject.input.clone()
        } else {
            safety::redact(subject.input, &scan, &decided.redact)
        };
        let elapsed = started.elapsed();
        safety::metrics::record(channel, &decided.findings, elapsed);
        let mut report = json!({
            "model": answered.model.as_deref().unwrap_or(&self.model),
            "decision": decided.decision,
            "language": { "code": scan.language.code, "english": scan.language.english },
            "thresholds": {
                "act": ACT,
                "act_non_english": ACT_NON_ENGLISH,
                "harm_minors": ACT_HARM_MINORS,
            },
            "answers": answered.answers,
            "chunks": answered.chunks,
            "failed_chunks": answered.failed,
            "findings": decided.findings,
            "elapsed_ms": elapsed.as_millis() as u64,
        });
        if !answered.errors.is_empty() {
            report["errors"] = json!(answered.errors);
        }
        if answered.failed > 0 {
            tracing::warn!(
                failed = answered.failed,
                chunks = answered.chunks,
                "safety screening: TypeSafe failed for some chunks (fail-open)"
            );
        }
        Ok(Screened {
            value,
            findings: decided.findings,
            block: decided.block,
            report,
        })
    }

    /// Send every chunk at once; the highest probability per question wins.
    async fn ask(&self, channel: Channel, subject: &Subject<'_>, plan: &Plan) -> Answered {
        let mut set = JoinSet::new();
        for (i, chunk) in plan.chunks.iter().enumerate() {
            let mut q = Map::new();
            if channel == Channel::Request {
                if let Value::Object(m) = questions::questions_in(subject, Shape::Fields) {
                    q.extend(m);
                }
            }
            let ids: &[usize] = if i == 0 { &plan.judged } else { &[] };
            if let Value::Object(m) = questions::safety_questions(channel, i == 0, ids) {
                q.extend(m);
            }
            let (http, endpoint, key, model) = (
                self.http.clone(),
                self.endpoint.clone(),
                self.api_key.clone(),
                self.model.clone(),
            );
            let chunk = chunk.clone();
            set.spawn(async move {
                let r = ask_chunk(
                    &http,
                    &endpoint,
                    &key,
                    &model,
                    &chunk,
                    channel,
                    Value::Object(q),
                )
                .await;
                (i, r)
            });
        }
        let mut a = Answered {
            chunks: plan.chunks.len(),
            ..Answered::default()
        };
        while let Some(joined) = set.join_next().await {
            let Ok((i, result)) = joined else {
                a.failed += 1;
                continue;
            };
            match result {
                ChunkResult::Answered {
                    answers,
                    model,
                    leftover,
                } => {
                    if i == 0 {
                        a.first_answered = true;
                    }
                    a.model = a.model.or(model);
                    for (id, p) in answers {
                        let best = a.answers.entry(id).or_insert(p);
                        *best = best.max(p);
                    }
                    a.unjudged_paths.extend(leftover);
                }
                ChunkResult::Failed { paths, error } => {
                    a.failed += 1;
                    a.unjudged_paths.extend(paths);
                    a.errors.push(error);
                }
            }
        }
        a
    }
}

#[derive(Debug, Default)]
struct Answered {
    answers: BTreeMap<String, f64>,
    model: Option<String>,
    chunks: usize,
    failed: usize,
    /// The first chunk (with the candidate questions) answered.
    first_answered: bool,
    /// Paths of text no chunk judged (failed chunks, a halved remainder).
    unjudged_paths: Vec<String>,
    errors: Vec<String>,
}

enum ChunkResult {
    Answered {
        answers: Vec<(String, f64)>,
        model: Option<String>,
        leftover: Vec<String>,
    },
    Failed {
        paths: Vec<String>,
        error: String,
    },
}

fn field_paths(fields: &[Value]) -> Vec<String> {
    fields
        .iter()
        .filter_map(|f| f["path"].as_str().map(str::to_owned))
        .collect()
}

/// One chunk's request. A 4xx (a request the relay shouldn't have built)
/// is retried once at half size; anything else is an outage for this chunk.
async fn ask_chunk(
    http: &reqwest::Client,
    endpoint: &str,
    key: &str,
    model: &str,
    chunk: &Chunk,
    channel: Channel,
    questions: Value,
) -> ChunkResult {
    let body = |state: &Value| json!({ "model": model, "state": state, "questions": questions });
    let first = post(http, endpoint, key, &body(&chunk.state)).await;
    let (resp, leftover) = match first {
        Ok(resp) => (Ok(resp), Vec::new()),
        Err(e) if e.rejected() => {
            tracing::error!(error = %e.message, "TypeSafe refused a safety chunk; retrying at half size");
            let (half, rest) = safety::halve(chunk, channel);
            (
                post(http, endpoint, key, &body(&half.state)).await,
                field_paths(&rest),
            )
        }
        Err(e) => (Err(e), Vec::new()),
    };
    match resp {
        Ok(resp) => {
            let answers: Vec<(String, f64)> = resp
                .get("answers")
                .and_then(Value::as_object)
                .map(|m| {
                    m.iter()
                        .filter_map(|(id, a)| Some((id.clone(), a.get("noul")?.as_f64()?)))
                        .collect()
                })
                .unwrap_or_default();
            if answers.is_empty() {
                return ChunkResult::Failed {
                    paths: field_paths(&chunk.fields),
                    error: "response carried no answers".into(),
                };
            }
            ChunkResult::Answered {
                answers,
                model: resp.get("model").and_then(Value::as_str).map(str::to_owned),
                leftover,
            }
        }
        Err(e) => ChunkResult::Failed {
            paths: field_paths(&chunk.fields),
            error: e.message,
        },
    }
}

struct Decided {
    findings: Vec<Finding>,
    redact: Vec<usize>,
    block: Option<Block>,
    decision: &'static str,
}

/// The policy (§4.3): block, redact or flag, by language.
fn decide(channel: Channel, scan: &Scan, plan: &Plan, a: &Answered) -> Decided {
    let english = scan.language.english;
    let path_of = |candidate: usize| {
        let c = &scan.candidates[candidate];
        scan.items[c.item(&scan.decoded)].path.clone()
    };
    let judged =
        |id: &str, p: f64, kind: FindingKind, action: Action, path: Option<String>| Finding {
            kind,
            action,
            path,
            question: Some(id.to_owned()),
            p: Some(p),
        };
    let mut findings = Vec::new();
    let mut redact = Vec::new();
    let mut hits: Vec<(String, f64)> = Vec::new();
    let all_failed = a.chunks > 0 && a.failed == a.chunks;

    if all_failed {
        // Fail-open: nothing is redacted without Jev's judgment.
        for c in &scan.candidates {
            findings.push(Finding::code(
                c.kind.finding(),
                Action::Flagged,
                Some(path_of(c.id)),
            ));
        }
    } else {
        for (id, &p) in &a.answers {
            let act = act_threshold(id, english);
            let band = !english && p >= ACT && p < act;
            let blocking = BLOCKING_SAFETY.contains(&id.as_str())
                || BLOCKING_COMPLIANCE.contains(&id.as_str());
            if blocking {
                let Some(kind) = FindingKind::from_question(id) else {
                    continue;
                };
                if p >= act {
                    hits.push((id.clone(), p));
                    findings.push(judged(id, p, kind, Action::Blocked, None));
                } else if band {
                    findings.push(judged(id, p, kind, Action::Flagged, None));
                }
            } else if let Some(cid) = id
                .strip_prefix(CONTACT_PREFIX)
                .and_then(|n| n.parse::<usize>().ok())
                .filter(|cid| *cid < scan.candidates.len())
            {
                let kind = scan.candidates[cid].kind.finding();
                if p >= act {
                    redact.push(cid);
                    findings.push(judged(id, p, kind, Action::Redacted, Some(path_of(cid))));
                } else if band {
                    findings.push(judged(id, p, kind, Action::Flagged, Some(path_of(cid))));
                }
            } else if id == OFF_PLATFORM {
                if p >= act && !plan.unjudged_candidates.is_empty() {
                    for &cid in &plan.unjudged_candidates {
                        redact.push(cid);
                        let kind = scan.candidates[cid].kind.finding();
                        findings.push(judged(id, p, kind, Action::Redacted, Some(path_of(cid))));
                    }
                } else if p >= act || band {
                    findings.push(judged(
                        id,
                        p,
                        FindingKind::OffPlatform,
                        Action::Flagged,
                        None,
                    ));
                }
            }
        }
        // The candidate questions went unanswered: flag, don't redact.
        if !a.first_answered {
            for &cid in &plan.judged {
                let kind = scan.candidates[cid].kind.finding();
                findings.push(Finding::code(kind, Action::Flagged, Some(path_of(cid))));
            }
        }
    }

    findings.extend(scan.opaque_findings());
    let mut seen = HashSet::new();
    let unjudged: Vec<String> = plan
        .unjudged
        .iter()
        .filter_map(|f| f.path.clone())
        .chain(a.unjudged_paths.iter().cloned())
        .filter(|p| seen.insert(p.clone()))
        .collect();
    for path in unjudged {
        findings.push(Finding::code(
            FindingKind::Unjudged,
            Action::Flagged,
            Some(path),
        ));
    }

    let block = block_for(channel, &hits);
    let decision = if block.is_some() {
        "deny"
    } else if all_failed {
        "error"
    } else {
        "allow"
    };
    Decided {
        findings,
        redact,
        block,
        decision,
    }
}

/// The block for the questions that crossed their threshold: the harm
/// questions first (`safety_blocked`), else today's (`compliance_denied`,
/// in today's wording).
fn block_for(channel: Channel, hits: &[(String, f64)]) -> Option<Block> {
    if hits.is_empty() {
        return None;
    }
    let harms: Vec<&(String, f64)> = hits
        .iter()
        .filter(|(id, _)| BLOCKING_SAFETY.contains(&id.as_str()))
        .collect();
    let list = |hits: &[&(String, f64)]| {
        hits.iter()
            .map(|(id, p)| format!("{} ({id}={p:.2})", questions::label(id)))
            .collect::<Vec<_>>()
            .join("; ")
    };
    if !harms.is_empty() {
        return Some(Block {
            reason: format!(
                "System One safety check blocked this {}: {}",
                channel.as_str(),
                list(&harms)
            ),
            code: "chk.policy.safety_blocked",
        });
    }
    let all: Vec<&(String, f64)> = hits.iter().collect();
    Some(Block {
        reason: format!(
            "System One compliance check denied this request: {}",
            list(&all)
        ),
        code: "chk.policy.compliance_denied",
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::*;
    use crate::system_one::test_support::{checker, serve, Fake};
    use crate::system_one::Relationship;

    fn subject(input: &Value) -> Subject<'_> {
        Subject {
            capability_name: "book_meeting",
            capability_description: Some("Books a meeting with the given attendees."),
            grant_purpose: Some("Schedule the weekly sync"),
            relationship: Some(Relationship {
                proposer_message: Some("Let's coordinate meetings"),
                response_message: None,
            }),
            input,
        }
    }

    async fn screen_with(fake: Fake, input: Value) -> (Screened, Fake) {
        let url = serve(fake.clone()).await;
        let c = checker(&url).with_safety(true);
        let s = c
            .screen(Channel::Request, &subject(&input), "input", None, "")
            .await
            .unwrap();
        (s, fake)
    }

    fn fake(nouls: &[(&'static str, f64)]) -> Fake {
        Fake {
            nouls: nouls.iter().copied().collect::<HashMap<_, _>>(),
            ..Fake::default()
        }
    }

    const ENGLISH: &str =
        "Please book the weekly sync with the design team for Friday afternoon, thanks a lot.";

    #[tokio::test]
    async fn delivers_untouched_when_nothing_fires() {
        let input = json!({ "message": ENGLISH, "attendees": ["ana@example.com"] });
        let (s, f) = screen_with(fake(&[]), input.clone()).await;
        assert!(s.block.is_none());
        assert_eq!(s.value, input);
        assert!(s.findings.is_empty(), "{:?}", s.findings);
        assert_eq!(s.report["decision"], "allow");
        let body = f.last_body.lock().unwrap().clone().unwrap();
        // One request with today's questions in the fields shape, the safety
        // questions and one per candidate.
        let q = body["questions"].as_object().unwrap();
        for id in [
            "off_capability",
            "off_purpose",
            "off_relationship",
            "prompt_injection",
            "data_exfiltration",
            "harm_crime",
            "harm_minors",
            "off_platform",
            "contact_0",
        ] {
            assert!(q.contains_key(id), "{id}");
        }
        assert!(q["off_capability"]["instructions"]
            .as_str()
            .unwrap()
            .contains("request.fields"));
        assert_eq!(body["state"]["candidates"][0]["text"], "ana@example.com");
        assert_eq!(f.bodies.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn redacts_the_contact_jev_flags_and_keeps_the_one_the_task_needs() {
        let input = json!({
            "message": format!("{ENGLISH} Also, text me on +91 98450 12345 instead."),
            "attendees": ["ana@example.com"],
        });
        // Candidates are numbered in walk order: the attendee (0), then the phone (1).
        let (s, _) = screen_with(fake(&[("contact_1", 0.95), ("contact_0", 0.05)]), input).await;
        assert!(s.block.is_none());
        assert_eq!(s.value["attendees"][0], "ana@example.com");
        assert!(s.value["message"]
            .as_str()
            .unwrap()
            .contains("text me on [contact removed] instead"));
        let f = &s.findings[0];
        assert_eq!((f.kind, f.action), (FindingKind::Phone, Action::Redacted));
        assert_eq!(f.path.as_deref(), Some("input.message"));
        assert_eq!(f.question.as_deref(), Some("contact_1"));
        // The report never holds the removed value.
        assert!(!s.report.to_string().contains("98450"));
    }

    #[tokio::test]
    async fn blocks_harm_with_its_own_code_and_compliance_with_todays() {
        let input = json!({ "message": ENGLISH });
        let (s, _) = screen_with(fake(&[("harm_crime", 0.93)]), input.clone()).await;
        let b = s.block.unwrap();
        assert_eq!(b.code, "chk.policy.safety_blocked");
        assert!(
            b.reason
                .contains("help with fraud or other crime (harm_crime=0.93)"),
            "{}",
            b.reason
        );
        assert_eq!(s.report["decision"], "deny");

        let (s, _) = screen_with(fake(&[("off_purpose", 0.9)]), input).await;
        let b = s.block.unwrap();
        assert_eq!(b.code, "chk.policy.compliance_denied");
        assert!(b
            .reason
            .starts_with("System One compliance check denied this request: "));
    }

    #[tokio::test]
    async fn non_english_findings_in_the_band_are_flagged_not_acted_on() {
        let input = json!({ "message": "¿Puedes reservar un vuelo de Madrid a Barcelona para el martes? Escríbeme a ana@example.com" });
        let (s, _) = screen_with(
            fake(&[("harm_crime", 0.9), ("contact_0", 0.9)]),
            input.clone(),
        )
        .await;
        assert!(s.block.is_none());
        assert_eq!(s.value, input);
        assert!(
            s.findings.iter().all(|f| f.action == Action::Flagged),
            "{:?}",
            s.findings
        );
        assert_eq!(s.findings.len(), 2);

        let (s, _) = screen_with(fake(&[("harm_crime", 0.98)]), input.clone()).await;
        assert!(s.block.is_some());
        // harm_minors acts at 0.5 in every language.
        let (s, _) = screen_with(fake(&[("harm_minors", 0.6)]), input).await;
        assert_eq!(s.block.unwrap().code, "chk.policy.safety_blocked");
    }

    #[tokio::test]
    async fn off_platform_redacts_candidates_past_the_sixteenth_or_flags() {
        let emails: Vec<String> = (0..20).map(|i| format!("person{i}@example.com")).collect();
        let input = json!({ "message": ENGLISH, "to": emails });
        let (s, _) = screen_with(fake(&[("off_platform", 0.9)]), input).await;
        let to = s.value["to"].as_array().unwrap();
        assert_eq!(to[0], "person0@example.com");
        assert!(to[16..].iter().all(|v| v == "[contact removed]"), "{to:?}");

        let (s, _) = screen_with(
            fake(&[("off_platform", 0.9)]),
            json!({ "message": format!("{ENGLISH} Let's continue on Telegram.") }),
        )
        .await;
        assert_eq!(s.findings.len(), 1);
        assert_eq!(
            (s.findings[0].kind, s.findings[0].action),
            (FindingKind::OffPlatform, Action::Flagged)
        );
    }

    #[tokio::test]
    async fn fails_open_and_flags_what_code_found() {
        let input = json!({ "message": format!("{ENGLISH} mail me at bob@example.com") });
        let f = Fake {
            status: Some(503),
            ..Fake::default()
        };
        let (s, _) = screen_with(f, input.clone()).await;
        assert!(s.block.is_none());
        assert_eq!(s.value, input);
        assert_eq!(s.report["decision"], "error");
        assert_eq!(
            (s.findings[0].kind, s.findings[0].action),
            (FindingKind::Email, Action::Flagged)
        );
        assert!(s.findings[0].question.is_none());
    }

    #[tokio::test]
    async fn a_refused_chunk_is_retried_at_half_size() {
        let input = json!({ "message": ENGLISH, "notes": "second field" });
        let f = fake(&[]);
        f.statuses.lock().unwrap().push_back(413);
        let (s, f) = screen_with(f, input).await;
        assert_eq!(s.report["decision"], "allow");
        assert_eq!(f.bodies.lock().unwrap().len(), 2);
        assert!(
            s.findings.iter().any(|x| x.kind == FindingKind::Unjudged),
            "{:?}",
            s.findings
        );
    }

    #[tokio::test]
    async fn padding_is_split_into_requests_that_all_fit() {
        let input = json!({ "pad": "\u{1}".repeat(200 * 1024), "tail": "bob@example.com" });
        let (s, f) = screen_with(fake(&[]), input).await;
        let bodies = f.bodies.lock().unwrap().clone();
        assert_eq!(bodies.len(), safety::MAX_CHUNKS);
        for b in &bodies {
            assert!(
                serde_json::to_string(&b["state"]).unwrap().len() <= safety::STATE_BUDGET_BYTES
            );
        }
        assert!(s.findings.iter().any(|x| x.kind == FindingKind::Unjudged));
    }

    #[tokio::test]
    async fn refuses_over_a_mebibyte() {
        let url = serve(Fake::default()).await;
        let c = checker(&url).with_safety(true);
        let input = json!("x".repeat(safety::MAX_WALK_BYTES + 1));
        let r = c
            .screen(Channel::Request, &subject(&input), "input", None, "")
            .await;
        assert!(r.is_err());
    }
}
