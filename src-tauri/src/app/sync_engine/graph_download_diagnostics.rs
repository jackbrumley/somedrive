use super::*;

// Diagnostics for HTTP 403 download failures: the Graph error and request
// ids a Microsoft support case needs. On unless SOMEDRIVE_SYNC_403_DIAGNOSTICS
// is 0, false, off or no.

pub(crate) fn download_403_diagnostics_enabled() -> bool {
    std::env::var("SOMEDRIVE_SYNC_403_DIAGNOSTICS")
        .ok()
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            !(normalized == "0"
                || normalized == "false"
                || normalized == "off"
                || normalized == "no")
        })
        .unwrap_or(true)
}

pub(crate) fn extract_graph_error_summary(body: &str) -> (String, String, String, String) {
    let parsed = serde_json::from_str::<serde_json::Value>(body).ok();
    let error_code = parsed
        .as_ref()
        .and_then(|root| root.get("error"))
        .and_then(|error| error.get("code"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let error_message = parsed
        .as_ref()
        .and_then(|root| root.get("error"))
        .and_then(|error| error.get("message"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let body_request_id = parsed
        .as_ref()
        .and_then(|root| root.get("error"))
        .and_then(|error| error.get("innerError"))
        .and_then(|inner| inner.get("request-id"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let body_client_request_id = parsed
        .as_ref()
        .and_then(|root| root.get("error"))
        .and_then(|error| error.get("innerError"))
        .and_then(|inner| inner.get("client-request-id"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    (
        error_code,
        error_message,
        body_request_id,
        body_client_request_id,
    )
}

pub(crate) fn header_value_text(headers: &reqwest::header::HeaderMap, key: &str) -> String {
    headers
        .get(key)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string()
}

pub(crate) async fn log_download_403_diagnostics(
    graph: &GraphContext,
    item_id: &str,
    relative_path: &str,
    attempt: u32,
    job_id: Option<i64>,
    response_headers: &reqwest::header::HeaderMap,
    response_body: &str,
) {
    if !download_403_diagnostics_enabled() {
        return;
    }

    let (error_code, error_message, body_request_id, body_client_request_id) =
        extract_graph_error_summary(response_body);
    let header_request_id = header_value_text(response_headers, "request-id");
    let header_client_request_id = header_value_text(response_headers, "client-request-id");
    log::warn!(
        "{} [cycle:{}] DOWNLOAD_403_DIAGNOSTIC item_id={} path={} attempt={} job_id={} graph_error_code={} graph_error_message={} header_request_id={} header_client_request_id={} body_request_id={} body_client_request_id={}",
        graph.account_prefix,
        graph.cycle_id,
        item_id,
        relative_path,
        attempt,
        job_id.unwrap_or_default(),
        error_code,
        error_message,
        header_request_id,
        header_client_request_id,
        body_request_id,
        body_client_request_id
    );

    let client = graph.http.clone();
    let access_token = graph.current_access_token().await;

    let metadata_url = format!(
        "{GRAPH_ROOT}/me/drive/items/{}?$select=id,name,size,file,folder,package,remoteItem,parentReference,webUrl,lastModifiedDateTime",
        item_id
    );
    match client
        .get(&metadata_url)
        .bearer_auth(&access_token)
        .send()
        .await
    {
        Ok(metadata_response) => {
            let metadata_status = metadata_response.status();
            let metadata_text = metadata_response.text().await.unwrap_or_default();
            if metadata_status.is_success() {
                let parsed = serde_json::from_str::<serde_json::Value>(&metadata_text).ok();
                let has_file = parsed
                    .as_ref()
                    .and_then(|value| value.get("file"))
                    .is_some();
                let has_folder = parsed
                    .as_ref()
                    .and_then(|value| value.get("folder"))
                    .is_some();
                let has_package = parsed
                    .as_ref()
                    .and_then(|value| value.get("package"))
                    .is_some();
                let has_remote_item = parsed
                    .as_ref()
                    .and_then(|value| value.get("remoteItem"))
                    .is_some();
                let web_url = parsed
                    .as_ref()
                    .and_then(|value| value.get("webUrl"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let parent_path = parsed
                    .as_ref()
                    .and_then(|value| value.get("parentReference"))
                    .and_then(|value| value.get("path"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                log::warn!(
                    "{} [cycle:{}] DOWNLOAD_403_ITEM_METADATA item_id={} path={} metadata_status={} has_file={} has_folder={} has_package={} has_remote_item={} parent_path={} web_url={}",
                    graph.account_prefix,
                    graph.cycle_id,
                    item_id,
                    relative_path,
                    metadata_status,
                    has_file,
                    has_folder,
                    has_package,
                    has_remote_item,
                    parent_path,
                    web_url
                );
            } else {
                let snippet: String = metadata_text.chars().take(300).collect();
                log::warn!(
                    "{} [cycle:{}] DOWNLOAD_403_ITEM_METADATA_FAILED item_id={} path={} metadata_status={} body_snippet={}",
                    graph.account_prefix,
                    graph.cycle_id,
                    item_id,
                    relative_path,
                    metadata_status,
                    snippet
                );
            }
        }
        Err(error) => {
            log::warn!(
                "{} [cycle:{}] DOWNLOAD_403_ITEM_METADATA_FAILED item_id={} path={} error={}",
                graph.account_prefix,
                graph.cycle_id,
                item_id,
                relative_path,
                error
            );
        }
    }

    let permissions_url = format!("{GRAPH_ROOT}/me/drive/items/{}/permissions", item_id);
    match client
        .get(&permissions_url)
        .bearer_auth(&access_token)
        .send()
        .await
    {
        Ok(permissions_response) => {
            let permissions_status = permissions_response.status();
            let permissions_text = permissions_response.text().await.unwrap_or_default();
            if permissions_status.is_success() {
                let parsed = serde_json::from_str::<serde_json::Value>(&permissions_text).ok();
                let permission_items = parsed
                    .as_ref()
                    .and_then(|value| value.get("value"))
                    .and_then(serde_json::Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let roles: Vec<String> = permission_items
                    .iter()
                    .flat_map(|permission| {
                        permission
                            .get("roles")
                            .and_then(serde_json::Value::as_array)
                            .cloned()
                            .unwrap_or_default()
                    })
                    .filter_map(|role| role.as_str().map(ToString::to_string))
                    .collect();
                let unique_roles: std::collections::BTreeSet<String> = roles.into_iter().collect();
                let role_summary = unique_roles.into_iter().collect::<Vec<String>>().join(",");
                log::warn!(
                    "{} [cycle:{}] DOWNLOAD_403_PERMISSIONS item_id={} path={} permissions_status={} permission_count={} roles={}",
                    graph.account_prefix,
                    graph.cycle_id,
                    item_id,
                    relative_path,
                    permissions_status,
                    permission_items.len(),
                    role_summary
                );
            } else {
                let snippet: String = permissions_text.chars().take(300).collect();
                log::warn!(
                    "{} [cycle:{}] DOWNLOAD_403_PERMISSIONS_FAILED item_id={} path={} permissions_status={} body_snippet={}",
                    graph.account_prefix,
                    graph.cycle_id,
                    item_id,
                    relative_path,
                    permissions_status,
                    snippet
                );
            }
        }
        Err(error) => {
            log::warn!(
                "{} [cycle:{}] DOWNLOAD_403_PERMISSIONS_FAILED item_id={} path={} error={}",
                graph.account_prefix,
                graph.cycle_id,
                item_id,
                relative_path,
                error
            );
        }
    }
}
