mod browser;
mod menu;
mod t42;

fn main() {
    tauri::Builder::default()
        .manage(browser::Browser::default())
        .menu(menu::build)
        .on_menu_event(|app, event| menu::forward(app, &event))
        .invoke_handler(tauri::generate_handler![
            browser::create_tab,
            browser::close_tab,
            browser::select_tab,
            browser::navigate_tab,
            browser::set_content_bounds,
            browser::devtools_report,
        ])
        .setup(|app| Ok(browser::open_window(app.handle())?))
        .run(tauri::generate_context!())
        .expect("Tauri runtime failed to start");
}
