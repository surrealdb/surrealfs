import Foundation
import SwiftUI
import Combine

@MainActor
public final class AppViewModel: ObservableObject {
    @Published public var connectionState: SurrealConnectionState = .disconnected
    @Published public var profiles: [ConnectionProfile] = []
    @Published public var activeProfile: ConnectionProfile
    @Published public var mountInfo: MountInfo
    @Published public var recentActivities: [AgentActivity] = []
    @Published public var isSearchPresented: Bool = false
    
    private let client: SurrealClient
    private let mountManager: MountManager
    private let keychain: KeychainService
    private var activitySubId: UUID?
    
    public init(
        client: SurrealClient = SurrealClient(),
        mountManager: MountManager = .shared,
        keychain: KeychainService = .shared
    ) {
        self.client = client
        self.mountManager = mountManager
        self.keychain = keychain
        
        let initialProfiles = ConnectionProfile.defaultProfiles
        self.profiles = initialProfiles
        self.activeProfile = initialProfiles.first(where: { $0.isDefault }) ?? initialProfiles[0]
        self.mountInfo = mountManager.checkMountStatus(path: "~/mnt/surrealfs")
        
        // Seed with sample initial activities if live query hasn't streamed yet
        self.recentActivities = [
            AgentActivity(icon: "🤖", actor: "hermes", action: .write, path: "/brain/acme/risks/okta.md"),
            AgentActivity(icon: "🤖", actor: "triage-bot", action: .append, path: "/incidents/log.md"),
            AgentActivity(icon: "👤", actor: "martin", action: .lock, path: "/projects/auth/", details: "32s left", canUndo: false)
        ]
        
        setupActivityListener()
    }
    
    public var statusIcon: String {
        switch connectionState {
        case .connected: return "circle.fill"
        case .connecting: return "circle.dotted"
        case .disconnected, .error: return "circle"
        }
    }
    
    public var statusColor: Color {
        switch connectionState {
        case .connected: return .green
        case .connecting: return .yellow
        case .disconnected, .error: return .red
        }
    }
    
    public var statusText: String {
        switch connectionState {
        case .connected: return "Connected (\(activeProfile.name))"
        case .connecting: return "Connecting to \(activeProfile.name)..."
        case .disconnected: return "Disconnected"
        case .error(let msg): return "Error: \(msg)"
        }
    }
    
    public func connect() async {
        let secret = keychain.getSecret(for: activeProfile.username)
        do {
            try await client.connect(profile: activeProfile, secret: secret)
            self.connectionState = client.state
        } catch {
            self.connectionState = .error(message: error.localizedDescription)
        }
    }
    
    public func disconnect() async {
        client.disconnect()
        self.connectionState = .disconnected
    }
    
    public func selectProfile(_ profile: ConnectionProfile) {
        self.activeProfile = profile
        Task {
            await disconnect()
            await connect()
        }
    }
    
    public func toggleMount() {
        var mounted = mountInfo.isMounted
        mountManager.toggleMount(path: mountInfo.mountPath, isMounted: &mounted)
        mountInfo.isMounted = mounted
    }
    
    public func openInFinder() {
        mountManager.openInFinder(path: mountInfo.mountPath)
    }
    
    public func openInTerminal() {
        mountManager.openInTerminal(path: mountInfo.mountPath)
    }
    
    public func undoActivity(_ activity: AgentActivity) {
        guard activity.canUndo, let gen = activity.generation else { return }
        Task {
            _ = try? await client.undo(path: activity.path, generation: gen)
            recentActivities.removeAll { $0.id == activity.id }
        }
    }
    
    private func setupActivityListener() {
        self.activitySubId = client.subscribeActivity { [weak self] activity in
            Task { @MainActor in
                self?.recentActivities.insert(activity, at: 0)
                if (self?.recentActivities.count ?? 0) > 20 {
                    self?.recentActivities.removeLast()
                }
            }
        }
    }
}
