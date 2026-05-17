//! Reserved capability templates — Rust-side mirror of the Python +
//! TS SDK template registries. Schemas are identical across the
//! three implementations; if you bump one you bump the others.
//!
//! These ship hardcoded into the CLI binary rather than fetched from
//! the relay at runtime — they're tiny, stable, and we don't want a
//! network round-trip on `chakramcp capabilities add --template`.

use serde_json::{json, Value};

/// Lookup the body for `POST /v1/agents/{id}/capabilities` for a
/// reserved template id (e.g. `"message_owner"`). Returns `None`
/// when the id is unknown — caller surfaces a friendly error.
pub fn get_template(id: &str) -> Option<Value> {
    match id {
        "message_owner" => Some(message_owner()),
        _ => None,
    }
}

/// Sorted list of every known template id. Used by the CLI's
/// "unknown template" error to suggest valid names.
pub fn known_templates() -> Vec<&'static str> {
    vec!["message_owner"]
}

/// `message_owner` is the protocol's canonical "ask the human owner"
/// capability. Issue #69 PR 2 flips its `semantics` field to
/// `human_in_loop` — the relay's gate at
/// `POST /v1/invocations/{id}/result` will reject any result for a
/// `message_owner` invocation unless the caller sets
/// `confirmed_by_human: true`. The CLI's `inbox respond` and
/// `message reply` sugar set the flag automatically; SDK callers must
/// route to a human or opt in explicitly.
fn message_owner() -> Value {
    json!({
        "name": "message_owner",
        "description":
            "Ping the human owner of this agent. Friend agents call this; \
             the owner sees the message and explicitly answers, acks, \
             ignores, or defers. Always human-in-the-loop.",
        "semantics": "human_in_loop",
        "input_schema": {
            "type": "object",
            "required": ["message"],
            "properties": {
                "message": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 4000,
                    "description": "Message body. Plain text. Be concise.",
                },
                "from_display_name": {
                    "type": "string",
                    "maxLength": 120,
                    "description": "Who's pinging. Falls back to calling agent's display_name.",
                },
                "urgency": {
                    "type": "string",
                    "enum": ["low", "normal", "high"],
                    "default": "normal",
                    "description": "low = batch in next digest; normal = surface when owner next interacts; high = alert immediately.",
                },
                "expects_reply": {
                    "type": "boolean",
                    "default": true,
                    "description": "If false, message is informational and an ack is enough.",
                },
                "reply_by": {
                    "type": "string",
                    "format": "date-time",
                    "description": "Optional soft deadline. After this, handler may auto-defer.",
                },
            },
        },
        "output_schema": {
            "type": "object",
            "required": ["status"],
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["replied", "acknowledged", "ignored", "deferred"],
                },
                "reply": {
                    "type": "string",
                    "maxLength": 8000,
                    "description": "Owner's reply text. Present only when status='replied'.",
                },
                "responded_at": {
                    "type": "string",
                    "format": "date-time",
                },
                "defer_until": {
                    "type": "string",
                    "format": "date-time",
                    "description": "Present only when status='deferred'.",
                },
            },
        },
        "visibility": "network",
    })
}
