import SwiftUI

public struct PreferencesView: View {
    @ObservedObject public var viewModel: AppViewModel
    @State private var selectedTab = 0
    @State private var newProfileName = ""
    @State private var newProfileUrl = "ws://localhost:8000"
    @State private var newProfileNs = "surrealfs"
    @State private var newProfileDb = "brain"
    @State private var newProfileUser = "root"
    @State private var newProfilePass = ""
    @State private var newProfileAuth: ConnectionProfile.AuthLevel = .record
    @State private var mountPathInput = "~/mnt/surrealfs"
    
    public init(viewModel: AppViewModel) {
        self.viewModel = viewModel
    }
    
    public var body: some View {
        TabView(selection: $selectedTab) {
            // Profiles Tab
            Form {
                Section(header: Text("Configured Profiles")) {
                    List {
                        ForEach(viewModel.profiles) { profile in
                            HStack {
                                VStack(alignment: .leading) {
                                    Text(profile.name)
                                        .fontWeight(.semibold)
                                    Text("\(profile.endpoint) (\(profile.namespace)/\(profile.database))")
                                        .font(.caption)
                                        .foregroundColor(.secondary)
                                }
                                Spacer()
                                if profile.id == viewModel.activeProfile.id {
                                    Text("Active")
                                        .font(.caption2)
                                        .padding(.horizontal, 6)
                                        .padding(.vertical, 2)
                                        .background(Color.green.opacity(0.2))
                                        .foregroundColor(.green)
                                        .cornerRadius(4)
                                }
                            }
                        }
                    }
                    .frame(height: 120)
                }
                
                Section(header: Text("Add Connection Profile")) {
                    TextField("Profile Name", text: $newProfileName)
                    TextField("SurrealDB URL", text: $newProfileUrl)
                    HStack {
                        TextField("Namespace", text: $newProfileNs)
                        TextField("Database", text: $newProfileDb)
                    }
                    HStack {
                        TextField("Username", text: $newProfileUser)
                        SecureField("Password / Token", text: $newProfilePass)
                    }
                    Picker("Auth Level", selection: $newProfileAuth) {
                        Text("Record Auth (Recommended)").tag(ConnectionProfile.AuthLevel.record)
                        Text("Root / System").tag(ConnectionProfile.AuthLevel.system)
                    }
                    
                    Button("Save Profile") {
                        let newProf = ConnectionProfile(
                            name: newProfileName.isEmpty ? "Custom" : newProfileName,
                            endpoint: newProfileUrl,
                            namespace: newProfileNs,
                            database: newProfileDb,
                            username: newProfileUser,
                            authLevel: newProfileAuth
                        )
                        viewModel.profiles.append(newProf)
                        if !newProfilePass.isEmpty {
                            _ = KeychainService.shared.saveSecret(newProfilePass, for: newProfileUser)
                        }
                        newProfileName = ""
                        newProfilePass = ""
                    }
                    .disabled(newProfileUrl.isEmpty)
                }
            }
            .tabItem {
                Label("Connections", systemImage: "network")
            }
            .tag(0)
            
            // Mount & Filesystem Tab
            Form {
                Section(header: Text("FUSE Mount Options")) {
                    TextField("Mount Path", text: $mountPathInput)
                    Toggle("Auto-mount on application launch", isOn: .constant(true))
                    Toggle("Auto-unmount on system sleep", isOn: .constant(true))
                }
                
                Section(header: Text("Keyboard Shortcuts")) {
                    HStack {
                        Text("Global Spotlight Search:")
                        Spacer()
                        Text("⌘ ⇧ S")
                            .font(.system(.body, design: .monospaced))
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .background(Color.secondary.opacity(0.1))
                            .cornerRadius(4)
                    }
                }
            }
            .tabItem {
                Label("General", systemImage: "gearshape")
            }
            .tag(1)
        }
        .padding(20)
        .frame(width: 480, height: 380)
    }
}
