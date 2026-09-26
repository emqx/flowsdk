"""Native boundary, planned restart, host recovery and storage regressions."""
import asyncio
import importlib.util
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest

if importlib.util.find_spec("flowsdk") is not None:
    import flowsdk
else:
    flowsdk = None

DURABLE_SESSION = flowsdk is not None and hasattr(flowsdk, "inspect_session_state")


async def packet(reader):
    header = (await reader.readexactly(1))[0]
    size, shift = 0, 0
    while True:
        digit = (await reader.readexactly(1))[0]
        size |= (digit & 127) << shift
        if not digit & 128:
            break
        shift += 7
    return header, await reader.readexactly(size)


@unittest.skipIf(flowsdk is None, "Build bindings and set PYTHONPATH=python/package")
class SessionSyncTests(unittest.TestCase):
    def client(self, peer="tcp://localhost:1883", **options):
        return flowsdk.FlowMqttClient("restart", clean_start=False,
            runtime_options=flowsdk.RuntimeOptions(peer=peer),
            connect_properties=flowsdk.ConnectProperties(session_expiry_interval=3600), **options)

    def test_store_lifecycle_native_errors_and_corruption(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "state.sqlite"
            with flowsdk.SqliteSessionStore(path) as store:
                self.assertIsNone(store.resume("key"))
                with self.assertRaises(KeyError): store.update("key", b"missing")
                store.create("key", b"first")
                with self.assertRaises(sqlite3.IntegrityError): store.create("key", b"duplicate")
                self.assertEqual(store.resume("key"), b"first")
                store.update("key", b"corrupt")
            with flowsdk.SqliteSessionStore(path) as store:
                saved = store.resume("key")
                error = (flowsdk.MqttErrorFfi.InvalidArgument if DURABLE_SESSION
                         else flowsdk.MqttErrorFfi.Unsupported)
                with self.assertRaises(error):
                    self.client(session_state=saved)
                self.assertEqual(store.resume("key"), b"corrupt")
                store.delete("key"); store.delete("key")
                self.assertIsNone(store.resume("key"))
            path.write_bytes(b"invalid database")
            with self.assertRaises(sqlite3.DatabaseError): flowsdk.SqliteSessionStore(path)

    @unittest.skipUnless(DURABLE_SESSION, "Requires durable-session feature")
    def test_two_process_disk_checkpoint_with_binary_payload(self):
        script = '''
import sys
from flowsdk import *
c = FlowMqttClient("process-restart", clean_start=False,
    runtime_options=RuntimeOptions(peer="tcp://fixture:1883"),
    connect_properties=ConnectProperties(session_expiry_interval=3600))
with SqliteSessionStore(sys.argv[1]) as store:
    if sys.argv[2] == "save":
        c.engine.connect_checked(); c.engine.take_outgoing()
        c.engine.handle_incoming(b"\\x20\\x03\\x00\\x00\\x00")
        pid = c.engine.publish("test", b"\\x00\\xff", 1, None)
        c.engine.take_outgoing()
        store.create("session", c.engine.snapshot_session())
        print(pid)
    else:
        c.engine.restore_session_state(store.resume("session"))
        c.engine.connect_checked(); c.engine.take_outgoing()
        c.engine.handle_incoming(b"\\x20\\x03\\x01\\x00\\x00")
        wire = bytes(c.engine.take_outgoing())
        assert wire[0] == 0x3a and wire.endswith(b"\\x00\\xff")
        topic_size = int.from_bytes(wire[2:4], "big")
        print(int.from_bytes(wire[4+topic_size:6+topic_size], "big"))
'''
        with tempfile.TemporaryDirectory() as directory:
            path = str(Path(directory) / "state.sqlite")
            saved = subprocess.run([sys.executable, "-c", script, path, "save"], check=True, capture_output=True, text=True)
            resumed = subprocess.run([sys.executable, "-c", script, path, "resume"], check=True, capture_output=True, text=True)
            self.assertEqual(saved.stdout, resumed.stdout)
            self.assertGreater(int(saved.stdout), 0)

    def test_operation_failure_only_resolves_matching_waiter(self):
        async def run():
            client = self.client()
            client.engine.connect_checked(); client.engine.take_outgoing()
            client.engine.handle_incoming(b"\x20\x03\x00\x00\x00")
            client.engine.take_events()
            protocol = flowsdk.FlowMqttProtocol(client.engine, flowsdk.TransportType.TCP,
                asyncio.get_running_loop(), client._on_event)
            client.protocol = protocol
            first = asyncio.create_task(client.publish("test", b"first", 1, timeout=None))
            second = asyncio.create_task(client.publish("test", b"second", 1, timeout=None))
            await asyncio.sleep(0)
            a, b = client._pending_publish
            events = []; client.on_event = events.append
            error = flowsdk.MqttEventFfi.OPERATION_FAILED(operation=flowsdk.MqttOperationKindFfi.PUBLISH,
                packet_id=a, kind=flowsdk.MqttFailureKindFfi.TIMEOUT, detail="deadline", timeout_ms=10)
            client._on_event(error)
            with self.assertRaises(flowsdk.OperationFailedError) as caught: await first
            self.assertEqual(caught.exception.packet_id, a)
            self.assertTrue(client.is_connected); self.assertFalse(second.done())
            client.engine.handle_incoming(bytes([0x40, 2, a >> 8, a & 255]))
            protocol.pump()  # A late ACK is observable even without a waiter.
            self.assertTrue(any(e.is_published() and e[0].packet_id == a for e in events))
            client.engine.handle_incoming(bytes([0x40, 2, b >> 8, b & 255]))
            protocol.pump()
            self.assertEqual(await second, b)
            await client.disconnect()
        asyncio.run(run())

    def test_reconnect_preserves_publish_id(self):
        self._check_recovery_preserves_publish_id(restart=False)

    @unittest.skipUnless(DURABLE_SESSION, "Requires durable-session feature")
    def test_checkpoint_preserves_publish_id(self):
        self._check_recovery_preserves_publish_id(restart=True)

    def _check_recovery_preserves_publish_id(self, restart):
        async def run(restart):
            received = asyncio.Event()
            wires, handlers = [], set()
            connections = 0

            async def serve(reader, writer):
                nonlocal connections
                handlers.add(asyncio.current_task())
                connections += 1
                attempt = connections
                try:
                    header, _ = await packet(reader)
                    self.assertEqual(header, 0x10)
                    writer.write(bytes([0x20, 3, int(attempt > 1), 0, 0])); await writer.drain()
                    while True:
                        header, body = await packet(reader)
                        if header == 0xe0: break
                        self.assertEqual(header >> 4, 3)
                        topic_size = int.from_bytes(body[:2], "big")
                        pid = int.from_bytes(body[2+topic_size:4+topic_size], "big")
                        wires.append((header, pid, body[-2:])); received.set()
                        if attempt == 1:
                            if not restart: break
                        else:
                            writer.write(bytes([0x40, 2, pid >> 8, pid & 255])); await writer.drain()
                except asyncio.IncompleteReadError:
                    pass
                finally:
                    writer.close(); await writer.wait_closed()
                    handlers.discard(asyncio.current_task())

            server = await asyncio.start_server(serve, "127.0.0.1", 0)
            port = server.sockets[0].getsockname()[1]
            peer = f"tcp://127.0.0.1:{port}"
            client = self.client(peer, auto_reconnect=not restart, reconnect_base_delay_ms=1,
                                 reconnect_max_delay_ms=2, max_reconnect_attempts=3)
            replacement = None
            try:
                await client.connect("127.0.0.1", port)
                publish = asyncio.create_task(client.publish("test", b"\x00\xff", 1, timeout=None))
                await asyncio.wait_for(received.wait(), 2)
                if restart:
                    saved = await client.checkpoint_for_restart()
                    with self.assertRaises(ConnectionError): await publish
                    with self.assertRaises(ConnectionError): await client.connect("127.0.0.1", port)
                    completed = asyncio.Event()
                    replacement = self.client(peer, session_state=saved,
                        on_event=lambda e: completed.set() if e.is_published() else None)
                    with self.assertRaises(ValueError): await replacement.connect("localhost", port)
                    await replacement.connect("127.0.0.1", port)
                    await asyncio.wait_for(completed.wait(), 2)
                else:
                    self.assertEqual(await asyncio.wait_for(publish, 2), wires[0][1])
                self.assertEqual(len(wires), 2)
                self.assertEqual(wires[0][1:], wires[1][1:])
                self.assertEqual(wires[1][0], 0x3a)
            finally:
                if replacement: await replacement.disconnect()
                await client.disconnect()
                server.close(); await server.wait_closed()
                if handlers: await asyncio.wait_for(asyncio.gather(*handlers), 2)
        asyncio.run(run(restart))

    @unittest.skipIf(DURABLE_SESSION, "Exercises a build without durable-session")
    def test_disabled_checkpoint_apis_and_non_destructive_errors(self):
        self.assertFalse(hasattr(flowsdk, "MqttSessionInfoFfi"))
        for engine in (flowsdk.MqttEngineFfi, flowsdk.TlsMqttEngineFfi,
                       getattr(flowsdk, "QuicMqttEngineFfi", None)):
            if engine is not None:
                self.assertFalse(hasattr(engine, "snapshot_session"))
                self.assertFalse(hasattr(engine, "restore_session_state"))
        with self.assertRaises(flowsdk.MqttErrorFfi.Unsupported):
            self.client(session_state=b"checkpoint")

        async def run():
            client = self.client()
            client.engine.connect_checked(); client.engine.take_outgoing()
            client.engine.handle_incoming(b"\x20\x03\x00\x00\x00")
            client.protocol = flowsdk.FlowMqttProtocol(client.engine, flowsdk.TransportType.TCP,
                asyncio.get_running_loop(), client._on_event)
            self.assertTrue(client.is_connected)
            with self.assertRaises(flowsdk.MqttErrorFfi.Unsupported):
                await client.checkpoint_for_restart()
            self.assertFalse(client._retired)
            self.assertTrue(client.is_connected)
            await client.disconnect()
        asyncio.run(run())

    def test_options_checkpoint_identity_and_core_deadline(self):
        # Missing identity must be caught before attempting decode/transport startup.
        error = ValueError if DURABLE_SESSION else flowsdk.MqttErrorFfi.Unsupported
        with self.assertRaises(error): flowsdk.FlowMqttClient("test", session_state=b"bad")
        client = flowsdk.FlowMqttClient("test", runtime_options=flowsdk.RuntimeOptions(
            operation_timeouts=flowsdk.OperationTimeouts(connect_ms=0)))
        client.engine.connect_checked(); client.engine.handle_tick(client.engine.elapsed_ms() + 1)
        self.assertTrue(any(e.is_operation_failed() and e.operation == flowsdk.MqttOperationKindFfi.CONNECT
                            for e in client.engine.take_events()))
        self.assertEqual(flowsdk.OperationTimeouts.cloud().connect_ms, 30_000)
