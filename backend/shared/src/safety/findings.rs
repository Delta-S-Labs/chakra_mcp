//! What screening reports (spec §5): findings, their kinds and actions, and
//! the `safety` object senders and receivers see.

use serde::{Deserialize, Serialize};

/// Which kind of message is being screened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// A call's input, on its way to the target agent.
    Request,
    /// A reply `{status, output?, error?}`, on its way back to the caller.
    Reply,
    /// A short text one party sends the other: a friendship message, a
    /// grant purpose, an approver's deny note.
    Note,
    /// A message on a call (calls spec §5), on its way to the other party.
    Message,
}

impl Channel {
    pub const ALL: [Channel; 4] = [
        Channel::Request,
        Channel::Reply,
        Channel::Note,
        Channel::Message,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Request => "request",
            Channel::Reply => "reply",
            Channel::Note => "note",
            Channel::Message => "message",
        }
    }
}

/// What the relay did about a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Redacted,
    Flagged,
    Blocked,
}

impl Action {
    pub const ALL: [Action; 3] = [Action::Redacted, Action::Flagged, Action::Blocked];

    pub fn as_str(self) -> &'static str {
        match self {
            Action::Redacted => "redacted",
            Action::Flagged => "flagged",
            Action::Blocked => "blocked",
        }
    }
}

/// Every kind a finding can have. The contact kinds come from the code
/// pass; the question kinds are Jev question ids (S2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    Email,
    Phone,
    Url,
    MessagingLink,
    Handle,
    OffPlatform,
    Opaque,
    Unjudged,
    HarmViolence,
    HarmCrime,
    HarmHarassment,
    HarmMinors,
    HarmSelf,
    HarmMalware,
    PromptInjection,
    OffCapability,
    OffPurpose,
    OffRelationship,
    DataExfiltration,
}

impl FindingKind {
    /// Every kind, for registering metric series at 0.
    pub const ALL: [FindingKind; 19] = [
        FindingKind::Email,
        FindingKind::Phone,
        FindingKind::Url,
        FindingKind::MessagingLink,
        FindingKind::Handle,
        FindingKind::OffPlatform,
        FindingKind::Opaque,
        FindingKind::Unjudged,
        FindingKind::HarmViolence,
        FindingKind::HarmCrime,
        FindingKind::HarmHarassment,
        FindingKind::HarmMinors,
        FindingKind::HarmSelf,
        FindingKind::HarmMalware,
        FindingKind::PromptInjection,
        FindingKind::OffCapability,
        FindingKind::OffPurpose,
        FindingKind::OffRelationship,
        FindingKind::DataExfiltration,
    ];

    /// The kind for a Jev question id (`harm_crime`), if it has one.
    pub fn from_question(id: &str) -> Option<FindingKind> {
        FindingKind::ALL.into_iter().find(|k| k.as_str() == id)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            FindingKind::Email => "email",
            FindingKind::Phone => "phone",
            FindingKind::Url => "url",
            FindingKind::MessagingLink => "messaging_link",
            FindingKind::Handle => "handle",
            FindingKind::OffPlatform => "off_platform",
            FindingKind::Opaque => "opaque",
            FindingKind::Unjudged => "unjudged",
            FindingKind::HarmViolence => "harm_violence",
            FindingKind::HarmCrime => "harm_crime",
            FindingKind::HarmHarassment => "harm_harassment",
            FindingKind::HarmMinors => "harm_minors",
            FindingKind::HarmSelf => "harm_self",
            FindingKind::HarmMalware => "harm_malware",
            FindingKind::PromptInjection => "prompt_injection",
            FindingKind::OffCapability => "off_capability",
            FindingKind::OffPurpose => "off_purpose",
            FindingKind::OffRelationship => "off_relationship",
            FindingKind::DataExfiltration => "data_exfiltration",
        }
    }
}

/// One thing screening found, and what it did about it. Never holds a
/// removed value: only its kind and where it was.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub kind: FindingKind,
    pub action: Action,
    /// Where in the message, for findings about a span (`input.message`).
    /// Absent for findings about the whole message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The Jev question that judged it, when Jev did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    /// Jev's probability, when Jev judged it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p: Option<f64>,
}

impl Finding {
    /// A finding from the code pass alone.
    pub fn code(kind: FindingKind, action: Action, path: Option<String>) -> Self {
        Self {
            kind,
            action,
            path,
            question: None,
            p: None,
        }
    }
}

/// The `safety` object on invoke responses, replies and invocations.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Safety {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<Vec<Finding>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<Vec<Finding>>,
}

impl Safety {
    /// True when there's nothing to report, so the field can be omitted.
    pub fn is_empty(&self) -> bool {
        self.request.as_ref().is_none_or(Vec::is_empty)
            && self.reply.as_ref().is_none_or(Vec::is_empty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn kinds_serialize_as_their_names() {
        for kind in FindingKind::ALL {
            assert_eq!(serde_json::to_value(kind).unwrap(), json!(kind.as_str()));
        }
        for action in Action::ALL {
            assert_eq!(
                serde_json::to_value(action).unwrap(),
                json!(action.as_str())
            );
        }
        for channel in Channel::ALL {
            assert_eq!(
                serde_json::to_value(channel).unwrap(),
                json!(channel.as_str())
            );
        }
    }

    #[test]
    fn findings_omit_what_they_dont_have() {
        let f = Finding::code(
            FindingKind::Email,
            Action::Redacted,
            Some("input.message".into()),
        );
        assert_eq!(
            serde_json::to_value(&f).unwrap(),
            json!({"kind": "email", "action": "redacted", "path": "input.message"})
        );
        let s = Safety {
            request: Some(vec![f]),
            reply: None,
        };
        assert!(!s.is_empty());
        assert_eq!(
            serde_json::to_value(&s).unwrap().as_object().unwrap().len(),
            1
        );
        assert!(Safety::default().is_empty());
    }
}
