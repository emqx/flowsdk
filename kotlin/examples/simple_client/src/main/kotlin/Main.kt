package example

import uniffi.flowsdk_ffi.*
import java.nio.charset.StandardCharsets
import kotlinx.coroutines.*

fun main() = runBlocking {
    println("Initializing FlowSDK Kotlin Example with Coroutines...")

    // 1. Configure Options
    val opts = MqttOptionsFfi(
        clientId = "kotlin_example_client",
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

    // 3. Connect (this is async in the engine, but we trigger it here)
    engine.connect()
    println("Connect triggered.")

    // 4. Event loop using coroutines
    val startTime = System.currentTimeMillis()
    var running = true
    var hasPublished = false

    // Run for 5 seconds using coroutines
    launch {
        while (running && System.currentTimeMillis() - startTime < 5000) {
            // Handle incoming events
            val nowMs = (System.currentTimeMillis() - startTime).toULong()
            val events = engine.handleTick(nowMs)
            for (event in events) {
                when (event) {
                    is MqttEventFfi.Connected -> {
                        println("✓ Connected! Session Present: ${event.v1.sessionPresent}")
                        
                        // Subscribe once connected
                        val subPid = engine.subscribe("test/topic", 1.toUByte())
                        println("✓ Subscribed to 'test/topic' with PID: $subPid")

                        // Publish a message (only once)
                        if (!hasPublished) {
                            val payload = "Hello from Kotlin with Coroutines!".toByteArray(StandardCharsets.UTF_8)
                            
                            val pubPid = engine.publish("test/topic", payload, 1.toUByte(), null)
                            println("✓ Published to 'test/topic' with PID: $pubPid")
                            hasPublished = true
                        }
                    }
                    is MqttEventFfi.MessageReceived -> {
                        // payload is ByteArray
                        val msg = String(event.v1.payload, StandardCharsets.UTF_8)
                        println("📨 Received Message on '${event.v1.topic}': $msg")
                    }
                    is MqttEventFfi.Published -> println("✓ Publish Ack: PID ${event.v1.packetId}")
                    is MqttEventFfi.Subscribed -> println("✓ Subscribe Ack: PID ${event.v1.packetId}")
                    is MqttEventFfi.Disconnected -> {
                        println("✗ Disconnected. Reason: ${event.reasonCode}")
                        running = false
                    }
                    else -> println("ℹ Event: $event")
                }
            }

            // Use coroutine delay instead of Thread.sleep
            delay(10)
        }
    }.join()  // Wait for the event loop coroutine to complete

    // 5. Cleanup
    engine.disconnect()
    println("Disconnected and exiting.")
}
