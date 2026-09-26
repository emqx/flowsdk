// SPDX-License-Identifier: MPL-2.0
import kotlin.test.*
import uniffi.flowsdk_ffi.*

class RuntimeTest {
    @Test fun inMemorySessionResumesWithoutCheckpoint() {
        val options = MqttOptionsFfi("kotlin-reconnect", 5u, false, 0u, null, null, 1000u, 60000u, 0u)
        val connect = MqttConnectOptionsFfi(options,
            listOf(MqttPropertyFfi.SessionExpiryInterval(3600u)), null, null, null)
        val runtime = MqttRuntimeOptionsFfi(null, null, null, null, null, null, false)
        MqttEngineFfi.newWithRuntimeOptions(connect, runtime).use { engine ->
            engine.connectChecked()
            assertFails { engine.connectChecked() }
            engine.takeOutgoing()
            engine.handleIncoming(byteArrayOf(0x20, 3, 0, 0, 0))
            engine.publishWithOptions("test", byteArrayOf(0, -1),
                MqttPublishOptionsFfi(1u, false, null, emptyList()))
            engine.takeOutgoing()
            engine.handleConnectionLost()
            engine.connectChecked()
            engine.takeOutgoing()
            engine.handleIncoming(byteArrayOf(0x20, 3, 1, 0, 0))
            assertEquals(0x3a.toByte(), engine.takeOutgoing()[0])
        }
    }
}
