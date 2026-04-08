package example

import uniffi.flowsdk_ffi.*
import java.net.InetSocketAddress
import java.nio.ByteBuffer
import java.nio.channels.SelectionKey
import java.nio.channels.Selector
import java.nio.channels.SocketChannel
import java.nio.charset.StandardCharsets
import kotlinx.coroutines.*

private const val BROKER_HOST = "broker.emqx.io"
private const val BROKER_PORT = 1883
private const val RUN_DURATION_MS = 15_000L
private const val TICK_INTERVAL_MS = 10L

fun main() = runBlocking {
    println("Initializing FlowSDK TCP MQTT Kotlin Example...")

    // 1. Configure Options
    val opts = MqttOptionsFfi(
        clientId = "kotlin_tcp_${System.currentTimeMillis() % 100000}",
        mqttVersion = 5.toUByte(),
        cleanStart = true,
        keepAlive = 60.toUShort(),
        username = null,
        password = null,
        reconnectBaseDelayMs = 1000.toULong(),
        reconnectMaxDelayMs = 10000.toULong(),
        maxReconnectAttempts = 3.toUInt()
    )

    // 2. Create Engine
    val engine = MqttEngineFfi.newWithOpts(opts)
    println("Engine created.")

    // 3. Connect TCP socket
    val brokerAddr = InetSocketAddress(BROKER_HOST, BROKER_PORT)
    val channel = SocketChannel.open().apply {
        configureBlocking(false)
        connect(brokerAddr)
    }
    val selector = Selector.open()
    channel.register(selector, SelectionKey.OP_CONNECT or SelectionKey.OP_READ)
    
    println("Connecting to TCP broker at $BROKER_HOST:$BROKER_PORT...")

    // 4. Trigger MQTT connect (produces outgoing CONNECT packet)
    engine.connect()
    
    // 5. Event loop using coroutines
    val recvBuf = ByteBuffer.allocateDirect(65536)
    val startTime = System.currentTimeMillis()
    var subscribed = false
    var published = false
    var tcpConnected = false

    launch {
        while (System.currentTimeMillis() - startTime < RUN_DURATION_MS) {
            // Wait for I/O or tick timeout
            selector.select(TICK_INTERVAL_MS)

            val keys = selector.selectedKeys()
            for (key in keys) {
                if (key.isConnectable) {
                    if (channel.finishConnect()) {
                        println("TCP connected.")
                        key.interestOps(SelectionKey.OP_READ)
                        tcpConnected = true
                        
                        // Send initial MQTT CONNECT packet
                        val outgoing = engine.takeOutgoing()
                        if (outgoing.isNotEmpty()) {
                            channel.write(ByteBuffer.wrap(outgoing))
                        }
                    }
                }
                if (key.isReadable) {
                    // Read from TCP socket
                    recvBuf.clear()
                    val bytesRead = channel.read(recvBuf)
                    if (bytesRead > 0) {
                        recvBuf.flip()
                        val data = ByteArray(recvBuf.limit())
                        recvBuf.get(data)
                        val incomingEvents = engine.handleIncoming(data)
                        
                        // Process events from handleIncoming immediately
                        for (event in incomingEvents) {
                            when (event) {
                                is MqttEventFfi.Connected -> {
                                    println("✓ Connected! Session Present: ${event.v1.sessionPresent}")
                                    
                                    if (!subscribed) {
                                        val subPid = engine.subscribe("test/kotlin/tcp", 1.toUByte())
                                        println("✓ Subscribed to 'test/kotlin/tcp' with PID: $subPid")
                                        subscribed = true
                                    }
                                }
                                is MqttEventFfi.Subscribed -> {
                                    println("✓ Subscribe Ack: PID ${event.v1.packetId}")
                                    
                                    if (!published) {
                                        val payload = "Hello from Kotlin TCP!".toByteArray(StandardCharsets.UTF_8)
                                        val pubPid = engine.publish("test/kotlin/tcp", payload, 1.toUByte(), null)
                                        println("✓ Published to 'test/kotlin/tcp' with PID: $pubPid")
                                        published = true
                                    }
                                }
                                is MqttEventFfi.MessageReceived -> {
                                    val msg = String(event.v1.payload, StandardCharsets.UTF_8)
                                    println("📨 Received Message on '${event.v1.topic}': $msg")
                                }
                                is MqttEventFfi.Published -> println("✓ Publish Ack: PID ${event.v1.packetId}")
                                is MqttEventFfi.Disconnected -> println("✗ Disconnected. Reason: ${event.reasonCode}")
                                is MqttEventFfi.Error -> println("✗ Error: ${event.message}")
                                else -> Unit
                            }
                        }
                    } else if (bytesRead < 0) {
                        println("TCP connection closed by broker")
                        break
                    }
                }
            }
            keys.clear()

            // Handle tick
            val nowMs = (System.currentTimeMillis() - startTime).toULong()
            val events = engine.handleTick(nowMs)
            
            for (event in events) {
                when (event) {
                    is MqttEventFfi.Connected -> {
                        println("✓ Connected! Session Present: ${event.v1.sessionPresent}")
                        
                        if (!subscribed) {
                            val subPid = engine.subscribe("test/kotlin/tcp", 1.toUByte())
                            println("✓ Subscribed to 'test/kotlin/tcp' with PID: $subPid")
                            subscribed = true
                        }
                    }
                    is MqttEventFfi.Subscribed -> {
                        println("✓ Subscribe Ack: PID ${event.v1.packetId}")
                        
                        // Publish after subscription is confirmed
                        if (!published) {
                            val payload = "Hello from Kotlin TCP!".toByteArray(StandardCharsets.UTF_8)
                            val pubPid = engine.publish("test/kotlin/tcp", payload, 1.toUByte(), null)
                            println("✓ Published to 'test/kotlin/tcp' with PID: $pubPid")
                            published = true
                        }
                    }
                    is MqttEventFfi.MessageReceived -> {
                        val msg = String(event.v1.payload, StandardCharsets.UTF_8)
                        println("📨 Received Message on '${event.v1.topic}': $msg")
                    }
                    is MqttEventFfi.Published -> println("✓ Publish Ack: PID ${event.v1.packetId}")
                    is MqttEventFfi.Disconnected -> {
                        println("✗ Disconnected. Reason: ${event.reasonCode}")
                    }
                    is MqttEventFfi.Error -> println("✗ Error: ${event.message}")
                    else -> Unit
                }
            }

            // Send any outgoing data (only after TCP connection is established)
            if (tcpConnected) {
                val outgoing = engine.takeOutgoing()
                if (outgoing.isNotEmpty()) {
                    channel.write(ByteBuffer.wrap(outgoing))
                }
            }

            // Use coroutine delay
            delay(10)
        }
    }.join()

    // 6. Cleanup
    println("Run time elapsed, disconnecting...")
    engine.disconnect()
    val finalData = engine.takeOutgoing()
    if (finalData.isNotEmpty()) {
        channel.write(ByteBuffer.wrap(finalData))
    }
    
    selector.close()
    channel.close()
    println("Done.")
}
