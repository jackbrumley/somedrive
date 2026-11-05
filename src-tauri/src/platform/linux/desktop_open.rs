//! Showing and opening synced items through the desktop, the same way on
//! every Linux desktop:
//! - showing an item selected in its folder uses the freedesktop
//!   `org.freedesktop.FileManager1.ShowItems` call (Nemo, Nautilus, Dolphin,
//!   Caja, Thunar, PCManFM-Qt, elementary Files). Where no file manager
//!   answers it, the folder is opened with `xdg-open` instead, and the
//!   fallback is logged;
//! - opening a file in its default app uses `xdg-open` (bundled with every
//!   package).

use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// How long a file manager has to answer before the folder is opened
/// instead (it may be starting up).
const SHOW_ITEMS_TIMEOUT: Duration = Duration::from_secs(5);

/// Opens `path` (a file or folder) with the desktop's default app.
pub fn open_with_default_app(path: &Path) -> Result<(), String> {
    Command::new("xdg-open")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not open '{}': {error}", path.display()))
}

/// Opens the folder holding `item` with `item` selected; opens the folder
/// alone when the file manager cannot select it.
pub async fn show_in_folder(item: &Path, folder: &Path) -> Result<(), String> {
    match tokio::time::timeout(SHOW_ITEMS_TIMEOUT, show_items(item)).await {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => {
            log::info!("SHOW_IN_FOLDER_FALLBACK reason=file_manager_refused detail={error}")
        }
        Err(_) => log::info!("SHOW_IN_FOLDER_FALLBACK reason=no_answer_in_5s"),
    }
    open_with_default_app(folder)
}

async fn show_items(item: &Path) -> Result<(), String> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|error| error.to_string())?;
    connection
        .call_method(
            Some("org.freedesktop.FileManager1"),
            "/org/freedesktop/FileManager1",
            Some("org.freedesktop.FileManager1"),
            "ShowItems",
            &(vec![file_uri(item)], ""),
        )
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// A `file://` URI, percent-encoding every byte outside the unreserved set.
fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris_encode_spaces_and_symbols() {
        assert_eq!(
            file_uri(Path::new("/home/a/GBA Backup/Tony Hawk's (U) [!].sav")),
            "file:///home/a/GBA%20Backup/Tony%20Hawk%27s%20%28U%29%20%5B%21%5D.sav"
        );
        assert_eq!(file_uri(Path::new("/tmp/é")), "file:///tmp/%C3%A9");
    }
}
