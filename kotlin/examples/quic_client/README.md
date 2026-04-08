# FlowSDK QUIC MQTT Client Example

A Kotlin example demonstrating QUIC-based MQTT connectivity using FlowSDK's FFI bindings.

## Prerequisites

- **JDK 17** or later (Temurin recommended)
- **Rust toolchain** (stable)
- **QUIC-enabled MQTT broker** (e.g., `broker.emqx.io:14567`)

## Build & Run

### 1. Build the FFI Bindings

From the repository root:

```bash
./scripts/build_kotlin_bindings.sh
```

This compiles the Rust FFI library with QUIC support and generates Kotlin bindings.

### 2. Run the Example

```bash
cd kotlin
./gradlew :examples:quic_client:run
```

### 3. (Optional) Enable TLS Key Logging for Wireshark

To capture and decrypt QUIC traffic with Wireshark, enable TLS key logging:

```bash
cd kotlin
SSLKEYLOGFILE=$PWD/sslkeylog.txt ./gradlew :examples:quic_client:run
```

The TLS session keys will be written to `sslkeylog.txt` in the kotlin directory. In Wireshark:
1. Go to **Edit** → **Preferences** → **Protocols** → **TLS**
2. Set **(Pre)-Master-Secret log filename** to the absolute path of `sslkeylog.txt`
3. Start capturing on your network interface
4. Filter for `udp.port == 14567` to see the QUIC traffic
5. Wireshark will automatically decrypt the QUIC packets using the logged keys

**Note:** Make sure to use an absolute path for `SSLKEYLOGFILE` as Gradle may change the working directory.

Or from the example directory:

```bash
cd kotlin/examples/quic_client
../../gradlew run
```

## What It Does

1. **Connects** to `broker.emqx.io:14567` via QUIC (MQTT over UDP)
2. **Subscribes** to topic `test/kotlin/quic` (QoS 1)
3. **Publishes** message `"Hello from Kotlin QUIC!"` to the same topic
4. **Receives** the echoed message back from the broker
5. **Runs** for 10 seconds demonstrating the QUIC event loop
6. **Disconnects** gracefully sending QUIC close frames

## Implementation Highlights

- **Java NIO**: Uses `DatagramChannel` + `Selector` for non-blocking UDP I/O
- **Event-driven**: Polls `engine.handleTick()` every 10ms to drive QUIC timers and process MQTT events
- **Relative Timing**: Uses milliseconds elapsed since engine creation (not absolute UNIX time) for proper timeout detection
- **TLS Key Logging**: Supports SSLKEYLOGFILE environment variable for Wireshark debugging
- **Datagram flow**: 
  - Outgoing: `engine.takeOutgoingDatagrams()` → UDP send
  - Incoming: UDP receive → `engine.handleDatagram()`

## Troubleshooting

**Build fails with "QuicMqttEngineFfi not found"**  
→ Run `./scripts/build_kotlin_bindings.sh` from repository root first

**Connection timeout**  
→ Ensure the broker supports QUIC and is reachable on UDP port 14567

**Connection instability / Messages not received**  
→ Public QUIC brokers may experience connection stability issues. The example successfully connects, subscribes, and publishes, but message delivery may be affected by connection drops. For production use, consider:
  - Using a dedicated QUIC broker with stable configuration
  - Implementing connection state monitoring and reconnection logic
  - Testing with a local QUIC-enabled MQTT broker

**Native library not found**  
→ Check `kotlin/package/src/main/resources/` contains `libflowsdk_ffi.dylib` (macOS) or `.so` (Linux)  
→ If missing, run `./scripts/build_kotlin_bindings.sh` from repository root to build and copy the library

## Configuration

Edit [Main.kt](src/main/kotlin/Main.kt) constants to customize:

```kotlin
private const val BROKER_HOST = "broker.emqx.io"
private const val BROKER_PORT = 14567
private const val RUN_DURATION_MS = 10_000L  // Run for 10 seconds
```

## See Also

- [Simple TCP Client Example](../simple_client/) — TCP-based MQTT with coroutines
- [C FFI Example](../../../examples/c_ffi_example/) — C client using the same FFI
- [FlowSDK Documentation](../../../README.md)
