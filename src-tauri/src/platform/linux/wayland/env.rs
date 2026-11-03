/// The app's identity on Linux: Wayland app_id, GTK prgname, and the launcher
/// name (`<APP_ID>.desktop`). Must equal the Tauri `identifier`. It must not
/// end in `.desktop`: KDE's kglobalacceld treats such component names as
/// desktop-file launch shortcuts and silently drops portal-registered global
/// shortcuts for them. It must not end in `.app` either, which collides with
/// macOS bundle names.
pub const APP_ID: &str = "info.brumley.somedrive";

/// Process environment for a Linux desktop session (App Standard section 2).
/// Must run first in `main`, before Tauri, GTK or any portal call.
pub fn configure_linux_session_environment() {
    // The app identity: GTK uses the prgname as the Wayland app_id, so the
    // desktop matches the window to `<APP_ID>.desktop` (its icon and name).
    gtk::glib::set_prgname(Some(APP_ID));
    gtk::glib::set_application_name("SomeDrive");

    // WebKitGTK shows a blank window or crashes with "Could not create default
    // EGL display: EGL_BAD_PARAMETER" on Arch-based distros (CachyOS, Manjaro)
    // and some NVIDIA setups unless its DMA-BUF renderer and GPU sandbox are
    // disabled.
    std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    std::env::set_var("WEBKIT_DISABLE_GPU_SANDBOX", "1");

    if should_force_wayland_backend() {
        // Native Wayland, rather than XWayland, on Wayland sessions.
        std::env::set_var("GDK_BACKEND", "wayland");
    }
}

/// Force GTK's native Wayland backend only on a real Wayland session with a
/// reachable display, and never over a `GDK_BACKEND` the user chose
/// (APP_STANDARD.md §2).
fn should_force_wayland_backend() -> bool {
    std::env::var_os("GDK_BACKEND").is_none()
        && std::env::var("XDG_SESSION_TYPE")
            .is_ok_and(|value| value.eq_ignore_ascii_case("wayland"))
        && std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::APP_ID;

    #[test]
    fn app_id_matches_tauri_identifier() {
        let tauri_config: serde_json::Value =
            serde_json::from_str(include_str!("../../../../tauri.conf.json")).unwrap();
        assert_eq!(tauri_config["identifier"], APP_ID);
    }

    #[test]
    fn packaging_files_are_named_after_app_id() {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(manifest_dir
            .join(format!("packaging/{APP_ID}.desktop"))
            .is_file());
    }

    #[test]
    fn app_id_avoids_reserved_suffixes() {
        assert!(!APP_ID.ends_with(".desktop"));
        assert!(!APP_ID.ends_with(".app"));
    }
}
