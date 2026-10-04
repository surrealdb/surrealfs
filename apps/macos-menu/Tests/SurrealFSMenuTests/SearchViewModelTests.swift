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
}
