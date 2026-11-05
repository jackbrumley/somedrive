use super::*;

// Exclude rules (Settings → Sync): paths that are never synced in either
// direction. A rule with a `/` is a path from the top of the sync folder
// (that item and everything under it); a rule without one matches a file or
// folder name anywhere (`node_modules`, `*.tmp`).

pub(crate) const ISSUE_EXCLUDED_BY_RULE: &str = "excluded_by_rule";

/// The compiled exclude rules for one cycle.
pub(crate) struct ExclusionRules {
    pub(crate) matcher: globset::GlobSet,
    pub(crate) is_empty: bool,
    /// The rules as written, so a change to them can be noticed.
    pub(crate) patterns: Vec<String>,
}

impl ExclusionRules {
    pub(crate) fn compile(patterns: &[String]) -> Result<Self, String> {
        let mut builder = globset::GlobSetBuilder::new();
        let mut count = 0;
        for pattern in patterns {
            let trimmed = pattern.trim().trim_matches('/');
            if trimmed.is_empty() {
                continue;
            }
            let expansions = if trimmed.contains('/') {
                vec![trimmed.to_string(), format!("{trimmed}/**")]
            } else {
                vec![
                    trimmed.to_string(),
                    format!("**/{trimmed}"),
                    format!("{trimmed}/**"),
                    format!("**/{trimmed}/**"),
                ]
            };
            for expansion in expansions {
                let glob = globset::GlobBuilder::new(&expansion)
                    .literal_separator(true)
                    .build()
                    .map_err(|error| format!("Exclude rule '{pattern}' is not valid: {error}"))?;
                builder.add(glob);
            }
            count += 1;
        }
        let matcher = builder
            .build()
            .map_err(|error| format!("Failed compiling exclude rules: {error}"))?;
        Ok(Self {
            matcher,
            is_empty: count == 0,
            patterns: patterns.to_vec(),
        })
    }

    pub(crate) fn load() -> Result<Self, String> {
        Self::compile(&crate::app::app_settings::load_exclude_patterns()?)
    }

    pub(crate) fn excludes(&self, relative_path: &str) -> bool {
        !self.is_empty && !relative_path.is_empty() && self.matcher.is_match(relative_path)
    }
}

/// Checks exclude rules before they are saved.
pub fn validate_exclude_patterns(patterns: &[String]) -> Result<(), String> {
    ExclusionRules::compile(patterns).map(|_| ())
}

/// Marks every tracked path the rules exclude (so nothing acts on it) and
/// clears the mark from paths no rule excludes any more.
pub(crate) fn apply_exclusion_rules(
    profile_id: &str,
    rules: &ExclusionRules,
) -> Result<usize, String> {
    let mut connection = open_sync_jobs_connection(profile_id)?;
    let rows: Vec<(String, Option<String>)> = {
        let mut statement = connection
            .prepare("SELECT path, issue_code FROM sync_files WHERE profile_id = ?1 AND path != ''")
            .map_err(|error| format!("Failed preparing exclusion query: {error}"))?;
        let rows = statement
            .query_map(params![profile_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|error| format!("Failed querying paths for exclusion: {error}"))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| format!("Failed reading paths for exclusion: {error}"))?
    };
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| format!("Failed starting exclusion transaction: {error}"))?;
    let now = current_unix_seconds();
    let mut excluded = 0;
    for (path, issue_code) in rows {
        let is_marked = issue_code.as_deref() == Some(ISSUE_EXCLUDED_BY_RULE);
        let should_mark = rules.excludes(&path);
        if should_mark {
            excluded += 1;
        }
        if should_mark && issue_code.is_none() {
            transaction
                .execute(
                    "UPDATE sync_files SET issue_code = ?3, issue_detail = ?4, desired_action = 'none', updated_at = ?5
                     WHERE profile_id = ?1 AND path = ?2",
                    params![profile_id, path, ISSUE_EXCLUDED_BY_RULE, "Excluded from sync by a rule in Settings.", now],
                )
                .map_err(|error| format!("Failed excluding '{path}': {error}"))?;
        } else if !should_mark && is_marked {
            transaction
                .execute(
                    "UPDATE sync_files SET issue_code = NULL, issue_detail = NULL, updated_at = ?3
                     WHERE profile_id = ?1 AND path = ?2",
                    params![profile_id, path, now],
                )
                .map_err(|error| format!("Failed including '{path}' again: {error}"))?;
        }
    }
    transaction
        .commit()
        .map_err(|error| format!("Failed committing exclusions: {error}"))?;
    Ok(excluded)
}

#[cfg(test)]
mod exclusion_rules_tests {
    use super::*;

    fn rules(patterns: &[&str]) -> ExclusionRules {
        ExclusionRules::compile(
            &patterns
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>(),
        )
        .expect("compile")
    }

    #[test]
    fn a_path_rule_excludes_that_item_and_everything_under_it() {
        let rules = rules(&["Documents/Chromium"]);
        assert!(rules.excludes("Documents/Chromium"));
        assert!(rules.excludes("Documents/Chromium/Backup/User Data/Cookies"));
        assert!(!rules.excludes("Documents/Chromium Notes.txt"));
        assert!(!rules.excludes("Other/Documents/Chromium"));
    }

    #[test]
    fn a_name_rule_matches_anywhere() {
        let rules = rules(&["node_modules", "*.tmp"]);
        assert!(rules.excludes("node_modules"));
        assert!(rules.excludes("Code/app/node_modules/react/index.js"));
        assert!(rules.excludes("Downloads/file.tmp"));
        assert!(!rules.excludes("Downloads/file.tmpl"));
        assert!(!rules.excludes("Code/app/src/index.js"));
    }

    #[test]
    fn no_rules_exclude_nothing_and_bad_rules_are_rejected() {
        assert!(!rules(&[]).excludes("anything"));
        assert!(!rules(&["  ", "/"]).excludes("anything"));
        assert!(ExclusionRules::compile(&["Docs/[".to_string()]).is_err());
    }
}
