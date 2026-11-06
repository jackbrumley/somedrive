use crate::app::identity::APP_ID;

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
