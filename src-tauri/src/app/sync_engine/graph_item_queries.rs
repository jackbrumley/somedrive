// Fresh reads of single items from OneDrive, used to re-check an item just
// before acting on it.

/// The item as OneDrive holds it right now; `None` when it no longer exists.
async fn fetch_remote_item(
    graph: &GraphContext,
    item_id: &str,
    cancel_flag: &Arc<AtomicBool>,
) -> SyncResult<Option<GraphDriveItem>>  {
    let url = format!("{GRAPH_ROOT}/me/drive/items/{item_id}");
    let Some(text) = graph_get_text_if_exists(graph, &url, cancel_flag).await? else {
        return Ok(None);
    };
    let item = serde_json::from_str::<GraphDriveItem>(&text)
        .map_err(|error| format!("Failed decoding item {item_id}: {error}"))?;
    Ok(Some(item))
}

/// The item at `relative_path` in OneDrive; `None` when there is none.
async fn fetch_remote_item_by_path(
    graph: &GraphContext,
    relative_path: &str,
    cancel_flag: &Arc<AtomicBool>,
) -> SyncResult<Option<GraphDriveItem>>  {
    let url = format!("{GRAPH_ROOT}/me/drive/root:/{}", encode_graph_path(relative_path));
    let Some(text) = graph_get_text_if_exists(graph, &url, cancel_flag).await? else {
        return Ok(None);
    };
    let item = serde_json::from_str::<GraphDriveItem>(&text)
        .map_err(|error| format!("Failed decoding item at '{relative_path}': {error}"))?;
    Ok(Some(item))
}

/// Whether the cloud folder currently has any children (a fresh listing,
/// not the stored state; Graph deletes folders recursively).
async fn remote_folder_has_children(
    graph: &GraphContext,
    item_id: &str,
    cancel_flag: &Arc<AtomicBool>,
) -> SyncResult<bool>  {
    #[derive(Deserialize)]
    struct ChildrenPage {
        value: Vec<serde_json::Value>,
    }
    let url = format!("{GRAPH_ROOT}/me/drive/items/{item_id}/children?$top=1&$select=id");
    let Some(text) = graph_get_text_if_exists(graph, &url, cancel_flag).await? else {
        return Ok(false);
    };
    let page: ChildrenPage = serde_json::from_str(&text)
        .map_err(|error| format!("Failed decoding children of {item_id}: {error}"))?;
    Ok(!page.value.is_empty())
}
