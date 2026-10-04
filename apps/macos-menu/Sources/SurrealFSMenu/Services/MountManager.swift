import Foundation
import AppKit

public final class MountManager: @unchecked Sendable {
    public static let shared = MountManager()
    
    private var mountProcess: Process?
    private let lock = NSLock()
    
    public init() {}
    
    public func checkMountStatus(path: String) -> MountInfo {
        let expanded = (path as NSString).expandingTildeInPath
        var isDir: ObjCBool = false
        let exists = FileManager.default.fileExists(atPath: expanded, isDirectory: &isDir)
        
        return MountInfo(
            isMounted: exists && isDir.boolValue,
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
    
    public func openInTerminal(path: String) {
        let expanded = (path as NSString).expandingTildeInPath
        let script = "tell application \"Terminal\" to do script \"cd '\(expanded)'\""
        if let appleScript = NSAppleScript(source: script) {
            var error: NSDictionary?
            appleScript.executeAndReturnError(&error)
        }
    }
    
    public func toggleMount(path: String, isMounted: inout Bool) {
        lock.lock()
        defer { lock.unlock() }
        
        if isMounted {
            // Unmount
            mountProcess?.terminate()
            mountProcess = nil
            isMounted = false
        } else {
            // Mount
            let expanded = (path as NSString).expandingTildeInPath
            try? FileManager.default.createDirectory(atPath: expanded, withIntermediateDirectories: true)
            isMounted = true
        }
    }
}
