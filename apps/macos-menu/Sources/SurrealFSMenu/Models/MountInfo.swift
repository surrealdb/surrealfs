import Foundation

public struct MountInfo: Codable, Equatable, Sendable {
    public var isMounted: Bool
    public var mountPath: String
    public var branch: String
    public var isClean: Bool
    public var activeLocksCount: Int
    
    public init(
        isMounted: Bool = true,
        mountPath: String = "~/mnt/surrealfs",
        branch: String = "main",
        isClean: Bool = true,
        activeLocksCount: Int = 0
    ) {
        self.isMounted = isMounted
        self.mountPath = mountPath
        self.branch = branch
        self.isClean = isClean
        self.activeLocksCount = activeLocksCount
    }
}
