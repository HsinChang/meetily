/// Embedded default templates using compile-time inclusion
///
/// These templates are bundled into the binary and serve as fallbacks
/// when custom templates are not available.

/// Brief minutes: background + key points only (fits 1-2 pages)
pub const GOV_BRIEF: &str = include_str!("../../../templates/gov_brief.json");

/// Detailed minutes: attendees, background and the flow of the meeting
pub const GOV_DETAILED: &str = include_str!("../../../templates/gov_detailed.json");

/// Comprehensive report with analysis and recommendations, for supervisors
pub const GOV_REPORT: &str = include_str!("../../../templates/gov_report.json");

/// Registry of all built-in templates
///
/// Maps template identifiers to their embedded JSON content
pub fn get_builtin_templates() -> Vec<(&'static str, &'static str)> {
    vec![
        ("gov_brief", GOV_BRIEF),
        ("gov_detailed", GOV_DETAILED),
        ("gov_report", GOV_REPORT),
    ]
}

/// Get a built-in template by identifier
///
/// # Arguments
/// * `id` - Template identifier (e.g., "gov_brief", "gov_detailed", "gov_report")
///
/// # Returns
/// The template JSON content if found, None otherwise
pub fn get_builtin_template(id: &str) -> Option<&'static str> {
    match id {
        "gov_brief" => Some(GOV_BRIEF),
        "gov_detailed" => Some(GOV_DETAILED),
        "gov_report" => Some(GOV_REPORT),
        _ => None,
    }
}

/// List all built-in template identifiers
pub fn list_builtin_template_ids() -> Vec<&'static str> {
    vec!["gov_brief", "gov_detailed", "gov_report"]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_templates_valid_json() {
        for (id, content) in get_builtin_templates() {
            let result = serde_json::from_str::<serde_json::Value>(content);
            assert!(
                result.is_ok(),
                "Built-in template '{}' contains invalid JSON: {:?}",
                id,
                result.err()
            );
        }
    }

    #[test]
    fn test_get_builtin_template() {
        assert!(get_builtin_template("gov_brief").is_some());
        assert!(get_builtin_template("gov_detailed").is_some());
        assert!(get_builtin_template("gov_report").is_some());
        assert!(get_builtin_template("nonexistent").is_none());
    }
}
