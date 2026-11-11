use super::*;

// The transfer rate each account has learned (`download_limit`), kept in
// `sync_download_rate` so a new cycle or a restart starts at the settled
// rate instead of climbing into Microsoft's limit again.

/// The account's learned rate; `None` before its first transfers.
pub(crate) fn read_learned_download_rate(
    profile_id: &str,
) -> Result<Option<LearnedDownloadRate>, String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .query_row(
            "SELECT parallel, starts_per_second, ceiling_parallel, ceiling_starts_per_second
             FROM sync_download_rate WHERE profile_id = ?1",
            params![profile_id],
            |row| {
                let ceiling_parallel = row.get::<_, Option<i64>>(2)?;
                let ceiling_rate = row.get::<_, Option<f64>>(3)?;
                Ok(LearnedDownloadRate {
                    parallel: row.get::<_, i64>(0)?.max(0) as usize,
                    starts_per_second: row.get(1)?,
                    ceiling: ceiling_parallel
                        .zip(ceiling_rate)
                        .map(|(parallel, rate)| (parallel.max(0) as usize, rate)),
                })
            },
        )
        .optional()
        .map_err(|error| format!("Failed reading the learned transfer rate: {error}"))
}

/// Saves the account's learned rate.
pub(crate) fn save_learned_download_rate(
    profile_id: &str,
    rate: &LearnedDownloadRate,
) -> Result<(), String> {
    let connection = open_sync_jobs_connection(profile_id)?;
    connection
        .execute(
            "INSERT INTO sync_download_rate (
                 profile_id, parallel, starts_per_second,
                 ceiling_parallel, ceiling_starts_per_second, updated_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(profile_id) DO UPDATE SET
                 parallel = excluded.parallel,
                 starts_per_second = excluded.starts_per_second,
                 ceiling_parallel = excluded.ceiling_parallel,
                 ceiling_starts_per_second = excluded.ceiling_starts_per_second,
                 updated_at = excluded.updated_at",
            params![
                profile_id,
                rate.parallel as i64,
                rate.starts_per_second,
                rate.ceiling.map(|(parallel, _)| parallel as i64),
                rate.ceiling.map(|(_, rate)| rate),
                current_unix_seconds(),
            ],
        )
        .map_err(|error| format!("Failed saving the learned transfer rate: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod download_rate_store_tests {
    use super::*;

    #[test]
    fn a_learned_rate_survives_a_save_and_read() {
        let profile_id = format!(
            "download-rate-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        assert_eq!(read_learned_download_rate(&profile_id).expect("read"), None);
        let first = LearnedDownloadRate {
            parallel: 9,
            starts_per_second: 11.5,
            ceiling: None,
        };
        save_learned_download_rate(&profile_id, &first).expect("save");
        assert_eq!(
            read_learned_download_rate(&profile_id).expect("read"),
            Some(first)
        );
        let refused = LearnedDownloadRate {
            parallel: 12,
            starts_per_second: 10.2,
            ceiling: Some((14, 12.0)),
        };
        save_learned_download_rate(&profile_id, &refused).expect("save again");
        assert_eq!(
            read_learned_download_rate(&profile_id).expect("read"),
            Some(refused)
        );
    }
}
