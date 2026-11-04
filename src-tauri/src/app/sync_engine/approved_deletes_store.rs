use super::*;

// Deletes the user asked for inside SomeDrive (the Clean up tab), after a
// typed confirmation. The large-delete guard lets these through without
// asking a second time and does not count them against its limit. An
// approval lasts a day and ends once its path is no longer tracked.

pub(crate) const APPROVED_DELETE_LIFETIME_SECONDS: i64 = 24 * 60 * 60;

/// Records that the user approved deleting `paths` (files and folders).
pub(crate) fn approve_deletes(profile_id: &str, paths: &[String]) -> Result<(), String> {
    let mut connection = open_sync_jobs_connection(profile_id)?;
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| format!("Failed starting delete approval transaction: {error}"))?;
    let now = current_unix_seconds();
    {
        let mut statement = transaction
            .prepare(
                "INSERT INTO sync_approved_deletes (profile_id, path, approved_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(profile_id, path) DO UPDATE SET approved_at = excluded.approved_at",
            )
            .map_err(|error| format!("Failed preparing delete approval: {error}"))?;
        for path in paths {
            statement
                .execute(params![profile_id, path, now])
                .map_err(|error| format!("Failed approving the delete of '{path}': {error}"))?;
        }
    }
    transaction
        .commit()
        .map_err(|error| format!("Failed committing delete approvals: {error}"))
}

/// Paths whose delete the user approved within the last day.
pub(crate) fn read_approved_deletes(profile_id: &str) -> Result<HashSet<String>, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    let mut statement = connection
        .prepare(
            "SELECT path FROM sync_approved_deletes
             WHERE profile_id = ?1 AND approved_at >= ?2",
        )
        .map_err(|error| format!("Failed preparing approved deletes query: {error}"))?;
    let rows = statement
        .query_map(
            params![
                profile_id,
                current_unix_seconds() - APPROVED_DELETE_LIFETIME_SECONDS
            ],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| format!("Failed querying approved deletes: {error}"))?;
    rows.collect::<rusqlite::Result<HashSet<_>>>()
        .map_err(|error| format!("Failed reading approved deletes: {error}"))
}

/// Ends approvals that expired or whose path is gone (deleted, or never
/// synced), so an approval never carries over to a file created later.
pub(crate) fn settle_approved_deletes(profile_id: &str) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .execute(
            "DELETE FROM sync_approved_deletes
             WHERE profile_id = ?1
               AND (approved_at < ?2 OR NOT EXISTS (
                    SELECT 1 FROM sync_files
                    WHERE sync_files.profile_id = sync_approved_deletes.profile_id
                      AND sync_files.path = sync_approved_deletes.path
                      AND sync_files.base_present = 1))",
            params![
                profile_id,
                current_unix_seconds() - APPROVED_DELETE_LIFETIME_SECONDS
            ],
        )
        .map_err(|error| format!("Failed settling approved deletes: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod approved_deletes_tests {
    use super::*;

    #[test]
    fn an_approval_ends_once_its_path_is_no_longer_tracked() {
        let profile_id = test_profile_id("approved-deletes");
        insert_row(
            &profile_id,
            "Old/a.txt",
            &RowSpec {
                cloud: Some((10, "hash-a")),
                base: BASE,
                ..RowSpec::default()
            },
        );
        approve_deletes(
            &profile_id,
            &["Old/a.txt".to_string(), "Old/gone.txt".to_string()],
        )
        .expect("approve");
        settle_approved_deletes(&profile_id).expect("settle");
        let approved = read_approved_deletes(&profile_id).expect("read");
        assert_eq!(approved, HashSet::from(["Old/a.txt".to_string()]));

        forget_sync_file(&profile_id, "Old/a.txt").expect("forget");
        settle_approved_deletes(&profile_id).expect("settle again");
        assert!(read_approved_deletes(&profile_id).expect("read").is_empty());
    }
}
