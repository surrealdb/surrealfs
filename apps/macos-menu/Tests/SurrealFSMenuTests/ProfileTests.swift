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
    
    func testProfileKeychainIsolation() {
        let p1 = ConnectionProfile(name: "Server A", endpoint: "ws://a:8000", username: "root")
        let p2 = ConnectionProfile(name: "Server B", endpoint: "ws://b:8000", username: "root")
        let keychain = KeychainService()
        
        _ = keychain.saveSecret("secret-a", for: p1)
        _ = keychain.saveSecret("secret-b", for: p2)
        
        XCTAssertEqual(keychain.getSecret(for: p1), "secret-a")
        XCTAssertEqual(keychain.getSecret(for: p2), "secret-b")
        
        _ = keychain.deleteSecret(for: p1)
        _ = keychain.deleteSecret(for: p2)
    }
    
    func testTerminalAppleScriptEscapesSingleQuotes() {
        let script = MountManager.terminalAppleScript(forPath: "/tmp/O'Reilly/mnt")
        XCTAssertEqual(script, "tell application \"Terminal\" to do script \"cd '/tmp/O'\\''Reilly/mnt'\"")
    }
}
