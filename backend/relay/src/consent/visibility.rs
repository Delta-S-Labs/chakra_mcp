//! Hiding held input (spec §4.3).
//!
//! Until an approver approves a held request, nobody in the target agent's
//! account sees its input through the invocation reads (REST list and get,
//! MCP list and poll), and never if it's denied, expires or is cancelled.
//! The rule keys on membership of the target's account (`i_served`), even
//! when the reader is on the requesting side too, so one owner of both
//! agents can't read it through `direction=outbound`. Approvers see the
//! input on the approval page.

use serde_json::Value;

/// What a held input reads as.
pub const HELD_MARKER: &str = "__chakramcp_held__";

/// The input a reader is shown. `request_status` is the invocation's gate-1
/// consent request status, `None` when the capability didn't ask first.
pub fn shown_input(
    i_served: bool,
    request_status: Option<&str>,
    input: Option<Value>,
) -> Option<Value> {
    match request_status {
        Some(status) if i_served && status != "approved" => {
            Some(serde_json::json!({ HELD_MARKER: true }))
        }
        _ => input,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hides_unapproved_input_from_the_target_side_only() {
        let input = Some(json!({"pay": 100}));
        let held = Some(json!({ HELD_MARKER: true }));
        for status in ["pending", "denied", "expired", "cancelled"] {
            assert_eq!(
                shown_input(true, Some(status), input.clone()),
                held,
                "{status}"
            );
            assert_eq!(
                shown_input(false, Some(status), input.clone()),
                input,
                "{status}"
            );
        }
        assert_eq!(shown_input(true, Some("approved"), input.clone()), input);
        assert_eq!(shown_input(true, None, input.clone()), input);
        assert_eq!(shown_input(false, None, input.clone()), input);
    }
}
