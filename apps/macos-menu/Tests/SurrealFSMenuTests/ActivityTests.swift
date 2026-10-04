import XCTest
@testable import SurrealFSMenu

final class ActivityTests: XCTestCase {
    func testFormattedActivityText() {
        let act1 = AgentActivity(
            icon: "🤖",
            actor: "hermes",
            action: .write,
            path: "/brain/acme/risks/okta.md"
        )
        XCTAssertEqual(act1.formattedText, "🤖 hermes wrote /brain/acme/risks/okta.md")
        
        let act2 = AgentActivity(
            icon: "👤",
            actor: "martin",
            action: .lock,
            path: "/projects/auth/",
            details: "32s left",
            canUndo: false
        )
        XCTAssertEqual(act2.formattedText, "👤 martin locked /projects/auth/ (32s left)")
    }
    
    func testActivityCodableRoundtrip() throws {
        let activity = AgentActivity(
            icon: "🤖",
            actor: "triage-bot",
            action: .append,
            path: "/incidents/log.md",
            generation: 5,
            canUndo: true
        )
        
        let data = try JSONEncoder().encode(activity)
        let decoded = try JSONDecoder().decode(AgentActivity.self, from: data)
        
        XCTAssertEqual(activity.id, decoded.id)
        XCTAssertEqual(activity.actor, decoded.actor)
        XCTAssertEqual(activity.action, decoded.action)
        XCTAssertEqual(activity.path, decoded.path)
        XCTAssertEqual(activity.generation, decoded.generation)
        XCTAssertEqual(activity.canUndo, decoded.canUndo)
    }
}
