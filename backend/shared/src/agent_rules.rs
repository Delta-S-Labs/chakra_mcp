//! Rules for an agent's directory listing (MCP and discovery spec §5.6–5.7),
//! applied everywhere agents are made or edited: the relay's REST and MCP
//! paths and the app's device pairing.

/// At most this many tags per agent.
pub const MAX_TAGS: usize = 10;
/// Each tag is 1 to this many characters.
pub const MAX_TAG_LEN: usize = 32;
/// A network-visible agent's description is at least this long, so people
/// browsing the directory can tell what it does.
pub const MIN_NETWORK_DESCRIPTION: usize = 20;

/// Tags as stored: trimmed, lowercased, duplicates dropped (first kept), at
/// most [`MAX_TAGS`], each 1–[`MAX_TAG_LEN`] characters of lowercase letters,
/// digits and hyphens. Empty entries are dropped. The error says what's
/// wrong, for a 400.
pub fn normalize_tags(tags: &[String]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for raw in tags {
        let tag = raw.trim().to_lowercase();
        if tag.is_empty() || out.contains(&tag) {
            continue;
        }
        if tag.chars().count() > MAX_TAG_LEN
            || !tag
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(format!(
                "tag \"{tag}\" must be 1-{MAX_TAG_LEN} lowercase letters, digits or hyphens"
            ));
        }
        out.push(tag);
    }
    if out.len() > MAX_TAGS {
        return Err(format!("an agent has at most {MAX_TAGS} tags"));
    }
    Ok(out)
}

/// A network-visible agent needs a description of at least
/// [`MIN_NETWORK_DESCRIPTION`] characters. Other visibilities need none.
pub fn check_description(visibility: &str, description: &str) -> Result<(), String> {
    if visibility == "network" && description.trim().chars().count() < MIN_NETWORK_DESCRIPTION {
        return Err(format!(
            "an agent anyone on the network can find needs a description of at least \
             {MIN_NETWORK_DESCRIPTION} characters saying what it does"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn tags_are_trimmed_lowercased_and_deduplicated() {
        assert_eq!(
            normalize_tags(&tags(&[" Travel ", "booking", "travel", "", "b2b"])).unwrap(),
            ["travel", "booking", "b2b"]
        );
    }

    #[test]
    fn tags_have_a_format_and_a_count() {
        assert!(normalize_tags(&tags(&["no spaces"])).is_err());
        assert!(normalize_tags(&tags(&["under_score"])).is_err());
        assert!(normalize_tags(&tags(&["émoji"])).is_err());
        assert!(normalize_tags(&tags(&[&"x".repeat(33)])).is_err());
        assert!(normalize_tags(&tags(&[&"x".repeat(32)])).is_ok());
        let eleven: Vec<String> = (0..11).map(|i| format!("t{i}")).collect();
        assert!(normalize_tags(&eleven).is_err());
        // Duplicates don't count toward the limit.
        let mut ten: Vec<String> = (0..10).map(|i| format!("t{i}")).collect();
        ten.push("t0".into());
        assert_eq!(normalize_tags(&ten).unwrap().len(), 10);
    }

    #[test]
    fn network_agents_need_a_description() {
        assert!(check_description("network", "Too short.").is_err());
        assert!(check_description("network", "   padded            ").is_err());
        assert!(check_description("network", "Books rooms for your team.").is_ok());
        assert!(check_description("private", "").is_ok());
        assert!(check_description("org", "").is_ok());
    }
}
