//! Naming agents on the command line (MCP and discovery spec §4.3).
//!
//! - [`mine`]: one of your agents, as an id, `account/agent`, or a slug unique
//!   among yours (`--as`, `--from`).
//! - [`peer`]: an agent you address, as an id, `account/agent`, or a bare slug:
//!   one of yours first, then one unique on the network.
//!
//! Both return the agent's row (`id`, `slug`, `account_slug`, …). A name that
//! matches nothing, or several agents, is an `InvalidArgs` error (exit 6).

use serde_json::Value;
use uuid::Uuid;

use crate::client::ApiClient;
use crate::commands::invoke::classify_http_err;
use crate::waiters::WaitError;

/// `/v1/network/agents` pages fetched at most, 100 agents each.
const MAX_PAGES: usize = 50;

/// One of your agents.
pub async fn mine(api: &ApiClient, ident: &str) -> Result<Value, WaitError> {
    let agents = my_agents(api).await?;
    one(
        pick(&agents, ident),
        ident,
        "of your agents",
        "see `chakramcp agents list`",
    )
}

/// An agent you address: yours, or one on the network.
pub async fn peer(api: &ApiClient, ident: &str) -> Result<Value, WaitError> {
    if let Ok(id) = Uuid::parse_str(ident) {
        return api
            .get_relay(&format!("/v1/agents/{id}"))
            .await
            .map_err(|e| classify_http_err(&e));
    }
    let agents = my_agents(api).await?;
    let own = pick(&agents, ident);
    if !own.is_empty() {
        return one(own, ident, "of your agents", "");
    }
    let network = network_agents(api).await?;
    one(
        pick(&network, ident),
        ident,
        "on the network",
        "see `chakramcp network`, or name it as `account/agent`",
    )
}

/// The id of [`mine`].
pub async fn mine_id(api: &ApiClient, ident: &str) -> Result<Uuid, WaitError> {
    id_of(&mine(api, ident).await?)
}

/// The id of [`peer`].
pub async fn peer_id(api: &ApiClient, ident: &str) -> Result<Uuid, WaitError> {
    id_of(&peer(api, ident).await?)
}

async fn my_agents(api: &ApiClient) -> Result<Vec<Value>, WaitError> {
    let v: Value = api
        .get_relay("/v1/agents")
        .await
        .map_err(|e| classify_http_err(&e))?;
    Ok(v.as_array().cloned().unwrap_or_default())
}

/// Every agent `/v1/network/agents` shows, page by page.
async fn network_agents(api: &ApiClient) -> Result<Vec<Value>, WaitError> {
    let mut agents = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let path = match &cursor {
            Some(c) => format!(
                "/v1/network/agents?limit=100&cursor={}",
                url::form_urlencoded::byte_serialize(c.as_bytes()).collect::<String>()
            ),
            None => "/v1/network/agents?limit=100".to_string(),
        };
        let page: Value = api
            .get_relay(&path)
            .await
            .map_err(|e| classify_http_err(&e))?;
        let (rows, next) = page_of(page);
        agents.extend(rows);
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    Ok(agents)
}

/// A network page's agents and next cursor. An older relay's plain list is a
/// single page.
fn page_of(page: Value) -> (Vec<Value>, Option<String>) {
    match page {
        Value::Array(rows) => (rows, None),
        Value::Object(mut o) => {
            let rows = match o.remove("agents") {
                Some(Value::Array(rows)) => rows,
                _ => Vec::new(),
            };
            let next = o
                .remove("next_cursor")
                .and_then(|c| c.as_str().map(str::to_owned));
            (rows, next)
        }
        _ => (Vec::new(), None),
    }
}

/// The agents `ident` names: by id, by `account/agent`, or by bare slug.
fn pick<'a>(agents: &'a [Value], ident: &str) -> Vec<&'a Value> {
    let field = |a: &Value, k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    if Uuid::parse_str(ident).is_ok() {
        return agents.iter().filter(|a| field(a, "id") == ident).collect();
    }
    let (account, slug) = match ident.split_once('/') {
        Some((account, slug)) => (Some(account), slug),
        None => (None, ident),
    };
    agents
        .iter()
        .filter(|a| {
            field(a, "slug") == slug && account.is_none_or(|acc| field(a, "account_slug") == acc)
        })
        .collect()
}

fn one(found: Vec<&Value>, ident: &str, where_: &str, hint: &str) -> Result<Value, WaitError> {
    match found.as_slice() {
        [only] => Ok((*only).clone()),
        [] => Err(WaitError::InvalidArgs(
            format!("no agent `{ident}` {where_}; {hint}")
                .trim_end_matches("; ")
                .to_owned(),
        )),
        several => {
            let names: Vec<String> = several
                .iter()
                .map(|a| {
                    format!(
                        "{}/{}",
                        a.get("account_slug").and_then(Value::as_str).unwrap_or("?"),
                        a.get("slug").and_then(Value::as_str).unwrap_or("?")
                    )
                })
                .collect();
            Err(WaitError::InvalidArgs(format!(
                "{} agents match `{ident}`; name one as `account/agent`: {}",
                several.len(),
                names.join(", ")
            )))
        }
    }
}

fn id_of(agent: &Value) -> Result<Uuid, WaitError> {
    agent
        .get("id")
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| WaitError::Transient("agent row has no id".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn agents() -> Vec<Value> {
        vec![
            json!({ "id": "00000000-0000-0000-0000-000000000001", "slug": "bot", "account_slug": "alice" }),
            json!({ "id": "00000000-0000-0000-0000-000000000002", "slug": "bot", "account_slug": "bob" }),
            json!({ "id": "00000000-0000-0000-0000-000000000003", "slug": "planner", "account_slug": "bob" }),
        ]
    }

    #[test]
    fn names_by_id_account_slug_and_bare_slug() {
        let a = agents();
        assert_eq!(pick(&a, "00000000-0000-0000-0000-000000000003").len(), 1);
        assert_eq!(pick(&a, "alice/bot")[0]["account_slug"], "alice");
        assert_eq!(pick(&a, "planner").len(), 1);
        assert_eq!(pick(&a, "bot").len(), 2);
        assert!(pick(&a, "carol/bot").is_empty());
    }

    #[test]
    fn one_match_or_a_helpful_error() {
        let a = agents();
        assert!(one(pick(&a, "planner"), "planner", "on the network", "").is_ok());
        let several = one(pick(&a, "bot"), "bot", "on the network", "").unwrap_err();
        assert!(
            several.to_string().contains("alice/bot, bob/bot"),
            "{several}"
        );
        let none = one(
            pick(&a, "x"),
            "x",
            "of your agents",
            "see `chakramcp agents list`",
        )
        .unwrap_err();
        assert!(
            none.to_string().contains("see `chakramcp agents list`"),
            "{none}"
        );
    }

    #[test]
    fn reads_pages_and_older_lists() {
        let (rows, next) = page_of(json!({ "agents": [{"id": "a"}], "next_cursor": "c2" }));
        assert_eq!((rows.len(), next.as_deref()), (1, Some("c2")));
        let (rows, next) = page_of(json!({ "agents": [] }));
        assert_eq!((rows.len(), next), (0, None));
        let (rows, next) = page_of(json!([{"id": "a"}, {"id": "b"}]));
        assert_eq!((rows.len(), next), (2, None));
    }
}
