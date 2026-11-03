import XCTest
import UIKit

// fastpilot iOS runner: one never-ending UI test serving the wire protocol in ../PROTOCOL.md.
// Everything runs on the main thread (XCUI APIs are main-actor), one client at a time, blocking sockets.

struct ProtocolError: Error { let message: String }

@MainActor
final class RunnerTests: XCTestCase {
    private var app: XCUIApplication?
    private var appId = "com.apple.springboard"
    private var running = true

    func testRunner() throws {
        continueAfterFailure = true
        let port = UInt16(ProcessInfo.processInfo.environment["FASTPILOT_PORT"] ?? "") ?? 22087
        let server = try listen(port: port)
        print("FASTPILOT_RUNNER_READY port=\(port) version=1")
        fflush(stdout)
        while running {
            let client = accept(server, nil, nil)
            if client < 0 { continue }
            var one: Int32 = 1
            setsockopt(client, SOL_SOCKET, SO_NOSIGPIPE, &one, socklen_t(MemoryLayout<Int32>.size))
            setsockopt(client, IPPROTO_TCP, TCP_NODELAY, &one, socklen_t(MemoryLayout<Int32>.size))
            serve(client)
            close(client)
        }
        close(server)
        print("FASTPILOT_RUNNER_EXIT")
        fflush(stdout)
    }

    // MARK: socket + framing

