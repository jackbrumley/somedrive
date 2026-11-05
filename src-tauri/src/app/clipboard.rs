//! The one place SomeDrive writes to the system clipboard. The webview's own
//! clipboard API is unreliable under WebKitGTK, so copies go through here.

/// Puts `text` on the clipboard.
pub fn copy_text(text: String) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|error| error.to_string())?;
    clipboard.set_text(text).map_err(|error| error.to_string())
}

/// A copy button in the UI, e.g. the file name to type when deleting it.
#[tauri::command]
pub fn copy_text_to_clipboard(text: String) -> Result<(), String> {
    copy_text(text)
}
