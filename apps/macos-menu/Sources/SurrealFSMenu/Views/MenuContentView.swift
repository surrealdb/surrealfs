import SwiftUI

public struct MenuContentView: View {
    @ObservedObject public var viewModel: AppViewModel
    public var onQuickSearch: () -> Void
    public var onPreferences: () -> Void
    public var onQuit: () -> Void
    
    public init(
        viewModel: AppViewModel,
        onQuickSearch: @escaping () -> Void = {},
        onPreferences: @escaping () -> Void = {},
        onQuit: @escaping () -> Void = {}
    ) {
        self.viewModel = viewModel
        self.onQuickSearch = onQuickSearch
        self.onPreferences = onPreferences
        self.onQuit = onQuit
    }
    
    public var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            // Header: Status indicator
            HStack(spacing: 8) {
                Circle()
                    .fill(viewModel.statusColor)
                    .frame(width: 9, height: 9)
                Text("SurrealFS: \(viewModel.statusText)")
                    .font(.headline)
                    .lineLimit(1)
                Spacer()
            }
            .padding(.horizontal, 12)
            .padding(.top, 10)
            
            Divider()
            
            // Mount Status Section
            VStack(alignment: .leading, spacing: 5) {
                Text(viewModel.mountInfo.isMounted ? "Mount Status: Mounted at \(viewModel.mountInfo.mountPath)" : "Mount Status: Unmounted")
                    .font(.caption)
                    .foregroundColor(viewModel.mountInfo.isMounted ? .primary : .secondary)
                
                Text("Branch: \(viewModel.mountInfo.branch) (\(viewModel.mountInfo.isClean ? "Clean" : "Dirty"))")
                    .font(.caption2)
                    .foregroundColor(.secondary)
                
                HStack(spacing: 8) {
                    Button(action: { viewModel.openInFinder() }) {
                        Label("Open in Finder", systemImage: "folder")
                            .font(.caption)
                    }
                    .disabled(!viewModel.mountInfo.isMounted)
                    
                    Button(action: { viewModel.openInTerminal() }) {
                        Label("Open in Terminal", systemImage: "terminal")
                            .font(.caption)
                    }
                    .disabled(!viewModel.mountInfo.isMounted)
                }
                .padding(.top, 2)
            }
            .padding(.horizontal, 12)
            
            Divider()
            
            // Profiles Section
            ProfileListView(viewModel: viewModel, onAddProfile: onPreferences)
                .padding(.horizontal, 4)
            
            Divider()
            
            // Live Agent Activity HUD Section
            ActivityHUDView(viewModel: viewModel)
                .padding(.horizontal, 2)
            
            Divider()
            
            // Quick Search Button
            Button(action: onQuickSearch) {
                HStack {
                    Image(systemName: "magnifyingglass")
                    Text("Quick Search (⌘⇧S)")
                        .fontWeight(.medium)
                    Spacer()
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 5)
                .background(Color.accentColor.opacity(0.1))
                .cornerRadius(6)
            }
            .buttonStyle(.plain)
            .padding(.horizontal, 12)
            
            Divider()
            
            // Footer Controls
            HStack {
                Button("Preferences...", action: onPreferences)
                    .font(.caption)
                    .buttonStyle(.plain)
                
                Spacer()
                
                Button(viewModel.mountInfo.isMounted ? "Unmount" : "Mount") {
                    viewModel.toggleMount()
                }
                .font(.caption)
                .buttonStyle(.plain)
                
                Button("Quit", action: onQuit)
                    .font(.caption)
                    .buttonStyle(.plain)
                    .foregroundColor(.secondary)
            }
            .padding(.horizontal, 12)
            .padding(.bottom, 10)
        }
        .frame(width: 320)
    }
}
