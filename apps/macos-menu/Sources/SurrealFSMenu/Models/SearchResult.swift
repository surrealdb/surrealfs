import Foundation

public struct SearchResult: Identifiable, Codable, Equatable, Sendable {
    public var id: UUID
    public var path: String
    public var filename: String
    public var snippet: String
    public var score: Double
    public var lineNumber: Int?
    public var matchType: String
    
    public init(
        id: UUID = UUID(),
        path: String,
        filename: String? = nil,
        snippet: String = "",
        score: Double = 0.0,
        lineNumber: Int? = nil,
        matchType: String = "fulltext"
    ) {
        self.id = id
        self.path = path
        self.filename = filename ?? (path as NSString).lastPathComponent
        self.snippet = snippet
        self.score = score
        self.lineNumber = lineNumber
        self.matchType = matchType
    }
}
