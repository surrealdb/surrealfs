import Foundation

public enum SurrealConnectionState: Equatable, Sendable {
    case disconnected
    case connecting
    case connected(endpoint: String)
    case error(message: String)
}

public final class SurrealClient: @unchecked Sendable {
    private var webSocketTask: URLSessionWebSocketTask?
    private let session: URLSession
    private var requestCounter: Int = 1
    private var pendingRequests: [Int: CheckedContinuation<Any, Error>] = [:]
    private var liveQueryListeners: [UUID: @Sendable (AgentActivity) -> Void] = [:]
    private let queue = DispatchQueue(label: "io.surrealfs.client")
    
    private var _state: SurrealConnectionState = .disconnected
    public var state: SurrealConnectionState {
        queue.sync { _state }
    }
    
    public init() {
        self.session = URLSession(configuration: .default)
    }
    
    public func connect(profile: ConnectionProfile, secret: String? = nil) async throws {
        queue.sync { _state = .connecting }
        
        guard let url = URL(string: profile.endpoint) else {
            queue.sync { _state = .error(message: "Invalid URL") }
            throw URLError(.badURL)
        }
        
        let wsUrl: URL
        if url.scheme == "http" {
            wsUrl = URL(string: "ws://\(url.host ?? "localhost"):\(url.port ?? 8000)/rpc")!
        } else if url.scheme == "https" {
            wsUrl = URL(string: "wss://\(url.host ?? "localhost"):\(url.port ?? 443)/rpc")!
        } else {
            var comp = URLComponents(url: url, resolvingAgainstBaseURL: false)
            if comp?.path.isEmpty ?? true || comp?.path == "/" {
                comp?.path = "/rpc"
            }
            wsUrl = comp?.url ?? url
        }
        
        let task = session.webSocketTask(with: wsUrl)
        queue.sync { self.webSocketTask = task }
        task.resume()
        
        listenForMessages()
        
        // Use namespace & database
        _ = try await sendRpc(method: "use", params: [profile.namespace, profile.database])
        
        // Sign in if credentials exist
        if let pass = secret, !pass.isEmpty {
            let authPayload: [String: Any]
            if profile.authLevel == .record {
                authPayload = [
                    "access": "account",
                    "user": profile.username,
                    "pass": pass
                ]
            } else {
                authPayload = [
                    "user": profile.username,
                    "pass": pass
                ]
            }
            _ = try? await sendRpc(method: "signin", params: [authPayload])
        }
        
        queue.sync { _state = .connected(endpoint: profile.endpoint) }
    }
    
    public func disconnect() {
        queue.sync {
            webSocketTask?.cancel(with: .goingAway, reason: nil)
            webSocketTask = nil
            _state = .disconnected
        }
    }
    
    public func search(query: String, limit: Int = 20) async throws -> [SearchResult] {
        let isConn: Bool = queue.sync {
            if case .connected = _state { return true }
            return false
        }
        
        guard isConn else {
            return []
        }
        
        let sql = "RETURN fn::sfs_search_text($q, $limit, 'any', NONE);"
        let response = try await queryRaw(sql: sql, vars: ["q": query, "limit": limit])
        
        guard let items = response as? [[String: Any]] else {
            return []
        }
        
        return items.map { item in
            SearchResult(
                path: item["path"] as? String ?? "",
                filename: (item["path"] as? String as? NSString)?.lastPathComponent ?? "",
                snippet: item["snippet"] as? String ?? "",
                score: item["score"] as? Double ?? 0.0,
                matchType: "fulltext"
            )
        }
    }
    
    public func undo(path: String, generation: Int) async throws -> Bool {
        let isConn: Bool = queue.sync {
            if case .connected = _state { return true }
            return false
        }
        
        guard isConn else { return false }
        let prevGen = max(1, generation - 1)
        let sql = "RETURN fn::sfs_restore($path, $version, NONE);"
        _ = try await queryRaw(sql: sql, vars: ["path": path, "version": prevGen])
        return true
    }
    
