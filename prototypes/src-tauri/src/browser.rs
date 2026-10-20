//! The browser window and its tabs.
//!
//! The window's own webview is the chrome (tab strip, toolbar, devtools).
//! Each tab is a child webview laid over the chrome's content area, with:
//! - its own `WebKit` data store, so cookies and storage are per tab;
//! - navigation locked to the app's tab page;
//! - the `tab` capability, which allows `devtools_report` and nothing else.
//!
//! Every command also checks its caller's label, so a mistake in the
//! capability files cannot hand chrome powers to a tab.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Rect, Runtime, State, TitleBarStyle, Url, Webview, WebviewBuilder,
    WebviewUrl, WebviewWindowBuilder,
};

use crate::t42;

pub const CHROME_LABEL: &str = "chrome";
const TAB_LABEL_PREFIX: &str = "tab-";
const TAB_PAGE: &str = "/tab.html";
const SRC_PARAM: &str = "src";
const DEVTOOLS_EVENT: &str = "devtools-entry";

const WINDOW_TITLE: &str = "NeoTeletext";
const WINDOW_SIZE: (f64, f64) = (1360.0, 900.0);
const MIN_WINDOW_SIZE: (f64, f64) = (760.0, 540.0);
// Centres the traffic lights in the chrome's 40px tab strip; the chrome's
// CSS reserves the same strip height (--strip-height).
const TRAFFIC_LIGHTS: (f64, f64) = (16.0, 20.0);

// One devtools report: a console line, a request, a storage snapshot or an
// image's stats. Larger payloads are refused rather than forwarded.
const MAX_REPORT_BYTES: usize = 16 * 1024;
// No display is this large; a bigger rectangle is a bug in the chrome.
const MAX_BOUNDS_EXTENT: f64 = 100_000.0;
// The short store id shown in the URL bar: enough hex to tell tabs apart.
const STORE_BADGE_CHARS: usize = 6;

#[derive(Default)]
pub struct Browser {
    content: Mutex<ContentBounds>,
}

impl Browser {
    fn content(&self) -> Result<ContentBounds, String> {
        self.content.lock().map(|bounds| *bounds).map_err(|_| "Browser state is unavailable".to_owned())
    }
}

/// The chrome's content area, in logical (CSS) pixels.
#[derive(Clone, Copy, Default, Deserialize)]
pub struct ContentBounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl ContentBounds {
    fn validated(self) -> Result<Self, String> {
        let values = [self.x, self.y, self.width, self.height];
        if values.iter().all(|value| value.is_finite() && (0.0..=MAX_BOUNDS_EXTENT).contains(value)) {
            Ok(self)
        } else {
            Err("Content bounds must be finite and on screen".to_owned())
        }
    }

