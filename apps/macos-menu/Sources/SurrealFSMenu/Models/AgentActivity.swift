import Foundation

public struct AgentActivity: Identifiable, Codable, Equatable, Sendable {
    public var id: UUID
    public var icon: String
    public var actor: String
    public var action: ActionType
    public var path: String
    public var details: String?
    public var timestamp: Date
    public var generation: Int?
    public var canUndo: Bool
    
    public enum ActionType: String, Codable, Sendable {
        case write = "wrote"
        case append = "appended"
        case lock = "locked"
        case unlock = "unlocked"
        case delete = "deleted"
        case fork = "forked"
        case merge = "merged"
        case read = "read"
    }
    
    public init(
        id: UUID = UUID(),
        icon: String = "🤖",
        actor: String,
        action: ActionType,
        path: String,
        details: String? = nil,
        timestamp: Date = Date(),
        generation: Int? = nil,
        canUndo: Bool = true
    ) {
        self.id = id
        self.icon = icon
        self.actor = actor
        self.action = action
        self.path = path
        self.details = details
        self.timestamp = timestamp
        self.generation = generation
        self.canUndo = canUndo
    }
    
    public var formattedText: String {
        var base = "\(icon) \(actor) \(action.rawValue) \(path)"
        if let details = details, !details.isEmpty {
            base += " (\(details))"
        }
        return base
    }
}
