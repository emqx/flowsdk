// SPDX-License-Identifier: MPL-2.0
// FlowSDK QUIC MQTT client example (macOS / Linux)
//
// Usage:
//   LIBRARY_PATH=swift/lib swift run QuicClientExample [host] [port]
//
// For Wireshark key logging:
//   SSLKEYLOGFILE=~/tmp/sslkeylog.txt LIBRARY_PATH=swift/lib swift run QuicClientExample
//
// Default broker: broker.emqx.io:14567

import Foundation
import FlowSDK

#if canImport(Darwin)
import Darwin
#elseif canImport(Glibc)
import Glibc
#endif

// MARK: - Configuration

private let brokerHost  = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "broker.emqx.io"
private let brokerPort  = CommandLine.arguments.count > 2 ? Int(CommandLine.arguments[2])! : 14567
private let runDurationMs:  UInt64 = 10_000
private let tickIntervalMs: UInt64 = 10
private let recvBufSize = 65536

// MARK: - POSIX UDP helpers

private func resolveHostAddr(_ host: String, port: Int) -> sockaddr_in {
    var hints = addrinfo()
    hints.ai_family   = AF_INET
    hints.ai_socktype = SOCK_DGRAM
    var res: UnsafeMutablePointer<addrinfo>?
    guard getaddrinfo(host, String(port), &hints, &res) == 0, let ai = res else {
        fatalError("getaddrinfo failed for \(host):\(port)")
    }
    defer { freeaddrinfo(res) }
    var addr = sockaddr_in()
    withUnsafeMutableBytes(of: &addr) {
        $0.copyMemory(from: UnsafeRawBufferPointer(start: ai.pointee.ai_addr,
                                                    count: Int(ai.pointee.ai_addrlen)))
    }
    return addr
}

private func makeNonBlockingUdpSocket() -> Int32 {
    let fd = socket(AF_INET, SOCK_DGRAM, 0)
    guard fd >= 0 else { fatalError("socket() failed") }
    let flags = fcntl(fd, F_GETFL)
    _ = fcntl(fd, F_SETFL, flags | O_NONBLOCK)
    return fd
}

private func sendDatagram(_ fd: Int32, data: Data, to addr: inout sockaddr_in) {
    guard !data.isEmpty else { return }
    data.withUnsafeBytes { ptr in
        withUnsafeBytes(of: &addr) { addrPtr in
            _ = sendto(fd, ptr.baseAddress!, data.count, 0,
                       addrPtr.baseAddress!.assumingMemoryBound(to: sockaddr.self),
                       socklen_t(MemoryLayout<sockaddr_in>.size))
        }
    }
}

private func recvDatagram(_ fd: Int32, buf: inout [UInt8]) -> Data? {
    var src = sockaddr_in()
    var srcLen = socklen_t(MemoryLayout<sockaddr_in>.size)
    let n = withUnsafeMutableBytes(of: &src) { srcPtr in
        recvfrom(fd, &buf, buf.count, 0,
                 srcPtr.baseAddress!.assumingMemoryBound(to: sockaddr.self),
                 &srcLen)
    }
    guard n > 0 else { return nil }
    return Data(buf[0..<n])
}

private func waitReadable(_ fd: Int32, timeoutMs: Int) -> Bool {
    var pfd = pollfd(fd: fd, events: Int16(POLLIN), revents: 0)
    return poll(&pfd, 1, Int32(timeoutMs)) > 0
}

// MARK: - Engine helpers

func nowMs(since startMs: UInt64) -> UInt64 {
    return UInt64(Date().timeIntervalSince1970 * 1000) - startMs
}

/// Addr string expected by the FFI engine: "1.2.3.4:14567"
func addrString(addr: sockaddr_in, port: Int) -> String {
    var a = addr.sin_addr
    var buf = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
    inet_ntop(AF_INET, &a, &buf, socklen_t(INET_ADDRSTRLEN))
    return "\(String(cString: buf)):\(port)"
}

func sendOutgoing(_ engine: QuicMqttEngineFfi, fd: Int32, to addr: inout sockaddr_in) {
    for dgram in engine.takeOutgoingDatagrams() {
        sendDatagram(fd, data: dgram.data, to: &addr)
    }
}

// MARK: - Entry point

print("Initializing FlowSDK QUIC Swift Example...")

