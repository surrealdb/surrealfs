import SwiftUI

public struct ActivityHUDView: View {
    @ObservedObject public var viewModel: AppViewModel
    
    public init(viewModel: AppViewModel) {
        self.viewModel = viewModel
    }
    
    public var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("Live Agent Activity")
                    .font(.caption)
                    .fontWeight(.bold)
                    .foregroundColor(.secondary)
                Spacer()
                Text("Streaming")
                    .font(.caption2)
                    .foregroundColor(.green)
            }
            .padding(.horizontal, 10)
            
            if viewModel.recentActivities.isEmpty {
                Text("No recent agent activity")
                    .font(.caption)
                    .foregroundColor(.secondary)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 4)
            } else {
                ForEach(viewModel.recentActivities) { activity in
                    ActivityRow(activity: activity) {
                        viewModel.undoActivity(activity)
                    }
                }
            }
        }
    }
}

struct ActivityRow: View {
    let activity: AgentActivity
    let onUndo: () -> Void
    @State private var isHovered = false
    
    var body: some View {
        HStack(spacing: 8) {
            Text(activity.icon)
                .font(.body)
            
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 4) {
                    Text(activity.actor)
                        .fontWeight(.semibold)
                    Text(activity.action.rawValue)
                        .foregroundColor(.secondary)
                    Text(activity.path)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .font(.system(.caption, design: .monospaced))
                }
                .font(.caption)
                
                if let details = activity.details {
                    Text(details)
                        .font(.caption2)
                        .foregroundColor(.secondary)
                }
            }
            
            Spacer()
            
            if activity.canUndo && isHovered {
                Button(action: onUndo) {
                    Text("Undo")
                        .font(.caption2)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Color.accentColor.opacity(0.15))
                        .cornerRadius(4)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 4)
        .background(isHovered ? Color.primary.opacity(0.05) : Color.clear)
        .cornerRadius(6)
        .onHover { isHovered = $0 }
    }
}
