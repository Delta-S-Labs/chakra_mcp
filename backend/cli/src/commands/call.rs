//! `chakramcp call` — two-way conversations between agents (calls).
//!
//! A call rings the other agent; once it answers, both sides send messages
//! (and, from the SDKs, call each other's capabilities on the call). Most
//! subcommands act *as* one of your agents: `--as` takes its id or its slug.
//! `pull` claims the agent's call events for a poll loop.

use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::client::ApiClient;
use crate::{print, read_json_arg};

/// The relay's page size for a transcript.
const PAGE: usize = 200;

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Call another agent. It rings until the agent answers or it's missed.
    Start {
        /// The agent to call: its id, `account_slug/agent_slug`, or the
        /// slug of one of your own agents.
        to: String,
        /// Your agent making the call (id or slug).
        #[arg(long = "as", value_name = "AGENT")]
        as_agent: String,
        /// `call`: the relay keeps the call's memory for both agents.
        /// `own`: each agent keeps its own.
        #[arg(long, value_parser = ["call", "own"])]
        memory: Option<String>,
        /// Days the content is kept after the call ends (0 deletes it at the end).
        #[arg(long)]
        ttl_days: Option<u32>,
        /// What the call is about, shown to the other agent's owner.
        #[arg(long)]
        topic: Option<String>,
        /// An opening message, delivered with the ring.
        #[arg(long)]
        message: Option<String>,
    },
    /// List your agents' calls, newest first.
    List {
        /// Only this agent's calls (id or slug).
        #[arg(long = "as", value_name = "AGENT")]
        as_agent: Option<String>,
        #[arg(long, value_parser = ["ringing", "active", "declined", "missed", "ended"])]
        state: Option<String>,
        #[arg(long, default_value_t = 25)]
        limit: i64,
        /// `next_cursor` from the previous page.
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Show a call and its whole transcript.
    Show { call: String },
    /// Claim your agent's new call events, oldest first; each is shown
    /// once: invites, messages, answers and ends. Run it in your poll loop.
    Pull {
        /// Your agent (id or slug).
        #[arg(long = "as", value_name = "AGENT")]
        as_agent: String,
        /// At most 25.
        #[arg(long, default_value_t = 25)]
        limit: i64,
        /// Wait up to this many seconds for events when there are none,
        /// instead of returning an empty list at once. Servers before 0.7
        /// don't wait.
        #[arg(long, default_value_t = 0, value_name = "SECS")]
        wait: u32,
    },
    /// Send a message on an active call.
    Send {
        call: String,
        text: String,
        /// Your agent on the call (id or slug).
        #[arg(long = "as", value_name = "AGENT")]
        as_agent: String,
        /// Structured data to send with the text. Use `@path` for a file.
        #[arg(long)]
        data: Option<String>,
    },
    /// Answer a ringing call.
    Answer {
        call: String,
        #[arg(value_parser = ["accept", "decline"])]
        decision: String,
        /// Your agent on the call (id or slug).
        #[arg(long = "as", value_name = "AGENT")]
        as_agent: String,
        /// A first message, sent when accepting.
        #[arg(long, conflicts_with = "reason")]
        message: Option<String>,
        /// Why, when declining.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Ask the call's memory a question (calls with `--memory call`).
    Ask {
        call: String,
        question: String,
        /// Your agent on the call (id or slug).
        #[arg(long = "as", value_name = "AGENT")]
        as_agent: String,
    },
    /// Hang up. Both agents are told.
    End {
        call: String,
        /// Your agent on the call (id or slug).
        #[arg(long = "as", value_name = "AGENT")]
        as_agent: String,
    },
}

pub async fn run(cmd: Cmd, api: ApiClient) -> Result<()> {
    match cmd {
        Cmd::Start {
            to,
            as_agent,
            memory,
            ttl_days,
            topic,
            message,
        } => {
            let from = my_agent(&api, &as_agent).await?;
            let to = any_agent(&api, &to).await?;
            let body = start_body(&from, &to, memory, ttl_days, topic, message);
            let v: Value = api.post_relay("/v1/calls", &body).await?;
            print(&v)
        }
        Cmd::List {
            as_agent,
            state,
            limit,
            cursor,
        } => {
            let agent = match as_agent {
                Some(a) => Some(my_agent(&api, &a).await?),
                None => None,
            };
            let qs = list_query(agent.as_deref(), state.as_deref(), limit, cursor.as_deref());
            let v: Value = api.get_relay(&format!("/v1/calls?{qs}")).await?;
            print(&v)
        }
        Cmd::Show { call } => {
            let id = call_id(&call)?;
            let c: Value = api.get_relay(&format!("/v1/calls/{id}")).await?;
            let mut entries: Vec<Value> = Vec::new();
            loop {
                let after = entries
                    .last()
                    .and_then(|e| e.get("seq"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let page: Vec<Value> = api
                    .get_relay(&format!(
                        "/v1/calls/{id}/messages?after={after}&limit={PAGE}"
                    ))
                    .await?;
                let n = page.len();
                entries.extend(page);
                if n < PAGE {
                    break;
                }
            }
            print(&json!({ "call": c, "entries": entries }))
        }
        Cmd::Pull {
            as_agent,
            limit,
            wait,
        } => {
            let agent = my_agent(&api, &as_agent).await?;
            let v: Value = api
                .get_relay(&format!(
                    "/v1/inbox/calls?agent_id={agent}&limit={limit}&wait={wait}"
                ))
                .await?;
            print(&v)
        }
        Cmd::Send {
            call,
            text,
            as_agent,
            data,
        } => {
            let id = call_id(&call)?;
            let agent = my_agent(&api, &as_agent).await?;
            let mut body = json!({ "agent_id": agent, "text": text });
            if let Some(d) = data {
                body["data"] = read_json_arg(&d)?;
            }
            let v: Value = api
                .post_relay(&format!("/v1/calls/{id}/messages"), &body)
                .await?;
            print(&v)
        }
        Cmd::Answer {
            call,
            decision,
            as_agent,
            message,
            reason,
        } => {
            let id = call_id(&call)?;
            let agent = my_agent(&api, &as_agent).await?;
            let (path, body) = answer_body(&id, &decision, &agent, message, reason)?;
            let v: Value = api.post_relay(&path, &body).await?;
            print(&v)
        }
        Cmd::Ask {
            call,
            question,
            as_agent,
        } => {
            let id = call_id(&call)?;
            let agent = my_agent(&api, &as_agent).await?;
            let v: Value = api
                .post_relay(
                    &format!("/v1/calls/{id}/memory/ask"),
                    &json!({ "agent_id": agent, "question": question }),
                )
                .await?;
            print(&v)
        }
        Cmd::End { call, as_agent } => {
            let id = call_id(&call)?;
            let agent = my_agent(&api, &as_agent).await?;
            let v: Value = api
                .post_relay(
                    &format!("/v1/calls/{id}/end"),
                    &json!({ "agent_id": agent }),
                )
                .await?;
            print(&v)
        }
    }
}

fn call_id(s: &str) -> Result<Uuid> {
    Uuid::parse_str(s).with_context(|| format!("`{s}` isn't a call id"))
}

/// One of your agents, by id, slug or `account/agent`.
async fn my_agent(api: &ApiClient, ident: &str) -> Result<String> {
    Ok(crate::resolve::mine_id(api, ident).await?.to_string())
}

/// The agent to call: an id, `account/agent`, or a bare slug (yours first).
async fn any_agent(api: &ApiClient, ident: &str) -> Result<String> {
    Ok(crate::resolve::peer_id(api, ident).await?.to_string())
}

fn start_body(
    from: &str,
    to: &str,
    memory: Option<String>,
    ttl_days: Option<u32>,
    topic: Option<String>,
    message: Option<String>,
) -> Value {
    let mut body = json!({ "from_agent_id": from, "to_agent_id": to });
    if let Some(m) = memory {
        body["memory"] = json!(m);
    }
    if let Some(d) = ttl_days {
        body["ttl_secs"] = json!(u64::from(d) * 86_400);
    }
    if let Some(t) = topic {
        body["topic"] = json!(t);
    }
    if let Some(m) = message {
        body["message"] = json!(m);
    }
    body
}

fn list_query(
    agent: Option<&str>,
    state: Option<&str>,
    limit: i64,
    cursor: Option<&str>,
) -> String {
    let mut qs = url::form_urlencoded::Serializer::new(String::new());
    qs.append_pair("limit", &limit.to_string());
    if let Some(a) = agent {
        qs.append_pair("agent_id", a);
    }
    if let Some(s) = state {
        qs.append_pair("state", s);
    }
    if let Some(c) = cursor {
        qs.append_pair("cursor", c);
    }
    qs.finish()
}

fn answer_body(
    id: &Uuid,
    decision: &str,
    agent: &str,
    message: Option<String>,
    reason: Option<String>,
) -> Result<(String, Value)> {
    let mut body = json!({ "agent_id": agent });
    match decision {
        "accept" => {
            if reason.is_some() {
                anyhow::bail!("--reason is for declining; use --message when accepting");
            }
            if let Some(m) = message {
                body["message"] = json!(m);
            }
            Ok((format!("/v1/calls/{id}/accept"), body))
        }
        "decline" => {
            if message.is_some() {
                anyhow::bail!("--message is for accepting; use --reason when declining");
            }
            if let Some(r) = reason {
                body["reason"] = json!(r);
            }
            Ok((format!("/v1/calls/{id}/decline"), body))
        }
        other => anyhow::bail!("answer with `accept` or `decline`, not `{other}`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_body_carries_only_what_was_given() {
        let v = start_body("a", "b", None, None, None, None);
        assert_eq!(v, json!({ "from_agent_id": "a", "to_agent_id": "b" }));

        let v = start_body(
            "a",
            "b",
            Some("call".into()),
            Some(7),
            Some("Venue".into()),
            Some("hi".into()),
        );
        assert_eq!(v["memory"], json!("call"));
        assert_eq!(v["ttl_secs"], json!(7 * 86_400));
        assert_eq!(v["topic"], json!("Venue"));
        assert_eq!(v["message"], json!("hi"));
    }

    #[test]
    fn zero_ttl_days_deletes_at_the_end() {
        let v = start_body("a", "b", None, Some(0), None, None);
        assert_eq!(v["ttl_secs"], json!(0));
    }

    #[test]
    fn list_query_encodes_the_cursor() {
        assert_eq!(list_query(None, None, 25, None), "limit=25");
        assert_eq!(
            list_query(Some("x"), Some("active"), 10, Some("a+b/c=")),
            "limit=10&agent_id=x&state=active&cursor=a%2Bb%2Fc%3D"
        );
    }

    #[test]
    fn answer_picks_the_route_and_its_field() {
        let id = Uuid::nil();
        let (path, body) = answer_body(&id, "accept", "me", Some("hello".into()), None).unwrap();
        assert_eq!(path, format!("/v1/calls/{id}/accept"));
        assert_eq!(body, json!({ "agent_id": "me", "message": "hello" }));

        let (path, body) = answer_body(&id, "decline", "me", None, Some("busy".into())).unwrap();
        assert_eq!(path, format!("/v1/calls/{id}/decline"));
        assert_eq!(body, json!({ "agent_id": "me", "reason": "busy" }));

        assert!(answer_body(&id, "decline", "me", Some("hi".into()), None).is_err());
        assert!(answer_body(&id, "accept", "me", None, Some("no".into())).is_err());
    }

    #[test]
    fn call_ids_must_be_uuids() {
        assert!(call_id("not-a-call").is_err());
        assert!(call_id(&Uuid::nil().to_string()).is_ok());
    }
}
