import Foundation
import Security

public final class KeychainService: @unchecked Sendable {
    public static let shared = KeychainService()
    private let serviceName = "io.surrealfs.menubar"
    
    // In-memory fallback cache for development and test harnesses
    private var memoryStore: [String: String] = [:]
    private let lock = NSLock()
    
    public init() {}
    
    public func saveSecret(_ secret: String, for account: String) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        memoryStore[account] = secret
        
        guard let data = secret.data(using: .utf8) else { return false }
        
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: serviceName,
            kSecAttrAccount as String: account,
            kSecValueData as String: data
        ]
        
        SecItemDelete(query as CFDictionary)
        let status = SecItemAdd(query as CFDictionary, nil)
        return status == errSecSuccess
    }
    
    public func getSecret(for account: String) -> String? {
        lock.lock()
        defer { lock.unlock() }
        
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: serviceName,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne
        ]
        
        var dataTypeRef: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &dataTypeRef)
        
        if status == errSecSuccess, let data = dataTypeRef as? Data, let str = String(data: data, encoding: .utf8) {
            return str
        }
        
        return memoryStore[account]
    }
    
    public func deleteSecret(for account: String) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        memoryStore.removeValue(forKey: account)
        
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: serviceName,
            kSecAttrAccount as String: account
        ]
        
        let status = SecItemDelete(query as CFDictionary)
        return status == errSecSuccess || status == errSecItemNotFound
    }
}
