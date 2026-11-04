use super::*;

// Items SomeDrive leaves alone by design, shown in the UI's "Not synced"
// tab with the reason, instead of as problems the user cannot solve.

/// Issue codes that mean "skipped by design", not "needs you".
pub(crate) const NOT_SYNCED_ISSUE_CODES: [&str; 3] = [
    ISSUE_ONENOTE_NOTEBOOK,
    ISSUE_EXCLUDED_BY_RULE,
    ISSUE_MALWARE_DETECTED,
];
/// The reason code for a folder shared with this account by someone else.
pub(crate) const NOT_SYNCED_SHARED: &str = "shared_with_you";

/// One line of the "Not synced" tab: a skipped file or folder (with the
/// skipped items inside it counted, not listed) and why it is skipped.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NotSyncedItem {
    pub path: String,
    pub reason_code: String,
    pub reason: String,
    pub is_folder: bool,
    /// Skipped items inside this one, covered by this line.
    pub contained: usize,
}

/// The account's skipped-by-design items, one line per top-most item.
pub(crate) fn list_not_synced_items(profile_id: &str) -> Result<Vec<NotSyncedItem>, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let mut statement = connection
        .prepare(
            "SELECT path, is_dir, is_shared_reference, COALESCE(issue_code, ''),
                    COALESCE(issue_detail, '')
             FROM sync_files
             WHERE profile_id = ?1
               AND (issue_code IN (?2, ?3, ?4) OR is_shared_reference = 1)
             ORDER BY path",
        )
        .map_err(|error| format!("Failed preparing not-synced query: {error}"))?;
    let rows = statement
        .query_map(
            params![
                profile_id,
                NOT_SYNCED_ISSUE_CODES[0],
                NOT_SYNCED_ISSUE_CODES[1],
                NOT_SYNCED_ISSUE_CODES[2],
            ],
            |row| {
                let shared = sql_to_bool(row.get(2)?);
                let code: String = row.get(3)?;
                let reason_code = if shared {
                    NOT_SYNCED_SHARED.to_string()
                } else {
                    code
                };
                let detail: String = row.get(4)?;
                Ok(NotSyncedItem {
                    path: row.get(0)?,
                    reason: not_synced_reason(&reason_code, &detail),
                    reason_code,
                    is_folder: sql_to_bool(row.get(1)?) || shared,
                    contained: 0,
                })
            },
        )
        .map_err(|error| format!("Failed querying not-synced items: {error}"))?;
    let items = rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("Failed reading not-synced items: {error}"))?;
    Ok(group_under_top_most(items))
}

/// Folds items inside an already-listed item into its `contained` count.
/// `items` must be sorted by path.
pub(crate) fn group_under_top_most(items: Vec<NotSyncedItem>) -> Vec<NotSyncedItem> {
    let mut grouped: Vec<NotSyncedItem> = Vec::new();
    for item in items {
        if let Some(parent) = grouped.last_mut() {
            if item.path.starts_with(&format!("{}/", parent.path)) {
                parent.contained += 1;
                continue;
            }
        }
        grouped.push(item);
    }
    grouped
}

/// The plain-language reason shown for each kind of skipped item.
fn not_synced_reason(reason_code: &str, detail: &str) -> String {
    match reason_code {
        ISSUE_ONENOTE_NOTEBOOK => {
            "OneNote notebook. OneNote keeps notebooks online; open it in OneNote or at onenote.com.".to_string()
        }
        ISSUE_EXCLUDED_BY_RULE => {
            if detail.is_empty() {
                "Excluded by your rules in Settings → Excluded from sync.".to_string()
            } else {
                format!("Excluded by your rules in Settings → Excluded from sync ({detail}).")
            }
        }
        ISSUE_MALWARE_DETECTED => {
            "OneDrive flagged this file as malware, so it cannot be downloaded.".to_string()
        }
        NOT_SYNCED_SHARED => {
            "Shared with you by someone else. Shared folders are not synced to this device.".to_string()
        }
        _ => detail.to_string(),
    }
}

#[cfg(test)]
mod not_synced_tests {
    use super::*;

    fn item(path: &str, code: &str) -> NotSyncedItem {
        NotSyncedItem {
            path: path.to_string(),
            reason_code: code.to_string(),
            reason: not_synced_reason(code, ""),
            is_folder: false,
            contained: 0,
        }
    }

    #[test]
    fn a_notebook_and_its_sections_are_one_line() {
        let grouped = group_under_top_most(vec![
            item("Documents/Family Notebook", ISSUE_ONENOTE_NOTEBOOK),
            item("Documents/Family Notebook/Food.one", ISSUE_ONENOTE_NOTEBOOK),
            item(
                "Documents/Family Notebook/Health.one",
                ISSUE_ONENOTE_NOTEBOOK,
            ),
            item("Documents/Family Notebook2", ISSUE_EXCLUDED_BY_RULE),
            item("Tayla's Photos", NOT_SYNCED_SHARED),
        ]);
        let summary: Vec<(&str, usize)> = grouped
            .iter()
            .map(|item| (item.path.as_str(), item.contained))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("Documents/Family Notebook", 2),
                ("Documents/Family Notebook2", 0),
                ("Tayla's Photos", 0),
            ],
            "a sibling with a longer name is not inside the notebook"
        );
        assert!(grouped[0].reason.contains("OneNote"));
        assert!(grouped[2].reason.contains("Shared with you"));
    }
}
