// Proof B1, the trusted side: a native strip in its own window, owned by
// its own process, at the floating window level.
import AppKit

let frame = NSRect(x: 200, y: 600, width: 800, height: 60)
let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let window = NSWindow(contentRect: frame, styleMask: [.borderless], backing: .buffered, defer: false)
window.level = .floating
window.backgroundColor = NSColor(calibratedRed: 0.1, green: 0.5, blue: 0.2, alpha: 1)
let label = NSTextField(labelWithString: "TRUSTED STRIP  —  teefax.example/channel/  🔒 encrypted")
label.font = NSFont.systemFont(ofSize: 22, weight: .bold)
label.textColor = .white
label.frame = NSRect(x: 16, y: 14, width: 760, height: 32)
window.contentView?.addSubview(label)
window.orderFrontRegardless()
app.run()
