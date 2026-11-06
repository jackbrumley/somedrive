//! The app's identity, per the Release and Update Standard's "App Identity"
//! section: one reverse-DNS ID used as the Tauri `identifier`, the Wayland
//! `app_id` / GTK prgname, the tray's StatusNotifierItem ID and the launcher
//! name (`<APP_ID>.desktop`).

/// Must equal the Tauri `identifier`. It must not end in `.desktop`: KDE's
/// kglobalacceld treats such component names as desktop-file launch shortcuts
/// and silently drops portal-registered global shortcuts for them. It must
/// not end in `.app` either, which collides with macOS bundle names.
pub const APP_ID: &str = "info.brumley.somedrive";

#[cfg(test)]
mod tests {
    use super::APP_ID;

    #[test]
    fn app_id_matches_tauri_identifier() {
        let tauri_config: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        assert_eq!(tauri_config["identifier"], APP_ID);
    }

    #[test]
    fn launcher_is_named_after_app_id() {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let launcher = manifest_dir.join(format!("packaging/{APP_ID}.desktop"));
        let contents = std::fs::read_to_string(&launcher).unwrap();
        assert!(contents.contains(&format!("StartupWMClass={APP_ID}")));
    }

    #[test]
    fn app_id_avoids_reserved_suffixes() {
        assert!(!APP_ID.ends_with(".desktop"));
        assert!(!APP_ID.ends_with(".app"));
    }
}
