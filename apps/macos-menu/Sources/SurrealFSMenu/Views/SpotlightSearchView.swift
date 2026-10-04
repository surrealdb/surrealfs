import SwiftUI
import AppKit

@MainActor
public struct SpotlightSearchView: View {
    @StateObject public var viewModel: SearchViewModel
    public var mountPath: String
    public var onClose: () -> Void
    
    public init(
        viewModel: SearchViewModel? = nil,
        mountPath: String = "~/mnt/surrealfs",
        onClose: @escaping () -> Void = {}
    ) {
        self._viewModel = StateObject(wrappedValue: viewModel ?? SearchViewModel())
        self.mountPath = mountPath
        self.onClose = onClose
    }
    
    public var body: some View {
        VStack(spacing: 0) {
            // Search Input Header
            HStack(spacing: 12) {
                Image(systemName: "magnifyingglass")
                    .font(.title2)
                    .foregroundColor(.secondary)
                
                TextField("Search agent brain with fulltext or vectors (⌘⇧S)...", text: $viewModel.query)
                    .textFieldStyle(.plain)
                    .font(.title3)
                
                if viewModel.isSearching {
                    ProgressView()
                        .scaleEffect(0.7)
                        .frame(width: 20, height: 20)
                } else if !viewModel.query.isEmpty {
                    Button(action: { viewModel.query = "" }) {
                        Image(systemName: "xmark.circle.fill")
                            .foregroundColor(.secondary)
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 14)
            .background(Color(NSColor.windowBackgroundColor))
            
            Divider()
            
            // Search Results List
            if viewModel.results.isEmpty {
                VStack(spacing: 8) {
                    if !viewModel.query.isEmpty && !viewModel.isSearching {
                        Text("No matches found for '\(viewModel.query)'")
                            .font(.callout)
                            .foregroundColor(.secondary)
                    } else {
                        Text("Type to search across documents, code, notes, and memory")
                            .font(.callout)
                            .foregroundColor(.secondary)
                    }
                }
                .frame(maxWidth: .infinity, minHeight: 180)
            } else {
                ScrollView {
                    LazyVStack(spacing: 2) {
                        ForEach(Array(viewModel.results.enumerated()), id: \.element.id) { index, result in
                            SearchResultRow(
                                result: result,
                                isSelected: index == viewModel.selectedIndex
                            ) {
                                viewModel.selectedIndex = index
                                viewModel.openSelected(mountPath: mountPath)
                                onClose()
                            }
                        }
                    }
                    .padding(.vertical, 8)
                }
                .frame(maxHeight: 320)
            }
            
            Divider()
            
            // Footer with shortcuts
            HStack(spacing: 16) {
                Text("↑↓ to navigate")
                Text("↵ to open")
                Text("esc to dismiss")
                Spacer()
                Text("\(viewModel.results.count) results")
            }
            .font(.caption2)
            .foregroundColor(.secondary)
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .background(Color(NSColor.controlBackgroundColor))
        }
        .frame(width: 580)
        .background(VisualEffectBlur())
        .cornerRadius(12)
        .shadow(color: Color.black.opacity(0.3), radius: 20, x: 0, y: 10)
    }
}

struct SearchResultRow: View {
    let result: SearchResult
    let isSelected: Bool
    let onSelect: () -> Void
    
    var body: some View {
        Button(action: onSelect) {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: iconForPath(result.path))
                    .font(.body)
                    .foregroundColor(isSelected ? .white : .accentColor)
                    .frame(width: 20, alignment: .center)
                    .padding(.top, 2)
                
                VStack(alignment: .leading, spacing: 3) {
                    HStack {
                        Text(result.filename)
                            .font(.callout)
                            .fontWeight(.medium)
                            .foregroundColor(isSelected ? .white : .primary)
                        
                        Text(result.path)
                            .font(.caption2)
                            .foregroundColor(isSelected ? .white.opacity(0.8) : .secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                        
                        Spacer()
                        
                        if result.score > 0 {
                            Text(String(format: "%.2f", result.score))
                                .font(.system(size: 9, weight: .bold, design: .monospaced))
                                .padding(.horizontal, 4)
                                .padding(.vertical, 1)
                                .background(isSelected ? Color.white.opacity(0.2) : Color.primary.opacity(0.08))
                                .cornerRadius(3)
                        }
                    }
                    
                    if !result.snippet.isEmpty {
                        Text(result.snippet)
                            .font(.caption)
                            .foregroundColor(isSelected ? .white.opacity(0.9) : .secondary)
                            .lineLimit(2)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                
                Spacer()
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            .background(isSelected ? Color.accentColor : Color.clear)
            .cornerRadius(6)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
    
    private func iconForPath(_ path: String) -> String {
        let ext = (path as NSString).pathExtension.lowercased()
        switch ext {
        case "md", "txt": return "doc.text"
        case "ts", "js", "py", "rs", "go": return "chevron.left.forwardslash.chevron.right"
        case "json", "yaml", "yml", "toml", "surql": return "slider.horizontal.3"
        default: return "doc"
        }
    }
}

struct VisualEffectBlur: NSViewRepresentable {
    public typealias NSViewType = NSVisualEffectView

    public func makeNSView(context: Context) -> NSVisualEffectView {
        let view = NSVisualEffectView()
        view.material = .popover
        view.blendingMode = .behindWindow
        view.state = .active
        return view
    }
    
    public func updateNSView(_ nsView: NSVisualEffectView, context: Context) {}
}
