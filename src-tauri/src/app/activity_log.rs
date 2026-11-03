use crate::app::json_store;
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEvent {
    pub id: String,
    pub profile_id: String,
    pub profile_name: String,
    pub kind: String,
    pub message: String,
    pub timestamp: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ActivityStore {
    events: Vec<ActivityEvent>,
}

pub fn list_events(limit: usize) -> Result<Vec<ActivityEvent>, String> {
    let path = events_file_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }

    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let store: ActivityStore = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    let mut events = store.events;
    events.reverse();
    Ok(events.into_iter().take(limit).collect())
}

pub fn append_event(
    profile_id: &str,
    profile_name: &str,
    kind: &str,
    message: &str,
) -> Result<(), String> {
    let mut events = load_all_events()?;
    events.push(ActivityEvent {
        id: generate_event_id(),
        profile_id: profile_id.to_string(),
        profile_name: profile_name.to_string(),
        kind: kind.to_string(),
        message: message.to_string(),
        timestamp: Local::now().to_rfc3339(),
    });

    if events.len() > 1200 {
        let start = events.len().saturating_sub(1200);
        events = events[start..].to_vec();
    }

    save_all_events(&events)
}

fn load_all_events() -> Result<Vec<ActivityEvent>, String> {
    Ok(
        json_store::read_json::<ActivityStore>(&events_file_path()?)?
            .map(|store| store.events)
            .unwrap_or_default(),
    )
}

fn save_all_events(events: &[ActivityEvent]) -> Result<(), String> {
    let store = ActivityStore {
        events: events.to_vec(),
    };
    json_store::write_json(&events_file_path()?, &store)
}

fn events_dir_path() -> Result<PathBuf, String> {
    crate::paths::activity_dir()
}

fn events_file_path() -> Result<PathBuf, String> {
    Ok(events_dir_path()?.join("events.json"))
}

fn generate_event_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("evt-{}", nanos)
}
