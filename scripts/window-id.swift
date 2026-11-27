// Prints the CGWindowID of the frontmost layer-0 on-screen window owned by a
// PID, or exits 1. Usage: window-id <pid>. Compiled on demand by app-shot.ts.
import CoreGraphics
import Foundation

guard CommandLine.arguments.count > 1, let pid = Int(CommandLine.arguments[1]) else {
    FileHandle.standardError.write(Data("usage: window-id <pid>\n".utf8))
    exit(2)
}
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
// The list is ordered front to back, so the first match is the frontmost.
for w in list where (w[kCGWindowOwnerPID as String] as? Int) == pid && (w[kCGWindowLayer as String] as? Int) == 0 {
    if let b = w[kCGWindowBounds as String] as? [String: Any],
        let h = b["Height"] as? Double, h < 50 { continue }
    print(w[kCGWindowNumber as String] as! Int)
    exit(0)
}
exit(1)
