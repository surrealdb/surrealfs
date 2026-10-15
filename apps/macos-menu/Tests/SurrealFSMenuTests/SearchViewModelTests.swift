import XCTest
@testable import SurrealFSMenu

@MainActor
final class SearchViewModelTests: XCTestCase {
    func testNavigationBounds() {
        let vm = SearchViewModel()
        vm.results = [
            SearchResult(path: "/docs/a.md", snippet: "hello"),
            SearchResult(path: "/docs/b.md", snippet: "world"),
            SearchResult(path: "/docs/c.md", snippet: "test")
        ]
        vm.selectedIndex = 0
        
        vm.selectNext()
        XCTAssertEqual(vm.selectedIndex, 1)
        
        vm.selectNext()
        XCTAssertEqual(vm.selectedIndex, 2)
        
        // Cannot exceed max index
        vm.selectNext()
        XCTAssertEqual(vm.selectedIndex, 2)
        
        vm.selectPrevious()
        XCTAssertEqual(vm.selectedIndex, 1)
        
        vm.selectPrevious()
        XCTAssertEqual(vm.selectedIndex, 0)
        
        // Cannot go below 0
        vm.selectPrevious()
        XCTAssertEqual(vm.selectedIndex, 0)
    }
    
    func testParseSearchResponseUnwrapsQueryEnvelope() {
        let envelope: [[String: Any]] = [
            [
                "status": "OK",
                "time": "120µs",
                "result": [
                    ["path": "/brain/risks.md", "snippet": "okta risk", "score": 0.95],
                    ["path": "/brain/roadmap.md", "snippet": "q4 plan", "score": 0.72]
                ]
            ]
        ]
        
        let parsed = SurrealClient.parseSearchResponse(envelope)
        XCTAssertEqual(parsed.count, 2)
        XCTAssertEqual(parsed[0].path, "/brain/risks.md")
        XCTAssertEqual(parsed[0].filename, "risks.md")
        XCTAssertEqual(parsed[0].snippet, "okta risk")
        XCTAssertEqual(parsed[0].score, 0.95)
        XCTAssertEqual(parsed[1].path, "/brain/roadmap.md")
    }
}
