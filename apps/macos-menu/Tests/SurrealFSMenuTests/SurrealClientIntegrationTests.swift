import Foundation
import XCTest
@testable import SurrealFSMenu

final class SurrealClientIntegrationTests: XCTestCase {
    private func findSurrealBinary() -> String? {
        let envPath = ProcessInfo.processInfo.environment["PATH"] ?? ""
        let candidates = envPath.split(separator: ":").map { String($0) + "/surreal" } + [
            "/opt/homebrew/bin/surreal",
            "/usr/local/bin/surreal",
            "\(NSHomeDirectory())/.surrealdb/surreal"
        ]
        for path in candidates {
            if FileManager.default.isExecutableFile(atPath: path) {
                return path
            }
        }
        return nil
    }

    private func freePort() throws -> UInt16 {
        let sock = socket(AF_INET, SOCK_STREAM, 0)
        guard sock >= 0 else { throw URLError(.cannotConnectToHost) }
        defer { close(sock) }

        var addr = sockaddr_in()
        addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = 0
        addr.sin_addr.s_addr = inet_addr("127.0.0.1")

        let bindRes = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.bind(sock, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        guard bindRes == 0 else { throw URLError(.cannotConnectToHost) }

        var len = socklen_t(MemoryLayout<sockaddr_in>.size)
        let nameRes = withUnsafeMutablePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                getsockname(sock, $0, &len)
            }
        }
        guard nameRes == 0 else { throw URLError(.cannotConnectToHost) }
        return UInt16(bigEndian: addr.sin_port)
    }

    func testLiveWebSocketConnectSearchUndoAndDisconnect() async throws {
        guard let surrealBin = findSurrealBinary() else {
            throw XCTSkip("surreal binary not found on PATH")
        }

        let port = try freePort()
        let bindAddr = "127.0.0.1:\(port)"
        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: surrealBin)
        proc.arguments = [
            "start",
            "--allow-all",
            "--user", "root",
            "--pass", "root",
            "--bind", bindAddr,
            "memory"
        ]
        proc.standardOutput = FileHandle.nullDevice
        proc.standardError = FileHandle.nullDevice
        try proc.run()
        defer {
            if proc.isRunning {
                proc.terminate()
                proc.waitUntilExit()
            }
        }

        // Wait for SurrealDB HTTP /health to become ready
        let healthUrl = URL(string: "http://\(bindAddr)/health")!
        var ready = false
        for _ in 0..<50 {
            if let (_, resp) = try? await URLSession.shared.data(from: healthUrl),
               let httpResp = resp as? HTTPURLResponse,
               httpResp.statusCode == 200 {
                ready = true
                break
            }
            try await Task.sleep(nanoseconds: 100_000_000)
        }
        XCTAssertTrue(ready, "SurrealDB server did not become ready on \(bindAddr)")

        let endpoint = "ws://\(bindAddr)"
        let profile = ConnectionProfile(
            name: "Integration Test DB",
            endpoint: endpoint,
            namespace: "test_swift",
            database: "test_swift",
            username: "root",
            authLevel: .system
        )

        let client = SurrealClient()
        try await client.connect(profile: profile, secret: "root")
        XCTAssertEqual(client.state, .connected(endpoint: endpoint))

        // Load and apply the repository's schema/file.surql
        let testFileUrl = URL(fileURLWithPath: #filePath)
        let repoRoot = testFileUrl
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let schemaUrl = repoRoot.appendingPathComponent("surrealfs/schema/file.surql")
        let schemaSql = try String(contentsOf: schemaUrl, encoding: .utf8)
        _ = try await client.queryRaw(sql: schemaSql)

        // Write a target file (v1, then v2) and filler files for BM25 corpus size
        _ = try await client.queryRaw(
            sql: "RETURN fn::sfs_write($path, $content, NONE, true, 'root', NONE);",
            vars: ["path": "/docs/architecture.md", "content": "Quantum storage engine v1"]
        )
        _ = try await client.queryRaw(
            sql: "RETURN fn::sfs_write($path, $content, NONE, true, 'root', NONE);",
            vars: ["path": "/docs/architecture.md", "content": "Quantum storage engine v2"]
        )
        for idx in 0..<8 {
            _ = try await client.queryRaw(
                sql: "RETURN fn::sfs_write($path, $content, NONE, true, 'root', NONE);",
                vars: ["path": "/docs/filler_\(idx).md", "content": "unrelated filler note \(idx)"]
            )
        }

        // Exercise full-text search over WebSocket RPC
        let hits = try await client.search(query: "Quantum", limit: 5)
        XCTAssertEqual(hits.count, 1)
        XCTAssertEqual(hits.first?.path, "/docs/architecture.md")
        XCTAssertEqual(hits.first?.filename, "architecture.md")

        // Exercise undo (restores generation 1 from generation 2)
        let undone = try await client.undo(path: "/docs/architecture.md", generation: 2)
        XCTAssertTrue(undone)

        let readResp = try await client.queryRaw(
            sql: "RETURN fn::sfs_read($path, 'root');",
            vars: ["path": "/docs/architecture.md"]
        )
        if let arr = readResp as? [[String: Any]],
           let restoredText = arr.first?["result"] as? String {
            XCTAssertEqual(restoredText, "Quantum storage engine v1")
        } else {
            XCTFail("Unexpected fn::sfs_read response: \(readResp)")
        }

        client.disconnect()
        XCTAssertEqual(client.state, .disconnected)
    }
}
