//! Sub-clients - agents, friendships, grants, invocations.
//! Each is a thin handle over the parent ChakraMCP that namespaces a
//! group of endpoints.

use serde::Serialize;
use serde_json::json;

use crate::client::ChakraMCP;
use crate::error::Result;
use crate::types::*;

pub struct AgentsClient<'a> {
    parent: &'a ChakraMCP,
    pub capabilities: CapabilitiesClient<'a>,
}

impl<'a> AgentsClient<'a> {
    pub(crate) fn new(parent: &'a ChakraMCP) -> Self {
        Self {
            parent,
            capabilities: CapabilitiesClient { parent },
        }
    }

    pub async fn list(&self) -> Result<Vec<Agent>> {
        self.parent.relay_get("/v1/agents").await
    }
    pub async fn get(&self, id: &str) -> Result<Agent> {
        self.parent
            .relay_get(&format!("/v1/agents/{}", urlencode(id)))
            .await
    }
    pub async fn create(&self, body: &CreateAgentRequest) -> Result<Agent> {
        self.parent.relay_post("/v1/agents", body).await
    }
    pub async fn update(&self, id: &str, body: &UpdateAgentRequest) -> Result<Agent> {
        self.parent
            .relay_patch(&format!("/v1/agents/{}", urlencode(id)), body)
            .await
    }
    pub async fn delete(&self, id: &str) -> Result<()> {
        self.parent
            .relay_delete(&format!("/v1/agents/{}", urlencode(id)))
            .await
    }
}

pub struct CapabilitiesClient<'a> {
    parent: &'a ChakraMCP,
}

impl CapabilitiesClient<'_> {
    pub async fn list(&self, agent_id: &str) -> Result<Vec<Capability>> {
        self.parent
            .relay_get(&format!("/v1/agents/{}/capabilities", urlencode(agent_id)))
            .await
    }
    pub async fn create(
        &self,
        agent_id: &str,
        body: &CreateCapabilityRequest,
    ) -> Result<Capability> {
        self.parent
            .relay_post(&format!("/v1/agents/{}/capabilities", urlencode(agent_id)), body)
            .await
    }
    pub async fn delete(&self, agent_id: &str, capability_id: &str) -> Result<()> {
        self.parent
            .relay_delete(&format!(
                "/v1/agents/{}/capabilities/{}",
                urlencode(agent_id),
                urlencode(capability_id)
            ))
            .await
    }
}

pub struct FriendshipsClient<'a> {
    parent: &'a ChakraMCP,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ListFriendshipsOpts<'a> {
    pub direction: Option<&'a str>,
    pub status: Option<FriendshipStatus>,
}

impl<'a> FriendshipsClient<'a> {
    pub(crate) fn new(parent: &'a ChakraMCP) -> Self {
        Self { parent }
    }

    pub async fn list(&self, opts: ListFriendshipsOpts<'_>) -> Result<Vec<Friendship>> {
        let mut q = Vec::new();
        if let Some(d) = opts.direction {
            q.push(format!("direction={}", urlencode(d)));
        }
        if let Some(s) = opts.status {
            q.push(format!("status={}", serde_plain(&s)));
        }
        let qs = if q.is_empty() {
            String::new()
        } else {
            format!("?{}", q.join("&"))
        };
        self.parent
            .relay_get(&format!("/v1/friendships{qs}"))
            .await
    }

    pub async fn get(&self, id: &str) -> Result<Friendship> {
        self.parent
            .relay_get(&format!("/v1/friendships/{}", urlencode(id)))
            .await
    }

    pub async fn propose(&self, body: &ProposeFriendshipRequest) -> Result<Friendship> {
        self.parent.relay_post("/v1/friendships", body).await
    }

