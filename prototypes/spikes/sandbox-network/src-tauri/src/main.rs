//! Proof A. The page tries every way of reaching the network and reports
//! what it saw; the test server's log is the ground truth. When
//! `SPIKE_PROXY` is set, every load of the webview is sent to that proxy,
//! to test whether a dead proxy confines a webview that must keep the
//! network entitlement.

use tauri::{Url, WebviewUrl, WebviewWindowBuilder};

const WINDOW_SIZE: (f64, f64) = (600.0, 400.0);

// Removes WebRTC before any page script runs, in every frame, because the
// Content Security Policy does not govern it. Enabled by SPIKE_NO_WEBRTC.
const REMOVE_WEBRTC: &str = r"
for (const name of ['RTCPeerConnection', 'webkitRTCPeerConnection', 'RTCDataChannel', 'RTCIceCandidate', 'RTCSessionDescription', 'RTCRtpSender', 'RTCRtpReceiver', 'RTCRtpTransceiver']) {
    try { delete window[name]; Object.defineProperty(window, name, {value: undefined, writable: false, configurable: false}); } catch (_) {}
}
";

#[tauri::command]
#[allow(clippy::needless_pass_by_value, reason = "Tauri extracts command arguments by value")]
fn report(app: tauri::AppHandle, lines: Vec<String>) {
    for line in &lines {
        println!("PROBE {line}");
    }
    app.exit(0);
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![report])
        .setup(|app| {
            let mut builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("Proof A")
                .inner_size(WINDOW_SIZE.0, WINDOW_SIZE.1);
            if std::env::var("SPIKE_NO_WEBRTC").is_ok() {
                println!("PROBE WebRTC removal: on");
                builder = builder.initialization_script_for_all_frames(REMOVE_WEBRTC);
            }
            if let Ok(proxy) = std::env::var("SPIKE_PROXY") {
                println!("PROBE proxy: {proxy}");
                builder = builder.proxy_url(Url::parse(&proxy)?);
            }
            let window = builder.build()?;
            // Linux: WebKitGTK's web process sandbox is off unless the host
            // turns it on, and wry does not. SPIKE_WEBKIT_SANDBOX tests
            // whether it can still be turned on once Tauri has built the
            // webview; WebKitGTK only honours it before its first web process.
            #[cfg(target_os = "linux")]
            if std::env::var("SPIKE_WEBKIT_SANDBOX").is_ok() {
                use webkit2gtk::{WebContextExt, WebViewExt};
                window.with_webview(|webview| {
                    if let Some(context) = webview.inner().context() {
                        context.set_sandbox_enabled(true);
                        println!("PROBE WebKitGTK sandbox requested; now reported as {}", context.is_sandbox_enabled());
                    }
                })?;
            }
            #[cfg(not(target_os = "linux"))]
            let _ = window;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Tauri runtime failed to start");
}
