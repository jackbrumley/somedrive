//! The one place SomeDrive writes to the system clipboard. The webview's own
//! clipboard API is unreliable under WebKitGTK, so copies go through here.

use std::sync::Mutex;

/// One clipboard for the app's whole life. On Linux the copying app must
/// keep serving the text until something pastes it; a handle dropped right
/// after `set_text` can leave the paste empty unless a clipboard manager
/// grabbed it first.
static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);

/// Puts `text` on the clipboard.
pub fn copy_text(text: String) -> Result<(), String> {
    let mut held = CLIPBOARD
        .lock()
        .map_err(|_| "The clipboard is unavailable after an earlier failure.".to_string())?;
    let clipboard = match held.as_mut() {
        Some(clipboard) => clipboard,
        None => held.insert(arboard::Clipboard::new().map_err(|error| error.to_string())?),
    };
    clipboard.set_text(text).map_err(|error| error.to_string())
}

/// A copy button in the UI, e.g. the file name to type when deleting it.
#[tauri::command]
pub fn copy_text_to_clipboard(text: String) -> Result<(), String> {
    copy_text(text)
}
