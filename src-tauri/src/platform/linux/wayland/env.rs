/// The app's identity on Linux: Wayland app_id, GTK prgname, and the launcher
/// name (`<APP_ID>.desktop`). Must equal the Tauri `identifier`. It must not
/// end in `.desktop`: KDE's kglobalacceld treats such component names as
/// desktop-file launch shortcuts and silently drops portal-registered global
/// shortcuts for them. It must not end in `.app` either, which collides with
/// macOS bundle names.
pub const APP_ID: &str = "info.brumley.somedrive";

/// Sets the app identity before any GTK or portal use, so the Wayland app_id
/// (taken from the GTK prgname) matches `<APP_ID>.desktop` and the desktop
/// shows SomeDrive's own icon and name for its window.
pub fn configure_linux_app_identity() {
    gtk::glib::set_prgname(Some(APP_ID));
    gtk::glib::set_application_name("SomeDrive");
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