    private func listen(port: UInt16) throws -> Int32 {
        let fd = socket(AF_INET, SOCK_STREAM, 0)
        var one: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &one, socklen_t(MemoryLayout<Int32>.size))
        var addr = sockaddr_in()
        addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = port.bigEndian
        addr.sin_addr.s_addr = inet_addr("127.0.0.1")
        let rc = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) }
        }
        guard fd >= 0, rc == 0, Darwin.listen(fd, 4) == 0 else {
            print("FASTPILOT_RUNNER_ERROR bind 127.0.0.1:\(port) failed errno=\(errno)")
            fflush(stdout)
            throw ProtocolError(message: "bind failed")
        }
        return fd
    }

    private func readFully(_ fd: Int32, _ n: Int) -> Data? {
        var buf = Data(count: n)
        var got = 0
        while got < n {
            let r = buf.withUnsafeMutableBytes { read(fd, $0.baseAddress! + got, n - got) }
            if r <= 0 { return nil }
            got += r
        }
        return buf
    }

    private func writeFrame(_ fd: Int32, _ obj: [String: Any]) -> Bool {
        guard let body = try? JSONSerialization.data(withJSONObject: obj) else { return false }
        var frame = Data(count: 4)
        let len = UInt32(body.count).bigEndian
        withUnsafeBytes(of: len) { frame.replaceSubrange(0..<4, with: $0) }
        frame.append(body)
        return frame.withUnsafeBytes { p -> Bool in
            var off = 0
            while off < frame.count {
                let w = write(fd, p.baseAddress! + off, frame.count - off)
                if w <= 0 { return false }
                off += w
            }
            return true
        }
    }

    private func serve(_ fd: Int32) {
        while running {
            guard let head = readFully(fd, 4) else { return }
            let len = head.reduce(0) { $0 << 8 | Int($1) }
            guard len <= 16 << 20, let body = readFully(fd, len) else { return }
            var resp: [String: Any]
            var id: Any = NSNull()
            do {
                guard let req = (try? JSONSerialization.jsonObject(with: body)) as? [String: Any] else {
                    throw ProtocolError(message: "request is not a JSON object")
                }
                id = req["id"] ?? NSNull()
                resp = try handle(req)
                resp["ok"] = true
            } catch let e as ProtocolError {
                resp = ["ok": false, "error": e.message]
            } catch {
                resp = ["ok": false, "error": "\(error)"]
            }
            resp["id"] = id
            if !writeFrame(fd, resp) { return }
        }
    }

    // MARK: commands

    private func handle(_ req: [String: Any]) throws -> [String: Any] {
        let cmd = req["cmd"] as? String ?? ""
        func str(_ k: String) throws -> String {
            guard let v = req[k] as? String else { throw ProtocolError(message: "missing string arg '\(k)'") }
            return v
        }
        func num(_ k: String, _ def: Double? = nil) throws -> Double {
            if let v = req[k] as? NSNumber { return v.doubleValue }
            if let d = def { return d }
            throw ProtocolError(message: "missing number arg '\(k)'")
        }
        switch cmd {
        case "hello":
            // UIScreen.main in the (letterboxed) runner process reports 320x480; SpringBoard's frame is the real screen.
            let b = XCUIApplication(bundleIdentifier: "com.apple.springboard").frame
            return ["version": "1", "width": b.width, "height": b.height, "scale": UIScreen.main.scale]
        case "launch":
            appId = try str("bundleId")
            let a = XCUIApplication(bundleIdentifier: appId)
            a.launch() // terminates a running instance first
            app = a
            return [:]
        case "activate":
            appId = try str("bundleId")
            let a = XCUIApplication(bundleIdentifier: appId)
            a.activate()
            app = a
            return [:]
        case "snapshot":
            return try snapshot()
        case "tap":
            point(try num("x"), try num("y")).tap()
            return [:]
        case "swipe":
            let from = point(try num("x1"), try num("y1")), to = point(try num("x2"), try num("y2"))
            let dist = hypot(try num("x2") - num("x1"), try num("y2") - num("y1"))
            let secs = max(try num("ms", 300), 50) / 1000
            from.press(forDuration: 0.05, thenDragTo: to,
                       withVelocity: XCUIGestureVelocity(rawValue: CGFloat(max(dist / secs, 10))), thenHoldForDuration: 0)
            return [:]
        case "text":
            let el = try focusedField()
            var existing = 0
            if let v = el.value as? String, v != (el.placeholderValue ?? "") { existing = v.count }
            el.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: existing) + (try str("text")))
            return [:]
        case "key":
            let keys: [String: XCUIKeyboardKey] = ["return": .return, "enter": .enter, "delete": .delete,
                                                   "tab": .tab, "escape": .escape, "space": .space]
            let name = try str("name")
            guard let k = keys[name.lowercased()] else { throw ProtocolError(message: "unknown key '\(name)'") }
            try focusedField().typeText(k.rawValue)
            return [:]
        case "back":
            let b = current().navigationBars.firstMatch.buttons.firstMatch
            guard b.exists else { throw ProtocolError(message: "no back button") }
            b.tap()
            return [:]
        case "screenshot":
            let q = try num("quality", 80)
            guard let jpeg = XCUIScreen.main.screenshot().image.jpegData(compressionQuality: min(max(q / 100, 0), 1)) else {
                throw ProtocolError(message: "jpeg encoding failed")
            }
            return ["jpeg": jpeg.base64EncodedString()]
        case "hideKeyboard":
            return ["hidden": hideKeyboard()]
        case "idle":
            // XCUI waits for app quiescence after every action; nothing extra to do.
            return ["idle": true]
        case "quit":
            running = false
            return [:]
        default:
            throw ProtocolError(message: "unknown cmd '\(cmd)'")
        }
    }

    /// Keyboard dismissal without a universal API: an accessory "Hide keyboard"/"Done" button,
    /// else a downward drag just above the keyboard (scroll views dismiss on drag), else Return.
    private func hideKeyboard() -> Bool {
        let a = current()
        let kb = a.keyboards.firstMatch
        guard kb.exists else { return false }
        for name in ["Hide keyboard", "Done", "Selesai"] {
            let b = a.buttons[name]
            if b.exists && b.isHittable {
                b.tap()
                return !kb.exists
            }
        }
        let top = kb.frame.minY
        point(a.frame.midX, top - 40).press(forDuration: 0.05, thenDragTo: point(a.frame.midX, top + 80))
        if kb.exists, let f = try? focusedField() {
            f.typeText("\n")
        }
        return !kb.exists
    }

    private func current() -> XCUIApplication {
        app ?? XCUIApplication(bundleIdentifier: "com.apple.springboard")
    }

    /// Screen-point coordinate. The app element's frame is the full screen, so offsets are absolute.
    private func point(_ x: Double, _ y: Double) -> XCUICoordinate {
        current().coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: x, dy: y))
    }

    private func focusedField() throws -> XCUIElement {
        let el = current().descendants(matching: .any).element(matching: NSPredicate(format: "hasKeyboardFocus == true"))
        guard el.exists else { throw ProtocolError(message: "no focused text field") }
        return el
    }

    // MARK: snapshot

    private func snapshot() throws -> [String: Any] {
        let a = current()
        // A crashed app is `notRunning`: report it instead of snapshotting nothing.
        if appId != "com.apple.springboard" && a.state == .notRunning {
            return ["bundleId": appId, "title": "", "state": Int(a.state.rawValue), "nodes": []]
        }
        let root = try a.snapshot()
        let screen = root.frame // the application element spans the screen
        var title = ""
        func find(_ s: XCUIElementSnapshot) -> Bool {
            if s.elementType == .navigationBar {
                title = s.identifier.isEmpty ? s.label : s.identifier
                return true
            }
            return s.children.contains(where: find)
        }
        _ = find(root)
        var budget = 2000
        var nodes = convert(root, screen, 0, &budget)
        // Permission prompts and other system alerts belong to SpringBoard, not the app.
        if appId != "com.apple.springboard" {
            let alert = XCUIApplication(bundleIdentifier: "com.apple.springboard").alerts.firstMatch
            if alert.exists, let snap = try? alert.snapshot() {
                var b = 200
                for var n in convert(snap, screen, 0, &b) {
                    n["system"] = true
                    nodes.append(n)
                }
            }
        }
        return ["bundleId": appId,
                "title": title,
                "state": Int(a.state.rawValue),
                "nodes": nodes]
    }

    /// Returns the node, or (for an offscreen/zero-size node) its visible children promoted in its place.
    /// Bounded (depth 60, 2000 nodes): pathological trees must not make snapshots unbounded.
    private func convert(_ s: XCUIElementSnapshot, _ screen: CGRect, _ depth: Int, _ budget: inout Int) -> [[String: Any]] {
        if depth > 60 || budget <= 0 { return [] }
        budget -= 1
        var kids: [[String: Any]] = []
        for c in s.children { kids += convert(c, screen, depth + 1, &budget) }
        let f = s.frame
        if f.width <= 0 || f.height <= 0 || !f.intersects(screen) { return kids }
        let type = s.elementType
        var n: [String: Any] = ["type": typeName(type), "frame": [f.minX, f.minY, f.width, f.height]]
        if !s.label.isEmpty { n["label"] = s.label }
        if !s.identifier.isEmpty { n["id"] = s.identifier }
        if type != .secureTextField, let v = s.value {
            let sv = (v as? String) ?? "\(v)"
            if !sv.isEmpty && sv != s.placeholderValue { n["value"] = sv }
        }
        if let p = s.placeholderValue, !p.isEmpty { n["placeholder"] = p }
        if s.isEnabled { n["enabled"] = true }
        if s.isSelected { n["selected"] = true }
        if keyboardFocus(s) { n["focused"] = true }
        if !kids.isEmpty { n["children"] = kids }
        return [n]
    }

    private func keyboardFocus(_ s: XCUIElementSnapshot) -> Bool {
        // XCElementSnapshot (the concrete class) exposes hasKeyboardFocus; public hasFocus is the tvOS focus engine.
        if let o = s as? NSObject, o.responds(to: NSSelectorFromString("hasKeyboardFocus")),
           let v = o.value(forKey: "hasKeyboardFocus") as? Bool {
            return v
        }
        return s.hasFocus
    }

    private static let typeNames = [
        "any", "other", "application", "group", "window", "sheet", "drawer", "alert", "dialog", "button",
        "radioButton", "radioGroup", "checkBox", "disclosureTriangle", "popUpButton", "comboBox", "menuButton",
        "toolbarButton", "popover", "keyboard", "key", "navigationBar", "tabBar", "tabGroup", "toolbar", "statusBar",
        "table", "tableRow", "tableColumn", "outline", "outlineRow", "browser", "collectionView", "slider",
        "pageIndicator", "progressIndicator", "activityIndicator", "segmentedControl", "picker", "pickerWheel",
        "switch", "toggle", "link", "image", "icon", "searchField", "scrollView", "scrollBar", "staticText",
        "textField", "secureTextField", "datePicker", "textView", "menu", "menuItem", "menuBar", "menuBarItem",
        "map", "webView", "incrementArrow", "decrementArrow", "timeline", "ratingIndicator", "valueIndicator",
        "splitGroup", "splitter", "relevanceIndicator", "colorWell", "helpTag", "matte", "dockItem", "ruler",
        "rulerMarker", "grid", "levelIndicator", "cell", "layoutArea", "layoutItem", "handle", "stepper", "tab",
        "touchBar", "statusItem",
    ]

    private func typeName(_ t: XCUIElement.ElementType) -> String {
        let i = Int(t.rawValue)
        return i < Self.typeNames.count ? Self.typeNames[i] : "type\(i)"
    }
}
