import SwiftUI

public struct ProfileListView: View {
    @ObservedObject public var viewModel: AppViewModel
    public var onAddProfile: () -> Void
    
    public init(viewModel: AppViewModel, onAddProfile: @escaping () -> Void = {}) {
        self.viewModel = viewModel
        self.onAddProfile = onAddProfile
    }
    
    public var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Profiles")
                .font(.caption)
                .fontWeight(.bold)
                .foregroundColor(.secondary)
                .padding(.horizontal, 10)
            
            ForEach(viewModel.profiles) { profile in
                Button(action: {
                    viewModel.selectProfile(profile)
                }) {
                    HStack {
                        Image(systemName: profile.id == viewModel.activeProfile.id ? "checkmark" : "")
                            .font(.system(size: 10, weight: .bold))
                            .frame(width: 14)
                            .foregroundColor(.accentColor)
                        
                        VStack(alignment: .leading, spacing: 1) {
                            Text(profile.name)
                                .font(.caption)
                                .fontWeight(profile.id == viewModel.activeProfile.id ? .semibold : .regular)
                            Text(profile.endpoint)
                                .font(.caption2)
                                .foregroundColor(.secondary)
                        }
                        
                        Spacer()
                    }
                    .padding(.horizontal, 8)
                    .padding(.vertical, 3)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            
            Button(action: onAddProfile) {
                HStack {
                    Image(systemName: "plus")
                        .font(.system(size: 10))
                        .frame(width: 14)
                    Text("Add New Connection...")
                        .font(.caption)
                    Spacer()
                }
                .foregroundColor(.secondary)
                .padding(.horizontal, 8)
                .padding(.vertical, 3)
            }
            .buttonStyle(.plain)
        }
    }
}
