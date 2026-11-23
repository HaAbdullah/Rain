fn main() {
    // The one command of the shell is named here, so the page may call exactly this and nothing
    // else (foundation section 6.4). Its permission is called `allow-api-request`.
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&["api_request"])),
    )
    .expect("tauri build failed");
}
