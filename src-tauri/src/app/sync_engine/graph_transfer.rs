use super::*;

pub(crate) fn graph_http_client_no_redirect() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(resolve_connect_timeout())
        .timeout(resolve_request_timeout())
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("Failed creating no-redirect Graph HTTP client: {error}"))
}

// the transfer needs the job, the item, the planned version and the target;
// a download request struct is tracked with the cycle context in docs/TODO.md.
pub(crate) async fn create_remote_folder(
    graph: &mut GraphContext,
    relative_path: &str,
    cancel_flag: &Arc<AtomicBool>,
) -> SyncResult<GraphDriveItem> {
    ensure_not_cancelled(cancel_flag)?;
    let (parent, name) = split_parent_and_name(relative_path)?;
    let endpoint = if parent.is_empty() {
        format!("{GRAPH_ROOT}/me/drive/root/children")
    } else {
        format!(
            "{GRAPH_ROOT}/me/drive/root:/{}:/children",
            encode_graph_path(parent)
        )
    };
    let payload = serde_json::json!({
        "name": name,
        "folder": {},
        "@microsoft.graph.conflictBehavior": "fail"
    });
    log::info!(
        "{} [cycle:{}] REMOTE_DIR_CREATE_REQUEST path={} parent={}",
        graph.account_prefix,
        graph.cycle_id,
        relative_path,
        parent
    );

    let client = graph.http.clone();
    let mut refreshed = false;
    loop {
        ensure_not_cancelled(cancel_flag)?;
        graph.wait_for_throttle_window(cancel_flag).await?;
        let access_token = graph.current_access_token().await;
        let response = tokio::select! {
            _ = wait_for_cancellation(Arc::clone(cancel_flag)) => {
                return Err(SyncError::from(SYNC_CANCELLED_ERROR.to_string()));
            }
            value = client
                .post(&endpoint)
                .bearer_auth(&access_token)
                .json(&payload)
                .send() => {
                value.map_err(|error| {
                    format!(
                        "Failed creating remote folder '{}': {}",
                        relative_path, error
                    )
                })?
            }
        };

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| format!("Failed reading create folder response: {error}"))?;

        if status.as_u16() == 401 && !refreshed {
            log::warn!(
                "{} [cycle:{}] GRAPH_DIR_CREATE_401_REFRESH",
                graph.account_prefix,
                graph.cycle_id
            );
            let _ = graph.refresh_token_if_needed(&access_token).await?;
            refreshed = true;
            continue;
        }

        if status == StatusCode::CONFLICT {
            // Something already exists at this path: use it if it is a
            // folder, never replace it.
            let existing = fetch_remote_item_by_path(graph, relative_path, cancel_flag)
                .await?
                .filter(|item| item.folder.is_some())
                .ok_or_else(|| {
                    format!("A file in OneDrive is in the way of folder '{relative_path}'")
                })?;
            log::info!(
                "{} [cycle:{}] REMOTE_DIR_EXISTS path={} remote_id={}",
                graph.account_prefix,
                graph.cycle_id,
                relative_path,
                existing.id
            );
            return Ok(existing);
        }
        if !status.is_success() {
            let snippet: String = text.chars().take(400).collect();
            return Err(SyncError::from(format!(
                "Create folder failed for '{}' with status {}: {}",
                relative_path, status, snippet
            )));
        }
        let parsed = serde_json::from_str::<GraphDriveItem>(&text)
            .map_err(|error| format!("Failed decoding create-folder response JSON: {error}"))?;
        log::info!(
            "{} [cycle:{}] REMOTE_DIR_CREATE_OK path={} remote_id={}",
            graph.account_prefix,
            graph.cycle_id,
            relative_path,
            parsed.id
        );
        return Ok(parsed);
    }
}

pub(crate) async fn delete_remote_item(
    graph: &mut GraphContext,
    item_id: &str,
    cancel_flag: &Arc<AtomicBool>,
) -> SyncResult<()> {
    ensure_not_cancelled(cancel_flag)?;
    let url = format!("{GRAPH_ROOT}/me/drive/items/{}", item_id);
    log::info!(
        "{} [cycle:{}] REMOTE_DELETE_REQUEST item_id={}",
        graph.account_prefix,
        graph.cycle_id,
        item_id
    );
    graph_delete(graph, &url).await?;
    log::info!(
        "{} [cycle:{}] REMOTE_DELETE_RESULT item_id={} status=ok",
        graph.account_prefix,
        graph.cycle_id,
        item_id
    );
    Ok(())
}
