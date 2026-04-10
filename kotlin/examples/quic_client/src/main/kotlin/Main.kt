package example.quic

import uniffi.flowsdk_ffi.*
import java.net.InetSocketAddress
import java.nio.ByteBuffer
import java.nio.channels.DatagramChannel
import java.nio.channels.SelectionKey
import java.nio.channels.Selector
import java.nio.charset.StandardCharsets

private const val BROKER_HOST = "broker.emqx.io"
private const val BROKER_PORT = 14567
private const val RUN_DURATION_MS = 10_000L
private const val TICK_INTERVAL_MS = 10L
private const val RECV_BUFFER_SIZE = 65536

fun main() {
    println("Initializing FlowSDK QUIC Kotlin Example...")

    // 1. Configure MQTT Options
    val opts = MqttOptionsFfi(
        clientId = "kotlin_quic_${System.currentTimeMillis() % 100000}",
        mqttVersion = 5.toUByte(),
        cleanStart = true,
        keepAlive = 30.toUShort(),  // Match Python default: 30 seconds (was 60)
        username = null,
        password = null,
        reconnectBaseDelayMs = 1000.toULong(),
        reconnectMaxDelayMs = 10000.toULong(),
        maxReconnectAttempts = 3.toUInt()
    )

    // 2. Create QUIC Engine
    val engine = QuicMqttEngineFfi(opts)
    println("QUIC Engine created.")

    // 3. TLS options — insecureSkipVerify=true for demo broker only
    val enableTlsKeyLog = !System.getenv("SSLKEYLOGFILE").isNullOrBlank()
    val tlsOpts = MqttTlsOptionsFfi(
        caCertFile = null,
        clientCertFile = null,
        clientKeyFile = null,
        insecureSkipVerify = true,
        alpnProtocols = listOf(),
        enableKeyLog = enableTlsKeyLog // Enable TLS keylog for debugging QUIC traffic
    )

    // 4. Resolve broker and open non-blocking UDP socket
    val brokerAddr = InetSocketAddress(BROKER_HOST, BROKER_PORT)
    val channel = DatagramChannel.open().apply {
        configureBlocking(false)
        connect(brokerAddr)
    }
    val selector = Selector.open()
    channel.register(selector, SelectionKey.OP_READ)

    val serverAddrStr = "${brokerAddr.address.hostAddress}:$BROKER_PORT"
    println("Connecting to QUIC broker at $serverAddrStr (host: $BROKER_HOST)...")

    // 5. Initiate QUIC handshake — tick to generate initial packets
    val startTime = System.currentTimeMillis()  // Track start time for relative timestamps
    engine.connect(serverAddrStr, BROKER_HOST, tlsOpts, nowMs(startTime))
    engine.handleTick(nowMs(startTime))
    sendOutgoing(engine, channel)

    // 6. Event loop: send/receive datagrams + tick engine
    val recvBuf = ByteBuffer.allocateDirect(RECV_BUFFER_SIZE)
    var subscribed = false
    var published = false

    while (System.currentTimeMillis() - startTime < RUN_DURATION_MS) {
        // Wait up to one tick interval for incoming data
        selector.select(TICK_INTERVAL_MS)

        // Drain all received datagrams
        recvBuf.clear()
        while (channel.receive(recvBuf) != null) {
            recvBuf.flip()
            val data = ByteArray(recvBuf.limit())
            recvBuf.get(data)
            engine.handleDatagram(data, serverAddrStr, nowMs(startTime))
            recvBuf.clear()
        }

        // Tick the engine (drives QUIC timers and MQTT keepalive)
        val events = engine.handleTick(nowMs(startTime))

        // Process events
        for (event in events) {
            when (event) {
                is MqttEventFfi.Connected -> {
                    println("Connected! sessionPresent=${event.v1.sessionPresent}")

                    if (!subscribed) {
                        val pid = engine.subscribe("test/kotlin/quic", 1.toUByte())
                        println("Subscribed to 'test/kotlin/quic' (PID $pid)")
                        subscribed = true
                        // Send the SUBSCRIBE packet immediately
                        sendOutgoing(engine, channel)
                    }
                }
                is MqttEventFfi.Subscribed -> {
                    println("Subscribe ack PID ${event.v1.packetId}")
                    
                    // Publish after subscription is confirmed
                    if (!published) {
                        val payload = "Hello from Kotlin QUIC!".toByteArray(StandardCharsets.UTF_8)
                        val pid = engine.publish("test/kotlin/quic", payload, 1.toUByte())
                        println("Published to 'test/kotlin/quic' (PID $pid)")
                        published = true
                        // Tick immediately to generate QUIC frames for the PUBLISH packet
                        engine.handleTick(nowMs(startTime))
                        // Send the PUBLISH packet immediately
                        sendOutgoing(engine, channel)
                    }
                }
                is MqttEventFfi.MessageReceived -> {
                    val msg = String(event.v1.payload, StandardCharsets.UTF_8)
                    println("✅ Message on '${event.v1.topic}': $msg")
                }
                is MqttEventFfi.Published -> println("✅ Publish ack PID ${event.v1.packetId}")
                is MqttEventFfi.Disconnected -> println("⚠️ Disconnected. reasonCode=${event.reasonCode}")
                is MqttEventFfi.Error -> println("❌ Error: ${event.message}")
                else -> Unit  // Ignore other events
            }
        }

        // Forward any engine-generated outgoing datagrams
        sendOutgoing(engine, channel)
    }

    // 7. Graceful shutdown
    println("Run time elapsed, disconnecting...")
    engine.disconnect()
    sendOutgoing(engine, channel) // flush final QUIC close frames

    selector.close()
    channel.close()
    println("Done.")
}

/** Calculate milliseconds elapsed since startTime (relative time for engine) */
private fun nowMs(startTime: Long): ULong = (System.currentTimeMillis() - startTime).toULong()

/** Drain all pending outgoing datagrams from the engine and send them over UDP. */
private fun sendOutgoing(engine: QuicMqttEngineFfi, channel: DatagramChannel) {
    for (dgram in engine.takeOutgoingDatagrams()) {
        channel.write(ByteBuffer.wrap(dgram.data))
    }
}
