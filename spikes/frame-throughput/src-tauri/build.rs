fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&["start", "parameters", "report"])))
        .expect("tauri build script failed");
}
