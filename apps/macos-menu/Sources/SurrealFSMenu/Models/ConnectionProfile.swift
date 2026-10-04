import Foundation

public struct ConnectionProfile: Identifiable, Codable, Equatable, Sendable {
    public var id: UUID
    public var name: String
    public var endpoint: String
    public var namespace: String
    public var database: String
    public var username: String
    public var authLevel: AuthLevel
    public var isDefault: Bool
    
    public enum AuthLevel: String, Codable, Sendable {
        case record
        case system
    }
    
    public init(
        id: UUID = UUID(),
        name: String,
        endpoint: String,
        namespace: String = "surrealfs",
        database: String = "brain",
        username: String = "root",
        authLevel: AuthLevel = .record,
        isDefault: Bool = false
    ) {
        self.id = id
        self.name = name
        self.endpoint = endpoint
        self.namespace = namespace
        self.database = database
        self.username = username
        self.authLevel = authLevel
        self.isDefault = isDefault
    }
    
    public static let defaultProfiles: [ConnectionProfile] = [
        ConnectionProfile(
            name: "Production Cloud",
            endpoint: "wss://cloud.surreal.io",
            namespace: "brain",
            database: "production",
            username: "agent-1",
            authLevel: .record,
            isDefault: true
        ),
        ConnectionProfile(
            name: "Local Dev",
            endpoint: "ws://localhost:8000",
            namespace: "test",
            database: "test",
            username: "root",
            authLevel: .system,
            isDefault: false
        )
    ]
}
