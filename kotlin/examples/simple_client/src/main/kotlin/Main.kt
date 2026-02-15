package example

import uniffi.flowsdk_ffi.*
import java.nio.charset.StandardCharsets

fun main() {
    println("Initializing FlowSDK Kotlin Example...")

    // 1. Configure Options
    val opts = MqttOptionsFFI(
        client_id = "kotlin_example_client",
        mqtt_version = 5.toByte(),
        clean_start = true,
        keep_alive = 60.toShort(),
        username = null,
        password = null,
        reconnect_base_delay_ms = 1000,
        reconnect_max_delay_ms = 10000,
        max_reconnect_attempts = 3
    )

    // 2. Create Engine
    val engine = MqttEngineFFI.new_with_opts(opts)
    println("Engine created.")

    // 3. Connect (this is async in the engine, but we trigger it here)
    engine.connect()
    println("Connect triggered.")

    // 4. Mimic an event loop
    // In a real application, you'd likely run this in a separate thread or coroutine
    val startTime = System.currentTimeMillis()
    var running = true

    // Run for 5 seconds
    while (System.currentTimeMillis() - startTime < 5000 && running) {
        // Handle incoming events
        val events = engine.handle_tick(System.currentTimeMillis() - startTime)
        for (event in events) {
            when (event) {
                is MqttEventFFI.Connected -> {
                    println("Connected! Session Present: ${event.res.session_present}")
                    
                    // Subscribe once connected
                    val subPid = engine.subscribe("test/topic", 1.toByte())
                    println("Subscribed to 'test/topic' with PID: $subPid")

                    // Publish a message
                    val payload = "Hello from Kotlin!".toByteArray(StandardCharsets.UTF_8)
                    // Convert ByteArray to List<Byte> - generated bindings might expect List<Byte> or RustBuffer logic handles it?
                    // Viewing generated code: `payload: List<Byte>` in MqttMessageFFI, but publish takes `payload: List<Byte>`?
                    // Let's check generated signature.
                    // The generated `publish` method takes `payload: List<Byte>`.
                    // We need to convert ByteArray to List<Byte>.
                    val payloadList = payload.toList()
                    
                    val pubPid = engine.publish("test/topic", payloadList, 1.toByte(), null)
                    println("Published to 'test/topic' with PID: $pubPid")
                }
                is MqttEventFFI.MessageReceived -> {
                    // payload is List<Byte>
                    val bytes = event.msg.payload.toByteArray()
                    val msg = String(bytes, StandardCharsets.UTF_8)
                    println("Received Message on '${event.msg.topic}': $msg")
                }
                is MqttEventFFI.Published -> println("Publish Ack: PID ${event.res.packet_id}")
                is MqttEventFFI.Subscribed -> println("Subscribe Ack: PID ${event.res.packet_id}")
                is MqttEventFFI.Disconnected -> println("Disconnected. Reason: ${event.reason_code}")
                else -> println("Event: $event")
            }
        }

        // Sleep to prevent tight loop
        Thread.sleep(10)
    }

    // 5. Cleanup
    engine.disconnect()
    println("Disconnected and exiting.")
}