    pub async fn accept(&self, id: &str, message: Option<&str>) -> Result<Friendship> {
        self.parent
            .relay_post(
                &format!("/v1/friendships/{}/accept", urlencode(id)),
                &json!({ "response_message": message }),
            )
            .await
    }
    pub async fn reject(&self, id: &str, message: Option<&str>) -> Result<Friendship> {
        self.parent
            .relay_post(
                &format!("/v1/friendships/{}/reject", urlencode(id)),
                &json!({ "response_message": message }),
            )
            .await
    }
    pub async fn counter(&self, id: &str, message: &str) -> Result<Friendship> {
        self.parent
            .relay_post(
                &format!("/v1/friendships/{}/counter", urlencode(id)),
                &json!({ "proposer_message": message }),
            )
            .await
    }
    pub async fn cancel(&self, id: &str) -> Result<Friendship> {
        self.parent
            .relay_post(
                &format!("/v1/friendships/{}/cancel", urlencode(id)),
                &json!({}),
            )
            .await
    }
}

pub struct GrantsClient<'a> {
    parent: &'a ChakraMCP,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ListGrantsOpts<'a> {
    pub direction: Option<&'a str>,
    pub status: Option<GrantStatus>,
}

impl<'a> GrantsClient<'a> {
    pub(crate) fn new(parent: &'a ChakraMCP) -> Self {
        Self { parent }
    }

    pub async fn list(&self, opts: ListGrantsOpts<'_>) -> Result<Vec<Grant>> {
        let mut q = Vec::new();
        if let Some(d) = opts.direction {
            q.push(format!("direction={}", urlencode(d)));
        }
        if let Some(s) = opts.status {
            q.push(format!("status={}", serde_plain(&s)));
        }
        let qs = if q.is_empty() {
            String::new()
        } else {
            format!("?{}", q.join("&"))
        };
        self.parent
            .relay_get(&format!("/v1/grants{qs}"))
            .await
    }
    pub async fn get(&self, id: &str) -> Result<Grant> {
        self.parent
            .relay_get(&format!("/v1/grants/{}", urlencode(id)))
            .await
    }
    pub async fn create(&self, body: &CreateGrantRequest) -> Result<Grant> {
        self.parent.relay_post("/v1/grants", body).await
    }
    pub async fn revoke(&self, id: &str, reason: Option<&str>) -> Result<Grant> {
        self.parent
            .relay_post(
                &format!("/v1/grants/{}/revoke", urlencode(id)),
                &json!({ "reason": reason }),
            )
            .await
    }
}

pub struct InvocationsClient<'a> {
    parent: &'a ChakraMCP,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ListInvocationsOpts<'a> {
    pub direction: Option<&'a str>,
    pub agent_id: Option<&'a str>,
    pub status: Option<InvocationStatus>,
}

impl<'a> InvocationsClient<'a> {
    pub(crate) fn new(parent: &'a ChakraMCP) -> Self {
        Self { parent }
    }

    pub async fn list(&self, opts: ListInvocationsOpts<'_>) -> Result<Vec<Invocation>> {
        let mut q = Vec::new();
        if let Some(d) = opts.direction {
            q.push(format!("direction={}", urlencode(d)));
        }
        if let Some(a) = opts.agent_id {
            q.push(format!("agent_id={}", urlencode(a)));
        }
        if let Some(s) = opts.status {
            q.push(format!("status={}", serde_plain(&s)));
        }
        let qs = if q.is_empty() {
            String::new()
        } else {
            format!("?{}", q.join("&"))
        };
        self.parent
            .relay_get(&format!("/v1/invocations{qs}"))
            .await
    }
    pub async fn get(&self, id: &str) -> Result<Invocation> {
        self.parent
            .relay_get(&format!("/v1/invocations/{}", urlencode(id)))
            .await
    }
}

// ─── Tiny utilities ──────────────────────────────────────

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// Serialize a tiny enum (e.g. FriendshipStatus) to its serde plain
/// rendering, without bringing in the serde_plain crate. We just JSON-
/// serialize the enum and strip the surrounding quotes.
fn serde_plain<T: Serialize>(v: &T) -> String {
    let mut s = serde_json::to_string(v).unwrap_or_default();
    if s.starts_with('"') && s.ends_with('"') {
        s = s[1..s.len() - 1].to_string();
    }
    s
}
