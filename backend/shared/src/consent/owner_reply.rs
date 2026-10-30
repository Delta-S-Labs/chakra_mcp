//! What the owner's reply page shows (waking pull agents §9): a call to a
//! `human_in_loop` capability, for one of the called agent's approvers, and
//! how the reply box turns into the call's output. The self-hosted built-in
//! page uses this; chakramcp.com's web app follows the same rules.
//! Answering is [`super::reply::answer_as_owner`].

use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiError;

/// A call as its owner sees it.
#[derive(Debug, Clone)]
pub struct MessageView {
    pub invocation_id: Uuid,
    /// `pending`, `in_progress`, or how it ended.
    pub status: String,
    pub capability_name: String,
    pub capability_description: Option<String>,
    pub output_schema: Value,
    /// The called agent.
    pub target: Option<String>,
    pub requester: Option<String>,
    pub requester_account: Option<String>,
    pub requester_verified: bool,
    pub input: Option<Value>,
    pub output: Option<Value>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    /// The agent's reply draft waiting for an approver, if there is one: it's
    /// decided on its approval page instead.
    pub draft_request_id: Option<Uuid>,
}

impl MessageView {
    /// Whether the owner can answer it here: open, with no draft waiting.
    pub fn answerable(&self) -> bool {
        matches!(self.status.as_str(), "pending" | "in_progress") && self.draft_request_id.is_none()
    }
}

/// Load call `invocation_id` for `user_id`, who must be one of the called
/// agent's approvers (`approvers::may_decide`), and the capability
/// `human_in_loop`. Anyone or anything else is `NotFound`.
pub async fn load(
    db: &PgPool,
    user_id: Uuid,
    invocation_id: Uuid,
) -> Result<MessageView, ApiError> {
    let r = sqlx::query!(
        r#"
        SELECT i.id, i.status, i.capability_name, i.input_preview, i.output_preview,
               i.error_message, i.created_at, i.expires_at,
               ga.account_id AS "granter_account_id?", ga.display_name AS "target?",
               ea.display_name AS "requester?",
               eacc.display_name AS "requester_account?",
               (eacc.verified_at IS NOT NULL) AS "requester_verified?",
               c.semantics AS "semantics?", c.description AS "capability_description?",
               c.output_schema AS "output_schema?", c.consent_approvers AS "approvers?",
               (SELECT cr.id FROM consent_requests cr
                 WHERE cr.invocation_id = i.id AND cr.kind = 'reply' AND cr.status = 'pending')
                   AS "draft_request_id?"
          FROM relay_invocations i
          LEFT JOIN agents ga ON ga.id = i.granter_agent_id
          LEFT JOIN agents ea ON ea.id = i.grantee_agent_id
          LEFT JOIN accounts eacc ON eacc.id = ea.account_id
          LEFT JOIN agent_capabilities c ON c.id = i.capability_id
         WHERE i.id = $1
        "#,
        invocation_id,
    )
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)?;
    let account = r.granter_account_id.ok_or(ApiError::NotFound)?;
    if r.semantics.as_deref() != Some("human_in_loop")
        || !super::approvers::may_decide(
            db,
            user_id,
            account,
            r.approvers.as_deref().unwrap_or_default(),
        )
        .await?
    {
        return Err(ApiError::NotFound);
    }
    Ok(MessageView {
        invocation_id: r.id,
        status: r.status,
        capability_name: r.capability_name,
        capability_description: r.capability_description.filter(|d| !d.trim().is_empty()),
        output_schema: r.output_schema.unwrap_or(Value::Null),
        target: r.target,
        requester: r.requester,
        requester_account: r.requester_account,
        requester_verified: r.requester_verified.unwrap_or(false),
        input: r.input_preview,
        output: r.output_preview,
        error_message: r.error_message,
        created_at: r.created_at,
        expires_at: r.expires_at,
        draft_request_id: r.draft_request_id,
    })
}

/// How the reply box becomes the call's output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyShape {
    /// No output schema: the output is the text.
    Text,
    /// `message_owner`'s shape: `{status: "replied", reply, responded_at}`,
    /// so its owners type a reply, not JSON.
    Replied,
    /// Any other output schema: the box holds the output as JSON.
    Json,
}

impl ReplyShape {
    /// The shape for a capability with `output_schema`.
    pub fn of(output_schema: &Value) -> Self {
        let Some(schema) = output_schema.as_object().filter(|s| !s.is_empty()) else {
            return ReplyShape::Text;
        };
        let props = schema.get("properties");
        let reply_is_text = props
            .and_then(|p| p.get("reply"))
            .and_then(|r| r.get("type"))
            .is_some_and(|t| t == "string");
        let can_say_replied = props
            .and_then(|p| p.get("status"))
            .and_then(|s| s.get("enum"))
            .and_then(Value::as_array)
            .is_some_and(|e| e.iter().any(|v| v == "replied"));
        if reply_is_text && can_say_replied {
            ReplyShape::Replied
        } else {
            ReplyShape::Json
        }
    }

    /// The call's output from what the owner typed. A blank reply, or JSON
    /// that doesn't parse, is refused with a sentence to show them.
    pub fn output(self, typed: &str, now: DateTime<Utc>) -> Result<Value, String> {
        let typed = typed.trim();
        if typed.is_empty() {
            return Err("Write a reply first.".into());
        }
        match self {
            ReplyShape::Text => Ok(Value::String(typed.to_owned())),
            ReplyShape::Replied => Ok(json!({
                "status": "replied",
                "reply": typed,
                "responded_at": now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            })),
            ReplyShape::Json => serde_json::from_str(typed)
                .map_err(|e| format!("The reply must be JSON for this capability: {e}.")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reply_shape_follows_the_output_schema() {
        assert_eq!(ReplyShape::of(&Value::Null), ReplyShape::Text);
        assert_eq!(ReplyShape::of(&json!({})), ReplyShape::Text);
        let message_owner = json!({
            "type": "object",
            "properties": {
                "status": { "type": "string", "enum": ["replied", "acknowledged"] },
                "reply": { "type": "string" }
            }
        });
        assert_eq!(ReplyShape::of(&message_owner), ReplyShape::Replied);
        assert_eq!(
            ReplyShape::of(
                &json!({ "type": "object", "properties": { "slots": { "type": "array" } } })
            ),
            ReplyShape::Json
        );
        assert_eq!(
            ReplyShape::of(&json!({ "type": "object" })),
            ReplyShape::Json
        );
    }

    #[test]
    fn typed_replies_become_outputs() {
        let now: DateTime<Utc> = "2026-10-30T09:00:00Z".parse().unwrap();
        assert_eq!(
            ReplyShape::Text.output(" See you at 3 ", now).unwrap(),
            json!("See you at 3")
        );
        assert_eq!(
            ReplyShape::Replied.output("See you at 3", now).unwrap(),
            json!({ "status": "replied", "reply": "See you at 3", "responded_at": "2026-10-30T09:00:00Z" })
        );
        assert_eq!(
            ReplyShape::Json.output(r#"{"slots":[1]}"#, now).unwrap(),
            json!({ "slots": [1] })
        );
        assert!(ReplyShape::Json.output("{nope", now).is_err());
        assert!(ReplyShape::Text.output("   ", now).is_err());
    }
}