    public func queryRaw(sql: String, vars: [String: Any] = [:]) async throws -> Any {
        return try await sendRpc(method: "query", params: [sql, vars])
    }
    
    public func subscribeActivity(listener: @escaping @Sendable (AgentActivity) -> Void) -> UUID {
        queue.sync {
            let id = UUID()
            liveQueryListeners[id] = listener
            return id
        }
    }
    
    public func unsubscribeActivity(id: UUID) {
        queue.sync {
            _ = liveQueryListeners.removeValue(forKey: id)
        }
    }
    
    private func sendRpc(method: String, params: [Any]) async throws -> Any {
        let (task, reqId) = try queue.sync { () -> (URLSessionWebSocketTask, Int) in
            guard let t = webSocketTask else {
                throw URLError(.notConnectedToInternet)
            }
            let id = requestCounter
            requestCounter += 1
            return (t, id)
        }
        
        let payload: [String: Any] = [
            "id": reqId,
            "method": method,
            "params": params
        ]
        
        let data = try JSONSerialization.data(withJSONObject: payload)
        let message = URLSessionWebSocketTask.Message.data(data)
        
        return try await withCheckedThrowingContinuation { continuation in
            queue.sync {
                pendingRequests[reqId] = continuation
            }
            
            task.send(message) { [weak self] error in
                if let error = error {
                    self?.queue.sync {
                        _ = self?.pendingRequests.removeValue(forKey: reqId)
                    }
                    continuation.resume(throwing: error)
                }
            }
        }
    }
    
    private func listenForMessages() {
        let task: URLSessionWebSocketTask? = queue.sync { webSocketTask }
        guard let task = task else { return }
        
        task.receive { [weak self] result in
            guard let self = self else { return }
            
            switch result {
            case .success(let message):
                self.handleIncomingMessage(message)
                self.listenForMessages()
            case .failure(let error):
                self.handleConnectionFailure(error)
            }
        }
    }
    
    private func handleIncomingMessage(_ message: URLSessionWebSocketTask.Message) {
        let data: Data?
        switch message {
        case .data(let d): data = d
        case .string(let s): data = s.data(using: .utf8)
        @unknown default: data = nil
        }
        
        guard let data = data,
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return
        }
        
        // Handle RPC responses
        let continuation: CheckedContinuation<Any, Error>? = queue.sync {
            if let id = json["id"] as? Int {
                return pendingRequests.removeValue(forKey: id)
            }
            return nil
        }
        
        if let continuation = continuation {
            if let error = json["error"] as? [String: Any] {
                let msg = error["message"] as? String ?? "RPC Error"
                continuation.resume(throwing: NSError(domain: "SurrealClient", code: -1, userInfo: [NSLocalizedDescriptionKey: msg]))
            } else {
                continuation.resume(returning: json["result"] ?? [:])
            }
            return
        }
        
        // Handle Live Query streaming notification
        if let liveResult = json["result"] as? [String: Any],
           let actionStr = liveResult["action"] as? String,
           let record = liveResult["result"] as? [String: Any] {
            let path = record["path"] as? String ?? "/"
            let author = record["author"] as? String ?? record["owner"] as? String ?? "agent"
            let actionType: AgentActivity.ActionType
            switch actionStr.uppercased() {
            case "CREATE": actionType = .write
            case "UPDATE": actionType = .append
            case "DELETE": actionType = .delete
            default: actionType = .write
            }
            
            let activity = AgentActivity(
                icon: author.contains("bot") || author.contains("hermes") || author.contains("agent") ? "🤖" : "👤",
                actor: author,
                action: actionType,
                path: path,
                details: "live",
                timestamp: Date(),
                generation: record["generation"] as? Int
            )
            
            let listeners: [@Sendable (AgentActivity) -> Void] = queue.sync {
                Array(liveQueryListeners.values)
            }
            
            for listener in listeners {
                listener(activity)
            }
        }
    }
    
    private func handleConnectionFailure(_ error: Error) {
        queue.sync {
            _state = .error(message: error.localizedDescription)
        }
    }
}
