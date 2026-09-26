// SPDX-License-Identifier: MPL-2.0
import kotlin.test.*
import java.nio.file.Files
import uniffi.flowsdk_ffi.*

class SessionTest {
    private fun engine(): MqttEngineFfi {
        val options = MqttOptionsFfi("kotlin-restart", 5u, false, 0u, null, null, 1000u, 60000u, 0u)
        val connect = MqttConnectOptionsFfi(options,
            listOf(MqttPropertyFfi.SessionExpiryInterval(3600u)), null, null, null)
        val runtime = MqttRuntimeOptionsFfi("tcp://fixture:1883", null, null, null, null, null, false)
        return MqttEngineFfi.newWithRuntimeOptions(connect, runtime)
    }

    @Test fun diskCheckpointRestoresOutstandingPublish() {
        val file = Files.createTempFile("flowsdk-session", ".json")
        try {
            engine().use { old ->
                old.connectChecked()
                assertFails { old.connectChecked() }
                old.takeOutgoing()
                old.handleIncoming(byteArrayOf(0x20, 3, 0, 0, 0))
                old.publishWithOptions("test", byteArrayOf(0, -1), MqttPublishOptionsFfi(1u, false, null, emptyList()))
                old.takeOutgoing()
                Files.write(file, old.snapshotSession())
            }
            val saved = Files.readAllBytes(file)
            assertEquals("kotlin-restart", inspectSessionState(saved).clientId)
            engine().use { resumed ->
                assertFails { resumed.restoreSessionState(byteArrayOf(0)) }
                resumed.restoreSessionState(saved)
                assertFails { resumed.restoreSessionState(saved) }
                resumed.connectChecked()
                resumed.takeOutgoing()
                resumed.handleIncoming(byteArrayOf(0x20, 3, 1, 0, 0))
                assertEquals(0x3a.toByte(), resumed.takeOutgoing()[0])
            }
        } finally { Files.deleteIfExists(file) }
    }
}
