import Foundation
import AppKit

public final class MountManager: @unchecked Sendable {
    public static let shared = MountManager()
    public let client = SurrealClient()
    
    private var mountProcess: Process?
    private let lock = NSLock()
    
    public init() {}
    
    public func checkMountStatus(path: String) -> MountInfo {
        let expanded = (path as NSString).expandingTildeInPath
        var isDir: ObjCBool = false
        let exists = FileManager.default.fileExists(atPath: expanded, isDirectory: &isDir)
        let running = lock.withLock { mountProcess?.isRunning ?? false }
        
        return MountInfo(
            isMounted: running || (exists && isDir.boolValue),
            mountPath: path,
            branch: "main",
            isClean: true,
            activeLocksCount: 0
        )
    }
    
    public func openInFinder(path: String) {
        let expanded = (path as NSString).expandingTildeInPath
        let url = URL(fileURLWithPath: expanded)
        NSWorkspace.shared.open(url)
    }
    
    public static func terminalAppleScript(forPath path: String) -> String {
        let expanded = (path as NSString).expandingTildeInPath
        let escaped = expanded.replacingOccurrences(of: "'", with: "'\\''")
        return "tell application \"Terminal\" to do script \"cd '\(escaped)'\""
    }
    
    public func openInTerminal(path: String) {
        let script = Self.terminalAppleScript(forPath: path)
        if let appleScript = NSAppleScript(source: script) {
            var error: NSDictionary?
            appleScript.executeAndReturnError(&error)
        }
    }
    
    public func toggleMount(path: String, isMounted: inout Bool) {
        lock.lock()
        defer { lock.unlock() }
        
        let expanded = (path as NSString).expandingTildeInPath
        if isMounted {
            // Unmount
            mountProcess?.terminate()
            mountProcess = nil
            isMounted = false
        } else {
            // Mount
            try? FileManager.default.createDirectory(atPath: expanded, withIntermediateDirectories: true)
            if let binaryPath = findSurrealFsBinary() {
                let proc = Process()
                proc.executableURL = URL(fileURLWithPath: binaryPath)
                proc.arguments = ["mount", expanded]
                try? proc.run()
                mountProcess = proc
            }
            isMounted = true
        }
    }
    
    private func findSurrealFsBinary() -> String? {
        let candidates = [
            "/usr/local/bin/surrealfs-rs",
            "/opt/homebrew/bin/surrealfs-rs",
            (NSHomeDirectory() as NSString).appendingPathComponent(".cargo/bin/surrealfs-rs")
        ]
        return candidates.first { FileManager.default.isExecutableFile(atPath: $0) }
    }
}
