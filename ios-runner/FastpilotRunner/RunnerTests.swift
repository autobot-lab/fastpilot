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
    /// XCTest's own wait for the app to be idle (animations included) after every action is
    /// switched off unless FASTPILOT_QUIESCENCE=1; `idle` then waits for a stable screen instead.
    private var quiescence = true
    /// Device runs: the host's 128-bit token; the first request must be `hello{token}`.
    private var token: String?

    /// The quiescence wait is a private XCTest method; replace it with a no-op, as
    /// WebDriverAgent does. Both known selector shapes are handled; if neither exists, nothing changes.
    private func skipQuiescence() {
        guard let cls = NSClassFromString("XCUIApplicationProcess") else { return }
        let one: @convention(block) (AnyObject, Bool) -> Void = { _, _ in }
        let two: @convention(block) (AnyObject, Bool, Bool) -> Void = { _, _, _ in }
        if let m = class_getInstanceMethod(cls, NSSelectorFromString("waitForQuiescenceIncludingAnimationsIdle:")) {
            method_setImplementation(m, imp_implementationWithBlock(one))
            quiescence = false
        }
        if let m = class_getInstanceMethod(cls, NSSelectorFromString("waitForQuiescenceIncludingAnimationsIdle:isPreEvent:")) {
            method_setImplementation(m, imp_implementationWithBlock(two))
            quiescence = false
        }
    }

    /// Cheap fingerprint of the current screen: element types, frames, labels and values.
    private func fingerprint() -> Int {
        guard let root = try? current().snapshot() else { return 0 }
        var h = Hasher()
        var budget = 2000
        func walk(_ s: XCUIElementSnapshot) {
            if budget <= 0 { return }
            budget -= 1
            h.combine(s.elementType.rawValue)
            h.combine(Int(s.frame.minX)); h.combine(Int(s.frame.minY)); h.combine(Int(s.frame.width)); h.combine(Int(s.frame.height))
            h.combine(s.label)
            h.combine((s.value as? String) ?? "")
            s.children.forEach(walk)
        }
        walk(root)
        return h.finalize()
    }

    func testRunner() throws {
        continueAfterFailure = true
        let env = ProcessInfo.processInfo.environment
        let port = UInt16(env["FASTPILOT_PORT"] ?? "") ?? 22087
        // A runner outlives its session so the next one reuses it (no xcodebuild start); it
        // exits on its own after this long without a client.
        let idleExit = Double(env["FASTPILOT_IDLE_EXIT"] ?? "") ?? 300
        if env["FASTPILOT_QUIESCENCE"] != "1" { skipQuiescence() }
        token = env["FASTPILOT_TOKEN"].flatMap { $0.isEmpty ? nil : $0 }
        let any = env["FASTPILOT_BIND"] == "any"
        let server = try listen(port: port, any: any)
        print("FASTPILOT_RUNNER_READY port=\(port) version=1 quiescence=\(quiescence) bind=\(any ? "any" : "loopback") token=\(token == nil ? "off" : "on")")
        fflush(stdout)
        var lastClient = Date()
        while running {
            var pfd = pollfd(fd: server, events: Int16(POLLIN), revents: 0)
            if poll(&pfd, 1, 1000) <= 0 {
                if Date().timeIntervalSince(lastClient) > idleExit { break }
                continue
            }
            let client = accept(server, nil, nil)
            if client < 0 { continue }
            lastClient = Date()
            var one: Int32 = 1
            setsockopt(client, SOL_SOCKET, SO_NOSIGPIPE, &one, socklen_t(MemoryLayout<Int32>.size))
            setsockopt(client, IPPROTO_TCP, TCP_NODELAY, &one, socklen_t(MemoryLayout<Int32>.size))
            serve(client)
            close(client)
            lastClient = Date()
        }
        close(server)
        print("FASTPILOT_RUNNER_EXIT")
        fflush(stdout)
    }

    // MARK: socket + framing

    /// Loopback IPv4 (simulator: shares the Mac's network), or every interface dual-stack
    /// (device: reached over usbmux or the CoreDevice tunnel; guarded by the token).
    private func listen(port: UInt16, any: Bool) throws -> Int32 {
        let fd = socket(any ? AF_INET6 : AF_INET, SOCK_STREAM, 0)
        var one: Int32 = 1
        setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &one, socklen_t(MemoryLayout<Int32>.size))
        let rc: Int32
        if any {
            var zero: Int32 = 0
            setsockopt(fd, IPPROTO_IPV6, IPV6_V6ONLY, &zero, socklen_t(MemoryLayout<Int32>.size))
            var addr = sockaddr_in6()
            addr.sin6_len = UInt8(MemoryLayout<sockaddr_in6>.size)
            addr.sin6_family = sa_family_t(AF_INET6)
            addr.sin6_port = port.bigEndian
            addr.sin6_addr = in6addr_any
            rc = withUnsafePointer(to: &addr) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(fd, $0, socklen_t(MemoryLayout<sockaddr_in6>.size)) }
            }
        } else {
            var addr = sockaddr_in()
            addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
            addr.sin_family = sa_family_t(AF_INET)
            addr.sin_port = port.bigEndian
            addr.sin_addr.s_addr = inet_addr("127.0.0.1")
            rc = withUnsafePointer(to: &addr) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) }
            }
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
        var authenticated = token == nil
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
                if !authenticated {
                    // Anything but a correct hello first closes the connection.
                    guard req["cmd"] as? String == "hello", req["token"] as? String == token else {
                        _ = writeFrame(fd, ["id": id, "ok": false, "error": "unauthorized"])
                        return
                    }
                    authenticated = true
                }
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
            #if targetEnvironment(simulator)
            let kind = "simulator"
            #else
            let kind = "physical"
            #endif
            return ["version": "1", "width": b.width, "height": b.height, "scale": UIScreen.main.scale,
                    "device": kind, "osVersion": UIDevice.current.systemVersion]
        case "launch":
            appId = try str("bundleId")
            let a = XCUIApplication(bundleIdentifier: appId)
            // Locale / timezone for this launch (e.g. -AppleLanguages (id), TZ=Asia/Jakarta).
            a.launchArguments = req["args"] as? [String] ?? []
            a.launchEnvironment = req["env"] as? [String: String] ?? [:]
            a.launch() // terminates a running instance first
            app = a
            return [:]
        case "activate":
            appId = try str("bundleId")
            let a = XCUIApplication(bundleIdentifier: appId)
            a.activate()
            app = a
            return [:]
        case "resetPermissions":
            // Back to "not determined", so the app asks again (simulator and device).
            let a = XCUIApplication(bundleIdentifier: try str("bundleId"))
            let names = req["resources"] as? [String]
            var all: [(String, XCUIProtectedResource)] = [("contacts", .contacts), ("calendar", .calendar), ("reminders", .reminders),
                                                          ("photos", .photos), ("microphone", .microphone), ("camera", .camera),
                                                          ("mediaLibrary", .mediaLibrary), ("homeKit", .homeKit)]
            all.append(("bluetooth", .bluetooth))
            if #available(iOS 14.0, *) { all.append(("location", .location)) }
            var done: [String] = []
            for (name, r) in all where names == nil || names!.contains(name) {
                a.resetAuthorizationStatus(for: r)
                done.append(name)
            }
            return ["reset": done]
        case "openUrl":
            // Real devices have no simctl openurl; XCUIApplication.open needs iOS 16.4+.
            guard let url = URL(string: try str("url")) else { throw ProtocolError(message: "bad url") }
            if #available(iOS 16.4, *) {
                XCUIDevice.shared.system.open(url)
            } else {
                throw ProtocolError(message: "openUrl needs iOS 16.4 or later")
            }
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
            var image = XCUIScreen.main.screenshot().image
            var result: [String: Any] = [:]
            if let marks = req["marks"] as? [[String: Any]] {
                // Set-of-Mark: boxes in screen points; UIImage.size is in points too.
                image = UIGraphicsImageRenderer(size: image.size).image { ctx in
                    image.draw(at: .zero)
                    let red = UIColor(red: 0.93, green: 0, blue: 0, alpha: 1)
                    let font = UIFont.monospacedSystemFont(ofSize: 11, weight: .bold)
                    for m in marks {
                        let d = { (k: String) in (m[k] as? NSNumber)?.doubleValue ?? 0 }
                        let r = CGRect(x: d("x"), y: d("y"), width: d("w"), height: d("h"))
                        red.setStroke()
                        ctx.cgContext.setLineWidth(2)
                        ctx.cgContext.stroke(r)
                        let label = (m["r"] as? String ?? "") as NSString
                        let size = label.size(withAttributes: [.font: font])
                        let tag = CGRect(x: r.minX, y: r.minY >= 14 ? r.minY - 14 : r.minY, width: size.width + 4, height: 14)
                        red.setFill()
                        ctx.fill(tag)
                        label.draw(at: CGPoint(x: tag.minX + 2, y: tag.minY), withAttributes: [.font: font, .foregroundColor: UIColor.white])
                    }
                }
                result["marked"] = marks.count
            }
            guard let jpeg = image.jpegData(compressionQuality: min(max(q / 100, 0), 1)) else {
                throw ProtocolError(message: "jpeg encoding failed")
            }
            result["jpeg"] = jpeg.base64EncodedString()
            return result
        case "longPress":
            point(try num("x"), try num("y")).press(forDuration: max(try num("ms", 800), 100) / 1000)
            return [:]
        case "doubleTap":
            point(try num("x"), try num("y")).doubleTap()
            return [:]
        case "erase":
            let el = try focusedField()
            var existing = 0
            if let v = el.value as? String, v != (el.placeholderValue ?? "") { existing = v.count }
            let n = req["count"] is NSNumber ? min(Int(try num("count")), existing) : existing
            if n > 0 { el.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: n)) }
            return ["erased": n]
        case "terminate":
            XCUIApplication(bundleIdentifier: try str("bundleId")).terminate()
            return [:]
        case "hideKeyboard":
            return ["hidden": hideKeyboard()]
        case "idle":
            // With XCTest's own wait in place there is nothing to add; otherwise wait until two
            // snapshots 50 ms apart agree (bounded by timeoutMs).
            if quiescence { return ["idle": true] }
            let deadline = Date().addingTimeInterval(try num("timeoutMs", 1500) / 1000)
            var last = fingerprint()
            while Date() < deadline {
                Thread.sleep(forTimeInterval: 0.05)
                let now = fingerprint()
                if now == last { return ["idle": true] }
                last = now
            }
            return ["idle": false]
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
