// Proof B2: one window, one process. A native strip sits above a WKWebView
// whose page tries to draw over it, open windows and go full screen.
import AppKit
import WebKit

let stripHeight: CGFloat = 60
let size = NSSize(width: 800, height: 400)
let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let window = NSWindow(contentRect: NSRect(origin: NSPoint(x: 200, y: 200), size: size),
                      styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
window.title = "Proof B2"
let content = NSView(frame: NSRect(origin: .zero, size: size))
window.contentView = content

let webView = WKWebView(frame: NSRect(x: 0, y: 0, width: size.width, height: size.height - stripHeight))
webView.autoresizingMask = [.width, .height]
content.addSubview(webView)

let strip = NSView(frame: NSRect(x: 0, y: size.height - stripHeight, width: size.width, height: stripHeight))
strip.wantsLayer = true
strip.layer?.backgroundColor = NSColor(calibratedRed: 0.1, green: 0.5, blue: 0.2, alpha: 1).cgColor
strip.autoresizingMask = [.width, .minYMargin]
let label = NSTextField(labelWithString: "TRUSTED STRIP  —  teefax.example/channel/  🔒 encrypted")
label.font = NSFont.systemFont(ofSize: 22, weight: .bold)
label.textColor = .white
label.frame = NSRect(x: 16, y: 14, width: 760, height: 32)
strip.addSubview(label)
content.addSubview(strip)

let hostile = """
<!doctype html><html><body style="margin:0;background:#222">
<div style="position:fixed;top:-200px;left:-50px;width:2000px;height:300px;z-index:2147483647;
background:#a11;color:#fff;font:bold 22px -apple-system;padding:210px 0 0 66px;transform:translateY(-100px) scale(1.5)">
FAKE STRIP drawn by the page — evil.example</div>
<p style="color:#fff;font:16px -apple-system;margin:120px 20px">The page is trying to draw over the strip, open a window and go full screen.</p>
<script>
window.open('about:blank', '_blank', 'popup');
document.documentElement.requestFullscreen?.().catch(() => {});
</script></body></html>
"""
webView.loadHTMLString(hostile, baseURL: nil)
window.orderFrontRegardless()
app.run()