let opts = MqttOptionsFfi(
    clientId: "swift_quic_\(Int(Date().timeIntervalSince1970) % 100_000)",
    mqttVersion: 5,
    cleanStart: true,
    keepAlive: 30,           // Must match QUIC idle timeout (30 s)
    username: nil,
    password: nil,
    reconnectBaseDelayMs: 1_000,
    reconnectMaxDelayMs: 10_000,
    maxReconnectAttempts: 3
)

let engine = QuicMqttEngineFfi(opts: opts)
print("QUIC Engine created.")

// Enable TLS key logging when SSLKEYLOGFILE is set (for Wireshark)
let enableKeyLog = ProcessInfo.processInfo.environment["SSLKEYLOGFILE"] != nil
let tlsOpts = MqttTlsOptionsFfi(
    caCertFile: nil,
    clientCertFile: nil,
    clientKeyFile: nil,
    insecureSkipVerify: true,   // Demo broker only — do not use in production
    alpnProtocols: [],
    enableKeyLog: enableKeyLog
)

// Open non-blocking UDP socket and resolve broker
var brokerAddr = resolveHostAddr(brokerHost, port: brokerPort)
let fd = makeNonBlockingUdpSocket()
let serverAddrStr = addrString(addr: brokerAddr, port: brokerPort)

// Track relative time from engine creation (required by tick API)
let engineStartMs = UInt64(Date().timeIntervalSince1970 * 1000)

print("Connecting to QUIC broker at \(serverAddrStr) (host: \(brokerHost))...")
engine.connect(serverAddr: serverAddrStr, serverName: brokerHost,
               tlsOpts: tlsOpts, nowMs: nowMs(since: engineStartMs))
// Tick immediately to generate initial QUIC handshake packets
_ = engine.handleTick(nowMs: nowMs(since: engineStartMs))
sendOutgoing(engine, fd: fd, to: &brokerAddr)

// Main event loop
var recvBuf = [UInt8](repeating: 0, count: recvBufSize)
var subscribed = false
var published  = false

while nowMs(since: engineStartMs) < runDurationMs {
    // Drain all received datagrams
    if waitReadable(fd, timeoutMs: Int(tickIntervalMs)) {
        while let data = recvDatagram(fd, buf: &recvBuf) {
            engine.handleDatagram(data: data, remoteAddr: serverAddrStr,
                                  nowMs: nowMs(since: engineStartMs))
        }
    }

    // Tick the engine (drives QUIC timers + MQTT keepalive)
    let events = engine.handleTick(nowMs: nowMs(since: engineStartMs))

    for event in events {
        switch event {
        case .connected(let r):
            print("Connected! sessionPresent=\(r.sessionPresent)")
            if !subscribed {
                let pid = engine.subscribe(topicFilter: "test/swift/quic", qos: 1)
                print("Subscribed to 'test/swift/quic' (PID \(pid))")
                subscribed = true
                sendOutgoing(engine, fd: fd, to: &brokerAddr)
            }
        case .subscribed(let r):
            print("Subscribe ack PID \(r.packetId)")
            if !published {
                let payload = Data("Hello from Swift QUIC!".utf8)
                let pid = engine.publish(topic: "test/swift/quic", payload: payload, qos: 1)
                print("Published to 'test/swift/quic' (PID \(pid))")
                published = true
                // Tick immediately so QUIC frames are generated before the next sendOutgoing
                _ = engine.handleTick(nowMs: nowMs(since: engineStartMs))
                sendOutgoing(engine, fd: fd, to: &brokerAddr)
            }
        case .messageReceived(let m):
            let msg = String(data: m.payload, encoding: .utf8) ?? "<binary>"
            print("✅ Message on '\(m.topic)': \(msg)")
        case .published(let r):
            print("✅ Publish ack PID \(r.packetId.map(String.init) ?? "none")")
        case .disconnected(let reasonCode):
            print("⚠️ Disconnected. reasonCode=\(String(describing: reasonCode))")
        case .error(let message):
            print("❌ Error: \(message)")
        case .reconnectNeeded:
            print("ℹ Reconnect needed")
        case .reconnectScheduled(let attempt, let delayMs):
            print("ℹ Reconnect attempt \(attempt) in \(delayMs)ms")
        case .pingResponse(let success):
            print("ℹ Ping response: success=\(success)")
        case .unsubscribed(_):
            break
        }
    }

    // Forward engine-generated outgoing datagrams
    sendOutgoing(engine, fd: fd, to: &brokerAddr)
}

// Graceful disconnect
print("Run time elapsed, disconnecting...")
engine.disconnect()
sendOutgoing(engine, fd: fd, to: &brokerAddr)
close(fd)
print("Done.")
