//! A capability's consent settings (spec §3): whether it asks its owner
//! before requests reach the agent, who decides, the longest consent an
//! approver may grant, and how long a held request waits. Set on publish
//! and update, over REST and MCP.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use chakramcp_shared::consent::terms::Validity;
use chakramcp_shared::error::ApiError;

/// The longest consents a publisher may allow.
pub const MAX_VALIDITY_CHOICES: [&str; 5] = ["once", "1h", "1d", "7d", "30d"];
/// How long a held request may wait, in seconds: 5 minutes to 7 days.
pub const WAIT_MIN_SECS: i32 = 5 * 60;
pub const WAIT_MAX_SECS: i32 = 7 * 24 * 60 * 60;
/// Most approvers one capability may list (the column's CHECK).
pub const MAX_APPROVERS: usize = 20;

/// `consent` on publish and update. Absent fields keep their value (on
/// update) or take the default (on publish).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ConsentInput {
    pub required: Option<bool>,
    /// User ids or emails of members of the agent's account. Empty means
    /// the account's owners and admins.
    pub approvers: Option<Vec<String>>,
    /// `once`, `1h`, `1d`, `7d` or `30d`.
    pub max_validity: Option<String>,
    /// `<n>m`, `<n>h` or `<n>d`, from 5 minutes to 7 days.
    pub wait: Option<String>,
}

/// The stored settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentSettings {
    pub required: bool,
    pub approvers: Vec<Uuid>,
    pub max_validity_secs: i32,
    pub wait_secs: i32,
}

impl Default for ConsentSettings {
    fn default() -> Self {
        Self {
            required: false,
            approvers: vec![],
            max_validity_secs: 0,
            wait_secs: 24 * 60 * 60,
        }
    }
}

/// `consent` on a capability. Callers who can't manage the agent see
/// whether it asks first and its limits, not who decides.
#[derive(Debug, Clone, Serialize)]
pub struct ConsentDto {
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approvers: Option<Vec<Uuid>>,
    pub max_validity: String,
    pub wait: String,
}

impl ConsentSettings {
    pub fn to_dto(&self, manageable: bool) -> ConsentDto {
        ConsentDto {
            required: self.required,
            approvers: manageable.then(|| self.approvers.clone()),
            max_validity: Validity::from_secs(self.max_validity_secs).to_string(),
            wait: format_wait(self.wait_secs),
        }
    }

    /// `current` changed by `input`: absent fields keep their value.
    /// Approvers are resolved against the members of `account_id`.
    pub async fn apply(
        mut self,
        db: &PgPool,
        account_id: Uuid,
        input: &ConsentInput,
    ) -> Result<Self, ApiError> {
        if let Some(required) = input.required {
            self.required = required;
        }
        if let Some(raw) = &input.max_validity {
            self.max_validity_secs = parse_max_validity(raw).map_err(ApiError::InvalidRequest)?;
        }
        if let Some(raw) = &input.wait {
            self.wait_secs = parse_wait(raw).map_err(ApiError::InvalidRequest)?;
        }
        if let Some(approvers) = &input.approvers {
            self.approvers = resolve_approvers(db, account_id, approvers).await?;
        }
        Ok(self)
    }
}

pub fn parse_max_validity(raw: &str) -> Result<i32, String> {
    match raw.trim() {
        "once" => Ok(0),
        "1h" => Ok(3600),
        "1d" => Ok(86_400),
        "7d" => Ok(604_800),
        "30d" => Ok(2_592_000),
        other => Err(format!(
            "consent.max_validity must be one of {}, not {other:?}",
            MAX_VALIDITY_CHOICES.join(", ")
        )),
    }
}

/// `<n>m`, `<n>h` or `<n>d` → seconds, within 5 minutes to 7 days.
pub fn parse_wait(raw: &str) -> Result<i32, String> {
    let raw = raw.trim();
    let bad = || format!("consent.wait must be <n>m, <n>h or <n>d, from 5m to 7d, not {raw:?}");
    let (num, unit) = raw.split_at(raw.len().saturating_sub(1));
    let n: i32 = num.parse().map_err(|_| bad())?;
    let secs = match unit {
        "m" => n.checked_mul(60),
        "h" => n.checked_mul(3600),
        "d" => n.checked_mul(86_400),
        _ => None,
    }
    .ok_or_else(bad)?;
    if (WAIT_MIN_SECS..=WAIT_MAX_SECS).contains(&secs) {
        Ok(secs)
    } else {
        Err(bad())
    }
}

/// Seconds → the shortest exact `<n>d`, `<n>h` or `<n>m`.
pub fn format_wait(secs: i32) -> String {
    if secs % 86_400 == 0 {
        format!("{}d", secs / 86_400)
    } else if secs % 3600 == 0 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}m", secs / 60)
    }
}

/// Each entry, a user id or an email, must be a member of `account_id`.
async fn resolve_approvers(
    db: &PgPool,
    account_id: Uuid,
    entries: &[String],
) -> Result<Vec<Uuid>, ApiError> {
    let mut ids: Vec<Uuid> = Vec::new();
    for entry in entries.iter().map(|e| e.trim()).filter(|e| !e.is_empty()) {
        let id = sqlx::query_scalar!(
            r#"
            SELECT u.id
              FROM account_memberships m
              JOIN users u ON u.id = m.user_id
             WHERE m.account_id = $1
               AND (u.id::text = $2 OR lower(u.email) = lower($2))
            "#,
            account_id,
            entry,
        )
        .fetch_optional(db)
        .await?
        .ok_or_else(|| {
            ApiError::InvalidRequest(format!(
                "consent.approvers: {entry} isn't a member of this agent's account"
            ))
        })?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.len() > MAX_APPROVERS {
        return Err(ApiError::InvalidRequest(format!(
            "consent.approvers: at most {MAX_APPROVERS}"
        )));
    }
    Ok(ids)
}

/// The relay forwards calls to a push agent at once, so it can't hold them
/// (spec §11).
pub fn refuse_on_push(settings: &ConsentSettings, agent_mode: &str) -> Result<(), ApiError> {
    if settings.required && agent_mode == "push" {
        return Err(ApiError::Conflict(
            "this agent takes calls at its own endpoint (push mode), so the relay can't hold \
             them for approval; consent.required needs a pull-mode agent"
                .into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_parse_and_print() {
        assert_eq!(parse_max_validity("7d"), Ok(604_800));
        assert_eq!(parse_max_validity("once"), Ok(0));
        assert!(parse_max_validity("2d").is_err());
        assert_eq!(parse_wait("5m"), Ok(300));
        assert_eq!(parse_wait("24h"), Ok(86_400));
        assert_eq!(parse_wait("7d"), Ok(604_800));
        for bad in ["4m", "8d", "1w", "", "h", "-1h"] {
            assert!(parse_wait(bad).is_err(), "{bad}");
        }
        assert_eq!(format_wait(86_400), "1d");
        assert_eq!(format_wait(7200), "2h");
        assert_eq!(format_wait(300), "5m");
    }

    #[test]
    fn dto_hides_approvers_from_outsiders() {
        let s = ConsentSettings {
            required: true,
            approvers: vec![Uuid::nil()],
            max_validity_secs: 604_800,
            wait_secs: 86_400,
        };
        let outside = serde_json::to_value(s.to_dto(false)).unwrap();
        assert!(outside.get("approvers").is_none());
        assert_eq!(outside["max_validity"], "7d");
        assert_eq!(outside["wait"], "1d");
        assert_eq!(
            serde_json::to_value(s.to_dto(true)).unwrap()["approvers"],
            serde_json::json!([Uuid::nil()])
        );
    }
}
