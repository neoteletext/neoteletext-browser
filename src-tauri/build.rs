// Declaring the commands generates an `allow-<command>` permission for each,
// so capabilities can grant them per webview. Undeclared means unreachable.
const COMMANDS: &[&str] = &["create_tab", "close_tab", "select_tab", "navigate_tab", "set_content_bounds", "devtools_report"];

fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)))
        .expect("tauri build script failed");
}
