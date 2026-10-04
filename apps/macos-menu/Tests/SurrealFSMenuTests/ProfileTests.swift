import XCTest
@testable import SurrealFSMenu

final class ProfileTests: XCTestCase {
    func testDefaultProfiles() {
        let profiles = ConnectionProfile.defaultProfiles
        XCTAssertEqual(profiles.count, 2)
        XCTAssertTrue(profiles.contains(where: { $0.name == "Production Cloud" && $0.authLevel == .record }))
        XCTAssertTrue(profiles.contains(where: { $0.name == "Local Dev" && $0.authLevel == .system }))
    }
    
    func testProfileCodableRoundtrip() throws {
        let profile = ConnectionProfile(
            name: "Test Server",
            endpoint: "ws://127.0.0.1:8000",
            namespace: "testns",
            database: "testdb",
            username: "worker",
            authLevel: .record
        )
        
        let encoder = JSONEncoder()
        let data = try encoder.encode(profile)
        
        let decoder = JSONDecoder()
        let decoded = try decoder.decode(ConnectionProfile.self, from: data)
        
        XCTAssertEqual(profile.id, decoded.id)
        XCTAssertEqual(profile.name, decoded.name)
        XCTAssertEqual(profile.endpoint, decoded.endpoint)
        XCTAssertEqual(profile.namespace, decoded.namespace)
        XCTAssertEqual(profile.database, decoded.database)
        XCTAssertEqual(profile.username, decoded.username)
        XCTAssertEqual(profile.authLevel, decoded.authLevel)
    }
}
