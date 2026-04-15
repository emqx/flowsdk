// SPDX-License-Identifier: MPL-2.0
// FlowSDK TCP MQTT client example (macOS / Linux)
//
// Usage:
//   LIBRARY_PATH=swift/lib swift run TcpClientExample [host] [port]
//
// Default broker: broker.emqx.io:1883

import Foundation
import FlowSDK

#if canImport(Darwin)
import Darwin
#elseif canImport(Glibc)
import Glibc
#endif

// MARK: - Configuration

private let brokerHost = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "broker.emqx.io"
private let brokerPort = CommandLine.arguments.count > 2 ? Int(CommandLine.arguments[2])! : 1883
private let runDurationMs: UInt64 = 15_000
private let tickIntervalMs:  UInt64 = 10
private let recvBufSize    = 65536

// MARK: - POSIX TCP helpers

private func makeNonBlockingTcpSocket(host: String, port: Int) -> Int32 {
    var hints = addrinfo()
    hints.ai_family   = AF_UNSPEC
    hints.ai_socktype = SOCK_STREAM
    var res: UnsafeMutablePointer<addrinfo>?
    guard getaddrinfo(host, String(port), &hints, &res) == 0, let ai = res else {
        fatalError("getaddrinfo failed for \(host):\(port)")
    }
    defer { freeaddrinfo(res) }

    let fd = socket(ai.pointee.ai_family, ai.pointee.ai_socktype, ai.pointee.ai_protocol)
    guard fd >= 0 else { fatalError("socket() failed") }

    // Non-blocking
    let flags = fcntl(fd, F_GETFL)
    _ = fcntl(fd, F_SETFL, flags | O_NONBLOCK)

    let rc = Darwin.connect(fd, ai.pointee.ai_addr, ai.pointee.ai_addrlen)
    if rc < 0 && errno != EINPROGRESS {
        fatalError("connect() failed: \(String(cString: strerror(errno)))")
    }
    return fd
}

private func waitWritable(_ fd: Int32, timeoutMs: Int) -> Bool {
    var pfd = pollfd(fd: fd, events: Int16(POLLOUT), revents: 0)
    return poll(&pfd, 1, Int32(timeoutMs)) > 0
}

private func waitReadable(_ fd: Int32, timeoutMs: Int) -> Bool {
    var pfd = pollfd(fd: fd, events: Int16(POLLIN), revents: 0)
    return poll(&pfd, 1, Int32(timeoutMs)) > 0
}

private func sendAll(_ fd: Int32, data: Data) {
    guard !data.isEmpty else { return }
    data.withUnsafeBytes { ptr in
        var sent = 0
        while sent < data.count {
            let n = send(fd, ptr.baseAddress!.advanced(by: sent), data.count - sent, 0)
            if n <= 0 { return }
            sent += n
        }
    }
}

// MARK: - Entry point

func nowMs(since startMs: UInt64) -> UInt64 {
    return UInt64(Date().timeIntervalSince1970 * 1000) - startMs
}

func handleEvent(_ event: MqttEventFfi, engine: MqttEngineFfi, fd: Int32,
                 subscribed: inout Bool, published: inout Bool, startMs: UInt64) {
    switch event {
    case .connected(let r):
        print("Connected! sessionPresent=\(r.sessionPresent)")
        if !subscribed {
            let pid = engine.subscribe(topicFilter: "test/swift/tcp", qos: 1)
            print("Subscribed to 'test/swift/tcp' (PID \(pid))")
            subscribed = true
            sendAll(fd, data: engine.takeOutgoing())
        }
    case .subscribed(let r):
        print("Subscribe ack PID \(r.packetId)")
        if !published {
            let payload = Data("Hello from Swift TCP!".utf8)
            let pid = engine.publish(topic: "test/swift/tcp", payload: payload, qos: 1, priority: nil)
            print("Published to 'test/swift/tcp' (PID \(pid))")
            published = true
            sendAll(fd, data: engine.takeOutgoing())
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

print("Initializing FlowSDK TCP Swift Example...")

let opts = MqttOptionsFfi(
    clientId: "swift_tcp_\(Int(Date().timeIntervalSince1970) % 100_000)",
    mqttVersion: 5,
    cleanStart: true,
    keepAlive: 60,
    username: nil,
    password: nil,
    reconnectBaseDelayMs: 1_000,
    reconnectMaxDelayMs: 10_000,
    maxReconnectAttempts: 3
)

let engine = MqttEngineFfi.newWithOpts(opts: opts)
print("Engine created.")

// Track relative time from engine creation (required by tick API)
let engineStartMs = UInt64(Date().timeIntervalSince1970 * 1000)

// Trigger MQTT CONNECT packet generation
engine.connect()

// Open TCP socket
print("Connecting to TCP broker at \(brokerHost):\(brokerPort)...")
let fd = makeNonBlockingTcpSocket(host: brokerHost, port: brokerPort)

// Complete async TCP connect
if !waitWritable(fd, timeoutMs: 5000) {
    fatalError("TCP connect timed out")
}
var err: Int32 = 0
var errLen = socklen_t(MemoryLayout<Int32>.size)
getsockopt(fd, SOL_SOCKET, SO_ERROR, &err, &errLen)
guard err == 0 else { fatalError("TCP connect error: \(err)") }
print("TCP connected.")

// Flush initial CONNECT packet
sendAll(fd, data: engine.takeOutgoing())

// Main event loop
var recvBuf = [UInt8](repeating: 0, count: recvBufSize)
var subscribed = false
var published  = false

while nowMs(since: engineStartMs) < runDurationMs {
    // Wait up to one tick interval for readable data
    if waitReadable(fd, timeoutMs: Int(tickIntervalMs)) {
        let n = recv(fd, &recvBuf, recvBufSize, 0)
        if n > 0 {
            let inData = Data(recvBuf[0..<n])
            let events = engine.handleIncoming(data: inData)
            for e in events {
                handleEvent(e, engine: engine, fd: fd,
                            subscribed: &subscribed, published: &published,
                            startMs: engineStartMs)
            }
        } else if n == 0 {
            print("TCP connection closed by broker")
            break
        }
    }

    // Tick the engine (drives MQTT keepalive and timeouts)
    let events = engine.handleTick(nowMs: nowMs(since: engineStartMs))
    for e in events {
        handleEvent(e, engine: engine, fd: fd,
                    subscribed: &subscribed, published: &published,
                    startMs: engineStartMs)
    }

    // Flush any new outgoing bytes
    sendAll(fd, data: engine.takeOutgoing())
}

// Graceful disconnect
print("Run time elapsed, disconnecting...")
engine.disconnect()
sendAll(fd, data: engine.takeOutgoing())
Darwin.close(fd)
print("Done.")
