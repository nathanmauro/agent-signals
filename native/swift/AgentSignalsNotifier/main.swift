import AppKit
import Foundation
import UserNotifications
import Darwin

private let stateDir = ProcessInfo.processInfo.environment["AGENT_SIGNALS_STATE_DIR"]
    ?? "\(NSHomeDirectory())/.local/state/agent-signals"
private let socketPath = "\(stateDir)/agent-signald.sock"

final class DaemonConnection {
    private let socketPath: String
    private let lock = NSLock()
    private var fd: Int32 = -1
    private var pendingAuthorization = "unknown"

    init(socketPath: String) {
        self.socketPath = socketPath
    }

    func start(authorization: String) {
        pendingAuthorization = authorization
        DispatchQueue.global(qos: .utility).async {
            self.connectLoop()
        }
    }

    func sendClicked(id: String) {
        sendJSON(["type": "notification_clicked", "id": id])
    }

    private func connectLoop() {
        while true {
            autoreleasepool {
                if self.connectOnce() {
                    self.readLoop()
                    self.closeSocket()
                }
            }
            Thread.sleep(forTimeInterval: 2.0)
        }
    }

    private func connectOnce() -> Bool {
        let nextFd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard nextFd >= 0 else {
            return false
        }

        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let maxPathLength = MemoryLayout.size(ofValue: addr.sun_path)
        var pathBytes = Array(socketPath.utf8)
        if pathBytes.count >= maxPathLength {
            pathBytes = Array(pathBytes.prefix(maxPathLength - 1))
        }

        withUnsafeMutablePointer(to: &addr.sun_path) { ptr in
            ptr.withMemoryRebound(to: CChar.self, capacity: maxPathLength) { dest in
                for idx in 0..<maxPathLength {
                    dest[idx] = 0
                }
                for (idx, byte) in pathBytes.enumerated() {
                    dest[idx] = CChar(bitPattern: byte)
                }
            }
        }

        let result = withUnsafePointer(to: &addr) { ptr -> Int32 in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockaddrPtr in
                Darwin.connect(nextFd, sockaddrPtr, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }

        guard result == 0 else {
            Darwin.close(nextFd)
            return false
        }

        lock.lock()
        fd = nextFd
        lock.unlock()

        sendJSON([
            "type": "notifier_hello",
            "pid": Int(ProcessInfo.processInfo.processIdentifier),
            "authorization": pendingAuthorization,
        ])
        return true
    }

    private func readLoop() {
        var buffer = [UInt8](repeating: 0, count: 8192)
        var pending = Data()

        while true {
            let currentFd = lockedFd()
            if currentFd < 0 {
                return
            }
            let count = Darwin.read(currentFd, &buffer, buffer.count)
            if count <= 0 {
                return
            }
            pending.append(buffer, count: count)
            while let newline = pending.firstIndex(of: 10) {
                let line = pending[..<newline]
                pending.removeSubrange(...newline)
                handleLine(Data(line))
            }
        }
    }

    private func handleLine(_ data: Data) {
        guard !data.isEmpty else {
            return
        }
        guard
            let object = try? JSONSerialization.jsonObject(with: data),
            let message = object as? [String: Any],
            let type = message["type"] as? String
        else {
            return
        }

        if type == "post_notification" {
            NotificationPoster.shared.post(message)
        } else if type == "clear_notifications" {
            NotificationPoster.shared.clearDelivered()
        }
    }

    private func sendJSON(_ object: [String: Any]) {
        guard
            JSONSerialization.isValidJSONObject(object),
            let data = try? JSONSerialization.data(withJSONObject: object)
        else {
            return
        }
        var line = data
        line.append(10)

        lock.lock()
        let currentFd = fd
        lock.unlock()
        guard currentFd >= 0 else {
            return
        }

        line.withUnsafeBytes { rawBuffer in
            guard let base = rawBuffer.baseAddress else {
                return
            }
            var written = 0
            while written < line.count {
                let result = Darwin.write(currentFd, base.advanced(by: written), line.count - written)
                if result <= 0 {
                    break
                }
                written += result
            }
        }
    }

    private func lockedFd() -> Int32 {
        lock.lock()
        let current = fd
        lock.unlock()
        return current
    }

    private func closeSocket() {
        lock.lock()
        let current = fd
        fd = -1
        lock.unlock()
        if current >= 0 {
            Darwin.close(current)
        }
    }
}

final class NotificationPoster: NSObject, UNUserNotificationCenterDelegate {
    static let shared = NotificationPoster()

    private var connection: DaemonConnection?

    func configure(connection: DaemonConnection) {
        self.connection = connection
        UNUserNotificationCenter.current().delegate = self
    }

    func post(_ message: [String: Any]) {
        guard let id = message["id"] as? String else {
            return
        }
        let content = UNMutableNotificationContent()
        content.title = message["title"] as? String ?? "Agent responded"
        content.subtitle = message["subtitle"] as? String ?? ""
        content.body = message["message"] as? String ?? "Prompt response is ready."
        content.threadIdentifier = message["group"] as? String ?? "agent-signals"
        content.userInfo = ["id": id]

        if (message["ignore_dnd"] as? Bool) == true {
            if #available(macOS 12.0, *) {
                content.interruptionLevel = .timeSensitive
            }
        }

        let request = UNNotificationRequest(identifier: id, content: content, trigger: nil)
        UNUserNotificationCenter.current().add(request) { error in
            if let error {
                NSLog("AgentSignalsNotifier post failed: \(error.localizedDescription)")
            }
        }
    }

    func clearDelivered() {
        let center = UNUserNotificationCenter.current()
        center.removeAllDeliveredNotifications()
        center.removeAllPendingNotificationRequests()
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        let id = response.notification.request.identifier
        connection?.sendClicked(id: id)
        completionHandler()
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        if #available(macOS 11.0, *) {
            completionHandler([.banner, .list, .sound])
        } else {
            completionHandler([.alert, .sound])
        }
    }
}

func authorizationName(_ status: UNAuthorizationStatus) -> String {
    switch status {
    case .notDetermined:
        return "not_determined"
    case .denied:
        return "denied"
    case .authorized:
        return "authorized"
    case .provisional:
        return "provisional"
    case .ephemeral:
        return "ephemeral"
    @unknown default:
        return "unknown"
    }
}

func start() {
    let center = UNUserNotificationCenter.current()
    let connection = DaemonConnection(socketPath: socketPath)
    NotificationPoster.shared.configure(connection: connection)

    center.getNotificationSettings { settings in
        if settings.authorizationStatus == .notDetermined {
            center.requestAuthorization(options: [.alert, .sound]) { granted, _ in
                connection.start(authorization: granted ? "authorized" : "denied")
            }
        } else {
            connection.start(authorization: authorizationName(settings.authorizationStatus))
        }
    }
}

// A bare Foundation RunLoop cannot answer the activation Apple Event macOS
// sends when a notification is clicked, so LaunchServices spawns a duplicate
// instance and the launch watchdog times out ("application is not responding").
// Running a real AppKit event loop as an .accessory (no Dock icon, paired with
// LSUIElement) lets the live instance handle the click and deliver
// UNUserNotificationCenterDelegate callbacks in-process.
final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        start()
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let delegate = AppDelegate()
app.delegate = delegate
app.run()
