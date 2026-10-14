// Proof B1, the hostile side: a different process places a window of its own
// exactly over the strip, at a higher window level, imitating it.
import AppKit

let frame = NSRect(x: 200, y: 600, width: 800, height: 60)
let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let window = NSWindow(contentRect: frame, styleMask: [.borderless], backing: .buffered, defer: false)
window.level = .screenSaver
window.backgroundColor = NSColor(calibratedRed: 0.6, green: 0.1, blue: 0.1, alpha: 1)
let label = NSTextField(labelWithString: "FAKE STRIP drawn by another process  —  evil.example")
label.font = NSFont.systemFont(ofSize: 22, weight: .bold)
label.textColor = .white
label.frame = NSRect(x: 16, y: 14, width: 760, height: 32)
window.contentView?.addSubview(label)
window.orderFrontRegardless()
app.run()
