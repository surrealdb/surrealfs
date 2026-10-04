import Foundation
import SwiftUI
import Combine

@MainActor
public final class SearchViewModel: ObservableObject {
    @Published public var query: String = ""
    @Published public var results: [SearchResult] = []
    @Published public var isSearching: Bool = false
    @Published public var selectedIndex: Int = 0
    @Published public var errorMessage: String? = nil
    
    private let client: SurrealClient
    private var searchCancellable: AnyCancellable?
    
    public init(client: SurrealClient = SurrealClient()) {
        self.client = client
        
        $query
            .debounce(for: .milliseconds(250), scheduler: RunLoop.main)
            .removeDuplicates()
            .sink { [weak self] q in
                Task {
                    await self?.performSearch(q)
                }
            }
            .store(in: &cancellables)
    }
    
    private var cancellables = Set<AnyCancellable>()
    
    public func performSearch(_ text: String) async {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            results = []
            isSearching = false
            return
        }
        
        isSearching = true
        errorMessage = nil
        
        do {
            let res = try await client.search(query: trimmed, limit: 15)
            self.results = res
            self.selectedIndex = 0
            self.isSearching = false
        } catch {
            self.results = []
            self.errorMessage = error.localizedDescription
            self.isSearching = false
        }
    }
    
    public func selectNext() {
        guard !results.isEmpty else { return }
        selectedIndex = min(results.count - 1, selectedIndex + 1)
    }
    
    public func selectPrevious() {
        guard !results.isEmpty else { return }
        selectedIndex = max(0, selectedIndex - 1)
    }
    
    public func openSelected(mountPath: String = "~/mnt/surrealfs") {
        guard !results.isEmpty, selectedIndex < results.count else { return }
        let item = results[selectedIndex]
        let expanded = (mountPath as NSString).expandingTildeInPath
        let filePath = (expanded as NSString).appendingPathComponent(item.path)
        let url = URL(fileURLWithPath: filePath)
        NSWorkspace.shared.open(url)
    }
}
