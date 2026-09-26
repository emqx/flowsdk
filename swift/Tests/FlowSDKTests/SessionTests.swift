// SPDX-License-Identifier: MPL-2.0
import Foundation
import FlowSDK

// Executable test harness also works with the Command Line Tools toolchain,
// which does not ship XCTest. CI runs it through build_swift_bindings.sh --test.
func expectFailure(_ action: () throws -> Void) {
    do { try action() } catch { return }
    fatalError("Expected operation to fail")
}

struct SessionTests {
    func engine() throws -> MqttEngineFfi {
        let options = MqttOptionsFfi(clientId: "swift-restart", mqttVersion: 5, cleanStart: false,
            keepAlive: 0, username: nil, password: nil, reconnectBaseDelayMs: 1000,
            reconnectMaxDelayMs: 60000, maxReconnectAttempts: 0)
        let connect = MqttConnectOptionsFfi(options: options,
            properties: [.sessionExpiryInterval(value: 3600)], will: nil, binaryPassword: nil, engineOptions: nil)
        let runtime = MqttRuntimeOptionsFfi(peer: "tcp://fixture:1883", operationTimeouts: nil,
            incomingReceiveMaximum: nil, maxIncomingPacketSize: nil, maxIncomingBufferBytes: nil,
            maxOutgoingBufferBytes: nil, reconnect: false)
        return try MqttEngineFfi.newWithRuntimeOptions(opts: connect, runtime: runtime)
    }

    func testInMemorySessionResumesWithoutCheckpoint() throws {
        let client = try engine()
        try client.connectChecked()
        expectFailure { try client.connectChecked() }
        _ = client.takeOutgoing()
        _ = client.handleIncoming(data: Data([0x20, 3, 0, 0, 0]))
        _ = try client.publishWithOptions(topic: "test", payload: Data([0, 255]),
            options: MqttPublishOptionsFfi(qos: 1, retain: false, priority: nil, properties: []))
        _ = client.takeOutgoing()
        client.handleConnectionLost()
        try client.connectChecked()
        _ = client.takeOutgoing()
        _ = client.handleIncoming(data: Data([0x20, 3, 1, 0, 0]))
        precondition(client.takeOutgoing().first == 0x3a)
    }

    #if FLOWSDK_DURABLE_SESSION
    func testDiskCheckpointRestoresOutstandingPublish() throws {
        let file = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: file) }
        do {
            let old = try engine()
            try old.connectChecked()
            expectFailure { try old.connectChecked() }
            _ = old.takeOutgoing()
            _ = old.handleIncoming(data: Data([0x20, 3, 0, 0, 0]))
            _ = try old.publishWithOptions(topic: "test", payload: Data([0, 255]),
                options: MqttPublishOptionsFfi(qos: 1, retain: false, priority: nil, properties: []))
            _ = old.takeOutgoing()
            try old.snapshotSession().write(to: file, options: .atomic)
        }
        let saved = try Data(contentsOf: file)
        let info = try inspectSessionState(state: saved)
        precondition(info.clientId == "swift-restart")
        let resumed = try engine()
        expectFailure { try resumed.restoreSessionState(state: Data([0])) }
        try resumed.restoreSessionState(state: saved)
        expectFailure { try resumed.restoreSessionState(state: saved) }
        try resumed.connectChecked()
        _ = resumed.takeOutgoing()
        _ = resumed.handleIncoming(data: Data([0x20, 3, 1, 0, 0]))
        precondition(resumed.takeOutgoing().first == 0x3a)
    }
    #endif
}

try SessionTests().testInMemorySessionResumesWithoutCheckpoint()
#if FLOWSDK_DURABLE_SESSION
try SessionTests().testDiskCheckpointRestoresOutstandingPublish()
#endif
print("Swift session tests passed")