    fn rect(self) -> Rect {
        Rect { position: LogicalPosition::new(self.x, self.y).into(), size: LogicalSize::new(self.width, self.height).into() }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TabCreated {
    id: String,
    store_id: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DevtoolsEntry {
    tab_id: String,
    entry: serde_json::Value,
}

pub fn open_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    WebviewWindowBuilder::new(app, CHROME_LABEL, WebviewUrl::App("index.html".into()))
        .title(WINDOW_TITLE)
        .inner_size(WINDOW_SIZE.0, WINDOW_SIZE.1)
        .min_inner_size(MIN_WINDOW_SIZE.0, MIN_WINDOW_SIZE.1)
        .title_bar_style(TitleBarStyle::Overlay)
        .hidden_title(true)
        .traffic_light_position(LogicalPosition::new(TRAFFIC_LIGHTS.0, TRAFFIC_LIGHTS.1))
        .build()?;
    Ok(())
}

fn require_chrome<R: Runtime>(caller: &Webview<R>) -> Result<(), String> {
    if caller.label() == CHROME_LABEL { Ok(()) } else { Err("Only the browser chrome can manage tabs".to_owned()) }
}

fn require_tab_label(id: &str) -> Result<(), String> {
    if id.starts_with(TAB_LABEL_PREFIX) { Ok(()) } else { Err(format!("{id} is not a tab")) }
}

fn tab<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<Webview<R>, String> {
    require_tab_label(id)?;
    app.get_webview(id).ok_or_else(|| format!("Tab {id} is closed"))
}

fn tabs<R: Runtime>(app: &AppHandle<R>) -> impl Iterator<Item = Webview<R>> {
    app.webviews().into_values().filter(|webview| webview.label().starts_with(TAB_LABEL_PREFIX))
}

fn encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

/// Scheme, host and port of the app's own pages. `Url::origin` is opaque
/// for the custom `tauri:` scheme, so it cannot be compared directly.
fn same_site(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme() && a.host_str() == b.host_str() && a.port_or_known_default() == b.port_or_known_default()
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value, reason = "Tauri extracts command arguments (AppHandle, Webview, State) by value")]
pub fn create_tab<R: Runtime>(app: AppHandle<R>, webview: Webview<R>, browser: State<'_, Browser>, src: &str) -> Result<TabCreated, String> {
    require_chrome(&webview)?;
    let src = t42::validate(src)?;
    let app_url = webview.url().map_err(|error| error.to_string())?;
    let window = app.get_window(CHROME_LABEL).ok_or("The browser window is closed")?;

    let store = uuid::Uuid::new_v4();
    let id = format!("{TAB_LABEL_PREFIX}{}", store.simple());
    let page = PathBuf::from(format!("{}?{SRC_PARAM}={}", TAB_PAGE.trim_start_matches('/'), encode(src.as_str())));
    let builder = WebviewBuilder::new(&id, WebviewUrl::App(page))
        // macOS 14+: a WKWebsiteDataStore of its own, so cookies, storage
        // and caches are never shared with other tabs or the chrome.
        .data_store_identifier(*store.as_bytes())
        .on_navigation(move |url| same_site(url, &app_url) && url.path() == TAB_PAGE);

    let bounds = browser.content()?;
    window
        .add_child(builder, LogicalPosition::new(bounds.x, bounds.y), LogicalSize::new(bounds.width, bounds.height))
        .map_err(|error| error.to_string())?;
    activate(&app, &id)?;

    let store_id = store.simple().to_string()[..STORE_BADGE_CHARS].to_owned();
    Ok(TabCreated { id, store_id })
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value, reason = "Tauri extracts command arguments (AppHandle, Webview, State) by value")]
pub fn close_tab<R: Runtime>(app: AppHandle<R>, webview: Webview<R>, id: &str) -> Result<(), String> {
    require_chrome(&webview)?;
    tab(&app, id)?.close().map_err(|error| error.to_string())
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value, reason = "Tauri extracts command arguments (AppHandle, Webview, State) by value")]
pub fn select_tab<R: Runtime>(app: AppHandle<R>, webview: Webview<R>, id: &str) -> Result<(), String> {
    require_chrome(&webview)?;
    activate(&app, id)
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value, reason = "Tauri extracts command arguments (AppHandle, Webview, State) by value")]
pub fn navigate_tab<R: Runtime>(app: AppHandle<R>, webview: Webview<R>, id: &str, src: &str) -> Result<(), String> {
    require_chrome(&webview)?;
    let src = t42::validate(src)?;
    let target = tab(&app, id)?;
    let mut url = target.url().map_err(|error| error.to_string())?;
    url.query_pairs_mut().clear().append_pair(SRC_PARAM, src.as_str());
    target.navigate(url).map_err(|error| error.to_string())
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value, reason = "Tauri extracts command arguments (AppHandle, Webview, State) by value")]
pub fn set_content_bounds<R: Runtime>(
    app: AppHandle<R>,
    webview: Webview<R>,
    browser: State<'_, Browser>,
    bounds: ContentBounds,
) -> Result<(), String> {
    require_chrome(&webview)?;
    let bounds = bounds.validated()?;
    *browser.content.lock().map_err(|_| "Browser state is unavailable")? = bounds;
    for tab in tabs(&app) {
        tab.set_bounds(bounds.rect()).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// The one native call a tab may make. The tab id comes from the runtime's
/// record of the calling webview, never from the payload, so a tab cannot
/// report as another tab.
#[tauri::command]
#[allow(clippy::needless_pass_by_value, reason = "Tauri extracts command arguments (AppHandle, Webview, State) by value")]
pub fn devtools_report<R: Runtime>(app: AppHandle<R>, webview: Webview<R>, entry: serde_json::Value) -> Result<(), String> {
    let tab_id = webview.label().to_owned();
    require_tab_label(&tab_id)?;
    let size = serde_json::to_string(&entry).map_err(|error| error.to_string())?.len();
    if size > MAX_REPORT_BYTES {
        return Err(format!("Report is {size} bytes; the limit is {MAX_REPORT_BYTES}"));
    }
    app.emit_to(CHROME_LABEL, DEVTOOLS_EVENT, DevtoolsEntry { tab_id, entry }).map_err(|error| error.to_string())
}

fn activate<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<(), String> {
    let target = tab(app, id)?;
    for other in tabs(app).filter(|other| other.label() != id) {
        other.hide().map_err(|error| error.to_string())?;
    }
    target.show().map_err(|error| error.to_string())?;
    target.set_focus().map_err(|error| error.to_string())
}
