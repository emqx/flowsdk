"""Broker-free regression tests for the handwritten asyncio wrapper."""

import asyncio
from contextlib import contextmanager
import importlib
import importlib.util
from pathlib import Path
import socket
import sys
from types import ModuleType, SimpleNamespace
import unittest
from unittest.mock import Mock, patch


spec = importlib.util.spec_from_file_location(
    "flowsdk_async_client_tests",
    Path(__file__).resolve().parents[1] / "package/flowsdk/__init__.py",
)
package = importlib.util.module_from_spec(spec)
stub_ffi = ModuleType("flowsdk_ffi")
stub_ffi.__all__ = []
# These tests exercise the wrapper without requiring a generated native library.
with patch.dict(sys.modules, {
    spec.name: package,
    spec.name + ".flowsdk_ffi": stub_ffi,
}):
    spec.loader.exec_module(package)
    async_client = importlib.import_module(spec.name + ".async_client")


class Event:
    def __init__(self, kind, result):
        self.kind = kind
        self.result = result

    def __getattr__(self, name):
        if name.startswith("is_"):
            return lambda: name[3:] == self.kind
        return getattr(self.result, name)

    def __getitem__(self, index):
        if index != 0:
            raise IndexError(index)
        return self.result


class ClientTestCase(unittest.TestCase):
    def setUp(self):
        self.engine = Mock()
        self.engine.publish.return_value = 7
        self.engine.subscribe.return_value = 7
        self.engine.unsubscribe.return_value = 7
        self.engine.elapsed_ms.return_value = 0
        self.engine.is_connected.return_value = True
        self.engine.take_events.return_value = []
        self.engine.take_outgoing.return_value = b""
        self.engine.take_socket_data.return_value = b""
        self.engine.take_outgoing_datagrams.return_value = []
        self.engine.next_tick_ms.return_value = -1
        ffi = SimpleNamespace(
            QuicZeroRttStatusFfi=SimpleNamespace(ATTEMPTED="attempted"),
            MqttPublishOptionsFfi=SimpleNamespace,
            MqttSubscribeOptionsFfi=SimpleNamespace,
            MqttUnsubscribeOptionsFfi=SimpleNamespace,
            MqttEventFfi=SimpleNamespace(RECONNECT_SCHEDULED=lambda **kw: Event("reconnect_scheduled", SimpleNamespace(**kw))),
            MqttOptionsFfi=SimpleNamespace,
            MqttTlsOptionsFfi=SimpleNamespace,
            MqttEngineFfi=SimpleNamespace(new_with_opts=lambda opts: self.engine),
            TlsMqttEngineFfi=lambda *args: self.engine,
            QuicMqttEngineFfi=lambda opts: self.engine,
        )
        patcher = patch.object(async_client, "flowsdk_ffi", ffi)
        patcher.start()
        self.addCleanup(patcher.stop)

    def make_client(self, transport=async_client.TransportType.TCP, mqtt_version=5):
        return async_client.FlowMqttClient(
            "test_client", transport=transport, mqtt_version=mqtt_version,
            server_name="localhost",
        )


class AcknowledgementTests(ClientTestCase):
    def test_connect_rejects_failed_connack(self):
        async def run(mqtt_version, reason_code):
            client = self.make_client(mqtt_version=mqtt_version)
            event = Event("connected", SimpleNamespace(
                reason_code=reason_code, session_present=False,
            ))
            loop = asyncio.get_running_loop()

            async def create_connection(factory, host, port):
                protocol = factory()
                loop.call_soon(protocol.on_event_cb, event)
                return Mock(), protocol

            with patch.object(loop, "create_connection", create_connection):
                with self.assertRaisesRegex(ConnectionError, f"0x{reason_code:02x}"):
                    await client.connect("localhost", 1883)

        for mqtt_version, reason_code in ((3, 5), (5, 0x87)):
            with self.subTest(mqtt_version=mqtt_version):
                asyncio.run(run(mqtt_version, reason_code))

    def test_connect_accepts_successful_connack(self):
        async def run():
            client = self.make_client()
            event = Event("connected", SimpleNamespace(
                reason_code=0, session_present=True,
            ))
            loop = asyncio.get_running_loop()

            async def create_connection(factory, host, port):
                protocol = factory()
                loop.call_soon(protocol.on_event_cb, event)
                return Mock(), protocol

            with patch.object(loop, "create_connection", create_connection):
                self.assertIsNone(await client.connect("localhost", 1883))

        asyncio.run(run())

    async def exercise_operation(self, transport, operation, result, rejected):
        client = self.make_client(transport)
        kind = {
            "publish": "published",
            "subscribe": "subscribed",
            "unsubscribe": "unsubscribed",
        }[operation]
        event = Event(kind, result)
        client.protocol = SimpleNamespace(closed=False, pump=lambda: client._on_event(event))
        args = {
            "publish": ("test/topic", b"payload", result.qos)
            if operation == "publish" else (),
            "subscribe": ("test/topic", 1),
            "unsubscribe": ("test/topic",),
        }[operation]
        pending = getattr(client, "_pending_" + operation)

        if operation != "publish":
            actual = await getattr(client, operation)(*args)
            result_type = {
                "subscribe": package.SubscribeResult,
                "unsubscribe": package.UnsubscribeResult,
            }[operation]
            self.assertIsInstance(actual, result_type)
            self.assertEqual(actual.packet_id, 7)
            self.assertEqual(actual.reason_codes, result.reason_codes)
            self.assertEqual(actual.is_failure, rejected)
            self.assertEqual(actual.is_success, not rejected)
            if rejected:
                with self.assertRaises(package.MqttAckError) as caught:
                    actual.raise_for_status()
                self.assertIs(caught.exception.result, actual)
                self.assertIn("packet 7", str(caught.exception))
                for code in result.reason_codes:
                    self.assertIn(f"0x{code:02x}", str(caught.exception))
            else:
                self.assertIsNone(actual.raise_for_status())
        elif rejected:
            with self.assertRaisesRegex(RuntimeError, "0x87"):
                await getattr(client, operation)(*args)
        else:
            self.assertEqual(await getattr(client, operation)(*args), 7)
        self.assertEqual(pending, {})
        # Late or duplicate acknowledgements must not settle a finished future.
        client._on_event(event)

    def test_publish_rejects_failed_acknowledgements(self):
        for transport in async_client.TransportType:
            for qos in (1, 2):
                with self.subTest(transport=transport, qos=qos):
                    result = SimpleNamespace(packet_id=7, reason_code=0x87, qos=qos)
                    asyncio.run(self.exercise_operation(transport, "publish", result, True))

    def test_publish_accepts_success_and_no_matching_subscribers(self):
        for reason_code in (None, 0, 0x10):
            with self.subTest(reason_code=reason_code):
                result = SimpleNamespace(packet_id=7, reason_code=reason_code, qos=1)
                asyncio.run(self.exercise_operation(
                    async_client.TransportType.TCP, "publish", result, False,
                ))

    def test_subscribe_returns_failed_and_mixed_acknowledgements(self):
        for transport in async_client.TransportType:
            for reason_codes in ([0x80], [0x87], [0, 0x87], [0x87, 2]):
                with self.subTest(transport=transport, reason_codes=reason_codes):
                    result = SimpleNamespace(packet_id=7, reason_codes=reason_codes)
                    asyncio.run(self.exercise_operation(transport, "subscribe", result, True))

    def test_subscribe_accepts_granted_qos(self):
        for qos in (0, 1, 2):
            with self.subTest(qos=qos):
                result = SimpleNamespace(packet_id=7, reason_codes=[qos])
                asyncio.run(self.exercise_operation(
                    async_client.TransportType.TCP, "subscribe", result, False,
                ))

    def test_unsubscribe_returns_failed_and_mixed_acknowledgements(self):
        for transport in async_client.TransportType:
            for reason_codes in ([0x80], [0x87], [0, 0x11, 0x87]):
                with self.subTest(transport=transport, reason_codes=reason_codes):
                    result = SimpleNamespace(packet_id=7, reason_codes=reason_codes)
                    asyncio.run(self.exercise_operation(transport, "unsubscribe", result, True))

    def test_unsubscribe_accepts_success_and_missing_subscription(self):
        for reason_codes in ([], [0], [0x11], [0, 0x11]):
            with self.subTest(reason_codes=reason_codes):
                result = SimpleNamespace(packet_id=7, reason_codes=reason_codes)
                asyncio.run(self.exercise_operation(
                    async_client.TransportType.TCP, "unsubscribe", result, False,
                ))

    def test_local_queue_failures_raise_without_waiting_for_ack(self):
        async def run(transport, operation):
            client = self.make_client(transport)
            client.protocol = SimpleNamespace(closed=False, pump=Mock())
            getattr(self.engine, operation).return_value = -1
            with self.assertRaisesRegex(RuntimeError, "could not be queued"):
                await asyncio.wait_for(getattr(client, operation)("test/topic"), 0.1)
            self.assertEqual(getattr(client, "_pending_" + operation), {})
            client.protocol.pump.assert_not_called()

        for transport in async_client.TransportType:
            for operation in ("subscribe", "unsubscribe"):
                with self.subTest(transport=transport, operation=operation):
                    asyncio.run(run(transport, operation))

    def test_disconnected_operations_raise_connection_error(self):
        async def run(operation):
            client = self.make_client()
            with self.assertRaises(ConnectionError):
                await getattr(client, operation)("test/topic")
            getattr(self.engine, operation).assert_not_called()

        for operation in ("subscribe", "unsubscribe"):
            with self.subTest(operation=operation):
                asyncio.run(run(operation))

    def test_acknowledgement_types_are_exported(self):
        for name in ("SubscribeResult", "UnsubscribeResult", "MqttAckError"):
            with self.subTest(name=name):
                self.assertIn(name, package.__all__)
                self.assertTrue(isinstance(getattr(package, name), type))


class LifecycleTests(ClientTestCase):
    operations = (
        ("publish", ("test/topic", b"payload", 1)),
        ("publish", ("test/topic", b"payload", 2)),
        ("subscribe", ("test/topic", 1)),
        ("unsubscribe", ("test/topic",)),
    )

    @contextmanager
    def endpoints(self, connack=0, pause_creation=False, setup_error=None, address=("127.0.0.1", 1883)):
        loop = asyncio.get_running_loop()
        state = SimpleNamespace(
            protocol=None, transport=Mock(), timer=None, created=asyncio.Event(),
        )
        state.transport.is_closing.return_value = False
        state.transport.get_extra_info.return_value = ("127.0.0.1", 1883)

        async def create_endpoint(factory, *args, **kwargs):
            state.endpoint_kwargs = kwargs
            protocol = state.protocol = factory()
            protocol.connection_made(state.transport)
            state.timer = protocol._tick_handle
            state.created.set()
            if pause_creation:
                await loop.create_future()
            if setup_error:
                raise setup_error
            if connack is not None:
                loop.call_soon(protocol.on_event_cb, Event("connected", SimpleNamespace(
                    reason_code=connack, session_present=False,
                )))
            return state.transport, protocol

        async def getaddrinfo(*args, **kwargs):
            state.resolve_kwargs = kwargs
            family = socket.AF_INET6 if len(address) == 4 else socket.AF_INET
            return [(family, socket.SOCK_DGRAM, None, None, address)]

        with patch.object(loop, "create_connection", create_endpoint), \
                patch.object(loop, "create_datagram_endpoint", create_endpoint), \
                patch.object(loop, "getaddrinfo", getaddrinfo):
            yield state

    def test_loss_starts_one_reconnect_and_disconnect_stops_retries(self):
        async def run(transport):
            client = self.make_client(transport)
            client.auto_reconnect = True
            client.opts.reconnect_base_delay_ms = 1
            client.opts.reconnect_max_delay_ms = 2
            with self.endpoints() as first:
                await client.connect("localhost", 1883)
                first.protocol.connection_lost(None)
                retry = client._reconnect_task
                first.protocol.connection_lost(None)
                self.assertIs(client._reconnect_task, retry)
            with self.endpoints() as second:
                await asyncio.wait_for(retry, 1)
                self.assertTrue(client.is_connected)
                self.assertIsNot(first.protocol, second.protocol)
                second.protocol.connection_lost(None)
                await client.disconnect()
                self.assertIsNone(client._reconnect_task)
                self.assertIsNone(client._reconnect_target)
                self.assertFalse(client.is_connected)
        for transport in async_client.TransportType:
            asyncio.run(run(transport))

    def test_reconnect_backoff_caps_and_stops_after_max_attempts(self):
        async def run():
            client = self.make_client()
            client.auto_reconnect = True
            client.opts.reconnect_base_delay_ms = 1
            client.opts.reconnect_max_delay_ms = 2
            client.opts.max_reconnect_attempts = 3
            events = []
            client.on_event = events.append
            with self.endpoints() as state:
                await client.connect("localhost", 1883)
                state.protocol.connection_lost(None)
                retry = client._reconnect_task
            with self.endpoints(setup_error=OSError("offline")):
                await asyncio.wait_for(retry, 1)
            self.assertEqual([event.delay_ms for event in events if event.is_reconnect_scheduled()], [1, 2, 2])
            self.assertIsInstance(client.reconnect_error, OSError)
            self.assertIsNone(client._reconnect_task)
            self.assertIsNone(client.protocol)
            await client.disconnect()
        asyncio.run(run())

    def test_quic_controls_route_operations_and_stop_on_intentional_close(self):
        async def run():
            client = self.make_client(async_client.TransportType.QUIC)
            client.auto_reconnect = True
            self.engine.open_data_stream.return_value = 12
            self.engine.publish_on.return_value = 7
            self.engine.subscribe_on.return_value = 7
            self.engine.unsubscribe_on.return_value = 7
            with self.endpoints() as state:
                await client.connect("localhost", 14567)
                self.assertEqual(client.quic.open_stream(), 12)
                state.protocol.pump = lambda: client._on_event(self.acknowledgement("publish"))
                await client.quic.publish(12, "topic", b"payload", qos=1)
                self.assertEqual(self.engine.publish_on.call_args.args[:3], (12, "topic", b"payload"))
                state.protocol.pump = lambda: client._on_event(self.acknowledgement("subscribe"))
                await client.quic.subscribe(12, [SimpleNamespace(topic_filter="topic")])
                self.assertEqual(self.engine.subscribe_on.call_args.args[0], 12)
                state.protocol.pump = lambda: client._on_event(self.acknowledgement("unsubscribe"))
                await client.quic.unsubscribe(12, ["topic"])
                self.assertEqual(self.engine.unsubscribe_on.call_args.args[0], 12)
                client.quic.set_stream_priority(12, 240)
                self.engine.set_stream_priority.assert_called_with(12, 240)
                client.quic.finish_stream(12)
                client.quic.reset_stream(12, 2)
                client.quic.stop_stream(12, 3)
                client.quic.ping()
                client.quic.notify_local_address_changed()
                client.quic.clear_session_cache()
                self.engine.finish_stream.assert_called_with(12)
                self.engine.reset_stream.assert_called_with(12, 2)
                self.engine.stop_stream.assert_called_with(12, 3)
                self.engine.quic_ping.assert_called_once()
                self.engine.notify_local_address_changed.assert_called_once()
                self.engine.clear_session_cache.assert_called_once()
                client.quic.close_transport(4, b"done")
                self.engine.close_transport.assert_called_with(4, b"done")
                self.assertIsNone(client._reconnect_task)
                self.assertIsNone(client._reconnect_target)
                self.assert_closed(client, state)
        asyncio.run(run())

    def test_early_publish_requires_explicit_opt_in_and_available_early_keys(self):
        async def run():
            client = self.make_client(async_client.TransportType.QUIC)
            self.engine.is_connected.return_value = False
            self.engine.zero_rtt_status.return_value = "unavailable"
            client.protocol = SimpleNamespace(closed=False, pump=Mock())
            with self.assertRaises(ConnectionError):
                await client.publish("early", b"data", early_data=True)
            self.engine.zero_rtt_status.return_value = "attempted"
            with self.assertRaises(ConnectionError):
                await client.publish("early", b"data")
            await client.publish("early", b"data", early_data=True)
            self.engine.publish.assert_called_once()
        asyncio.run(run())

    def test_quic_connect_uses_an_unconnected_ipv6_endpoint(self):
        async def run():
            client = self.make_client(async_client.TransportType.QUIC)
            with self.endpoints(address=("::1", 14567, 0, 0)) as state:
                await client.connect("localhost", 14567)
                self.assertEqual(state.resolve_kwargs["family"], socket.AF_UNSPEC)
                self.assertEqual(state.endpoint_kwargs, {"family": socket.AF_INET6, "local_addr": ("::", 0)})
                self.engine.connect.assert_called_with("[::1]:14567", "localhost", client.tls_opts, 0)
                await client.disconnect()
        asyncio.run(run())

    def test_disconnect_cancels_connection_during_dns(self):
        async def run():
            client = self.make_client(async_client.TransportType.QUIC)
            loop = asyncio.get_running_loop()
            started = asyncio.Event()
            async def resolve(*args, **kwargs):
                started.set()
                await loop.create_future()
            with patch.object(loop, "getaddrinfo", resolve):
                task = asyncio.create_task(client.connect("localhost", 1883, timeout=None))
                await started.wait()
                await client.disconnect()
                with self.assertRaises(asyncio.CancelledError):
                    await task
            self.assertIsNone(client._connect_task)
            self.assertIsNone(client._connect_future)
            self.assertFalse(client._disconnecting)
        asyncio.run(run())

    def assert_closed(self, client, state):
        self.assertIsNone(client.protocol)
        self.assertTrue(state.protocol.closed)
        state.transport.close.assert_called_once()
        if state.timer is not None:
            self.assertTrue(state.timer.cancelled())
        self.assertIsNone(state.protocol._tick_handle)

    @staticmethod
    def acknowledgement(operation, packet_id=7):
        if operation == "publish":
            return Event("published", SimpleNamespace(
                packet_id=packet_id, qos=1, reason_code=0,
            ))
        return Event(operation + "d", SimpleNamespace(
            packet_id=packet_id, reason_codes=[0],
        ))

    def test_cancelled_operations_remove_waiters_and_ignore_late_ack(self):
        async def run(transport, operation, args):
            client = self.make_client(transport)
            client.protocol = SimpleNamespace(closed=False, pump=Mock())
            pending = getattr(client, "_pending_" + operation)
            task = asyncio.create_task(getattr(client, operation)(*args, timeout=None))
            await asyncio.sleep(0)
            self.assertEqual(list(pending), [7])
            task.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await task
            self.assertEqual(pending, {})
            client._on_event(self.acknowledgement(operation))

        for transport in async_client.TransportType:
            for operation, args in self.operations:
                with self.subTest(transport=transport, operation=operation, args=args):
                    asyncio.run(run(transport, operation, args))

    def test_operation_timeouts_remove_waiters_and_allow_later_requests(self):
        async def run(transport, operation, args):
            client = self.make_client(transport)
            client.protocol = SimpleNamespace(closed=False, pump=Mock())
            pending = getattr(client, "_pending_" + operation)
            with self.assertRaises(asyncio.TimeoutError):
                await getattr(client, operation)(*args, timeout=0.001)
            self.assertEqual(pending, {})
            client._on_event(self.acknowledgement(operation))
            getattr(self.engine, operation).return_value = 8
            task = asyncio.create_task(getattr(client, operation)(*args, timeout=None))
            await asyncio.sleep(0)
            client._on_event(self.acknowledgement(operation))
            self.assertFalse(task.done())
            client._on_event(self.acknowledgement(operation, 8))
            result = await asyncio.wait_for(task, 0.1)
            self.assertEqual(result if operation == "publish" else result.packet_id, 8)
            self.assertEqual(pending, {})
            getattr(self.engine, operation).return_value = 7

        for transport in async_client.TransportType:
            for operation, args in self.operations:
                with self.subTest(transport=transport, operation=operation, args=args):
                    asyncio.run(run(transport, operation, args))

    def test_cancelling_one_waiter_keeps_other_requests_pending(self):
        async def run():
            client = self.make_client()
            client.protocol = SimpleNamespace(closed=False, pump=Mock())
            self.engine.subscribe.side_effect = [7, 8]
            first = asyncio.create_task(client.subscribe("one", timeout=None))
            second = asyncio.create_task(client.subscribe("two", timeout=None))
            await asyncio.sleep(0)
            first.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await first
            self.assertEqual(list(client._pending_subscribe), [8])
            client._on_event(self.acknowledgement("subscribe", 7))
            self.assertFalse(second.done())
            client._on_event(self.acknowledgement("subscribe", 8))
            self.assertEqual((await asyncio.wait_for(second, 0.1)).packet_id, 8)

        asyncio.run(run())

    def test_connection_failures_finish_all_pending_operations(self):
        async def run(transport, failure):
            client = self.make_client(transport)
            with self.endpoints() as state:
                await client.connect("localhost", 1883)
                tasks = [
                    asyncio.create_task(getattr(client, operation)(*args, timeout=None))
                    for operation, args in self.operations[:1] + self.operations[2:]
                ]
                await asyncio.sleep(0)
                if failure == "transport":
                    state.protocol.connection_lost(OSError("socket failed"))
                elif failure == "eof":
                    state.protocol.connection_lost(None)
                elif failure == "disconnect":
                    await client.disconnect()
                else:
                    state.protocol.on_event_cb(Event(failure, SimpleNamespace(
                        message="engine failed", reason_code=0x80,
                    )))
                for task in tasks:
                    with self.assertRaises(ConnectionError):
                        await asyncio.wait_for(task, 0.1)
                for operation, args in self.operations:
                    self.assertEqual(getattr(client, "_pending_" + operation), {})
                    with self.assertRaises(ConnectionError):
                        await getattr(client, operation)(*args)
                self.assert_closed(client, state)
                # asyncio can report loss after an explicit close.
                state.protocol.connection_lost(None)
                state.protocol._on_timer()
                self.assert_closed(client, state)

        for transport in async_client.TransportType:
            for failure in ("transport", "eof", "error", "disconnected", "reconnect_needed", "disconnect"):
                with self.subTest(transport=transport, failure=failure):
                    asyncio.run(run(transport, failure))

    def test_failed_connect_closes_transport_and_timer(self):
        async def run(transport, failure):
            client = self.make_client(transport)
            kwargs = {}
            expected = ConnectionError
            if failure == "timeout":
                kwargs["connack"] = None
                expected = asyncio.TimeoutError
            elif failure == "setup":
                kwargs["setup_error"] = OSError("endpoint setup failed")
                expected = OSError
            elif failure == "connack":
                kwargs["connack"] = 0x87
            else:
                self.engine.connect.side_effect = OSError("engine setup failed")
                expected = OSError  # ConnectionError is also an OSError.
            with self.endpoints(**kwargs) as state:
                with self.assertRaises(expected):
                    await client.connect("localhost", 1883, timeout=0.01)
                self.assert_closed(client, state)
                self.assertIsNone(client._connect_future)
            self.engine.connect.side_effect = None

        for transport in async_client.TransportType:
            for failure in ("timeout", "setup", "connack", "engine"):
                with self.subTest(transport=transport, failure=failure):
                    asyncio.run(run(transport, failure))

    def test_cancelled_connect_closes_even_before_endpoint_creation_returns(self):
        async def run(transport, pause_creation):
            client = self.make_client(transport)
            with self.endpoints(connack=None, pause_creation=pause_creation) as state:
                task = asyncio.create_task(client.connect("localhost", 1883))
                await state.created.wait()
                task.cancel()
                with self.assertRaises(asyncio.CancelledError):
                    await task
                self.assert_closed(client, state)
                self.assertIsNone(client._connect_future)

        for transport in async_client.TransportType:
            for pause_creation in (False, True):
                with self.subTest(transport=transport, pause_creation=pause_creation):
                    asyncio.run(run(transport, pause_creation))

    def test_cancelled_dns_lookup_does_not_leave_connect_state(self):
        async def run():
            client = self.make_client(async_client.TransportType.QUIC)
            loop = asyncio.get_running_loop()
            resolving = asyncio.Event()

            async def getaddrinfo(*args, **kwargs):
                resolving.set()
                await loop.create_future()

            with patch.object(loop, "getaddrinfo", getaddrinfo):
                task = asyncio.create_task(client.connect("localhost", 1883))
                await resolving.wait()
                task.cancel()
                with self.assertRaises(asyncio.CancelledError):
                    await task
            self.assertIsNone(client.protocol)
            self.assertIsNone(client._connect_future)

        asyncio.run(run())

    def test_failed_connect_closes_real_asyncio_socket(self):
        async def run(cancel):
            client = self.make_client()
            loop = asyncio.get_running_loop()
            create_connection = loop.create_connection
            connected = asyncio.Event()
            local, peer = socket.socketpair()
            with local, peer:
                local.setblocking(False)
                peer.setblocking(False)

                async def connect_socket(factory, *args):
                    result = await create_connection(factory, sock=local)
                    connected.set()
                    return result

                with patch.object(loop, "create_connection", connect_socket):
                    task = asyncio.create_task(client.connect("localhost", 1883, timeout=0.02))
                    await connected.wait()
                    protocol = client.protocol
                    if cancel:
                        task.cancel()
                    expected = asyncio.CancelledError if cancel else asyncio.TimeoutError
                    with self.assertRaises(expected):
                        await task
                    self.assertIsNone(client.protocol)
                    self.assertTrue(protocol.closed)
                    self.assertIsNone(protocol._tick_handle)
                    # EOF proves asyncio closed the socket, beyond a mock close call.
                    self.assertEqual(await asyncio.wait_for(loop.sock_recv(peer, 1), 0.1), b"")

        for cancel in (False, True):
            with self.subTest(cancel=cancel):
                asyncio.run(run(cancel))

    def test_stale_protocol_callbacks_do_not_affect_new_connection(self):
        async def run(transport):
            client = self.make_client(transport)
            with self.endpoints(connack=None) as old:
                with self.assertRaises(asyncio.TimeoutError):
                    await client.connect("localhost", 1883, timeout=0.001)
            with self.endpoints() as current:
                await client.connect("localhost", 1883)
                task = asyncio.create_task(client.subscribe("test/topic", timeout=None))
                await asyncio.sleep(0)
                self.engine.handle_connection_lost.reset_mock()
                old.protocol.connection_lost(OSError("late close"))
                old.protocol.on_event_cb(Event("disconnected", SimpleNamespace()))
                old.protocol.on_event_cb(self.acknowledgement("subscribe"))
                self.engine.handle_connection_lost.assert_not_called()
                self.assertIs(client.protocol, current.protocol)
                self.assertFalse(task.done())
                current.protocol.on_event_cb(self.acknowledgement("subscribe"))
                self.assertEqual((await asyncio.wait_for(task, 0.1)).packet_id, 7)
                await client.disconnect()

        for transport in async_client.TransportType:
            with self.subTest(transport=transport):
                asyncio.run(run(transport))

    def test_connect_cannot_replace_active_or_connecting_transport(self):
        async def run(connected):
            client = self.make_client()
            with self.endpoints(connack=0 if connected else None) as state:
                first = asyncio.create_task(client.connect("localhost", 1883))
                await state.created.wait()
                if connected:
                    await first
                with self.assertRaisesRegex(ConnectionError, "Already connected or connecting"):
                    await client.connect("localhost", 1883)
                self.assertIs(client.protocol, state.protocol)
                if not connected:
                    first.cancel()
                    with self.assertRaises(asyncio.CancelledError):
                        await first
                await client.disconnect()
                self.assert_closed(client, state)

        for connected in (False, True):
            with self.subTest(connected=connected):
                asyncio.run(run(connected))

    def test_operations_require_engine_connection(self):
        async def run(operation, args):
            client = self.make_client()
            client.protocol = SimpleNamespace(closed=False, pump=Mock())
            self.engine.is_connected.return_value = False
            with self.assertRaises(ConnectionError):
                await getattr(client, operation)(*args)
            getattr(self.engine, operation).assert_not_called()

        for operation, args in self.operations:
            with self.subTest(operation=operation, args=args):
                asyncio.run(run(operation, args))

    def test_publish_queue_failures_do_not_wait_for_ack(self):
        async def run(transport, qos):
            client = self.make_client(transport)
            client.protocol = SimpleNamespace(closed=False, pump=Mock())
            self.engine.publish.return_value = -1
            with self.assertRaisesRegex(RuntimeError, "could not be queued"):
                await asyncio.wait_for(client.publish("test/topic", b"data", qos), 0.1)
            self.assertEqual(client._pending_publish, {})
            client.protocol.pump.assert_not_called()

        for transport in async_client.TransportType:
            for qos in (0, 1, 2):
                with self.subTest(transport=transport, qos=qos):
                    asyncio.run(run(transport, qos))

    def test_qos_zero_publish_returns_without_waiting(self):
        async def run(transport):
            client = self.make_client(transport)
            client.protocol = SimpleNamespace(closed=False, pump=Mock())
            self.engine.publish.return_value = 0
            self.assertEqual(await client.publish("test/topic", b"data", timeout=0), 0)
            self.assertEqual(client._pending_publish, {})
            client.protocol.pump.assert_called_once()

        for transport in async_client.TransportType:
            with self.subTest(transport=transport):
                asyncio.run(run(transport))

    def test_synchronous_pump_failure_removes_waiter(self):
        async def run(operation, args):
            client = self.make_client()
            client.protocol = SimpleNamespace(closed=False, pump=Mock(side_effect=OSError("write failed")))
            with self.assertRaisesRegex(OSError, "write failed"):
                await getattr(client, operation)(*args)
            self.assertEqual(getattr(client, "_pending_" + operation), {})

        for operation, args in self.operations:
            with self.subTest(operation=operation, args=args):
                asyncio.run(run(operation, args))

    def test_engine_tick_failure_finishes_waiter(self):
        async def run(transport):
            client = self.make_client(transport)
            with self.endpoints() as state:
                await client.connect("localhost", 1883)
                task = asyncio.create_task(client.subscribe("test/topic", timeout=None))
                await asyncio.sleep(0)
                self.engine.handle_tick.side_effect = OSError("tick failed")
                state.protocol._on_timer()
                with self.assertRaises(ConnectionError) as caught:
                    await asyncio.wait_for(task, 0.1)
                self.assertEqual(str(caught.exception.__cause__), "tick failed")
                self.assert_closed(client, state)
                self.engine.handle_tick.side_effect = None

        for transport in async_client.TransportType:
            with self.subTest(transport=transport):
                asyncio.run(run(transport))

    def test_disconnect_drives_output_before_closing(self):
        async def run(transport):
            client = self.make_client(transport)
            calls = []
            with self.endpoints() as state:
                await client.connect("localhost", 1883)
                state.protocol._tick_handle.cancel()
                self.engine.disconnect.side_effect = lambda: calls.append("disconnect")
                self.engine.handle_tick.side_effect = lambda now: calls.append("tick")
                state.protocol.pump = lambda: calls.append("pump")
                self.engine.disconnect_complete.side_effect = [False, True]
                state.transport.close.side_effect = lambda: calls.append("close")
                await client.disconnect()
                self.assertEqual(calls, ["disconnect", "tick", "pump", "tick", "pump", "close"])
                self.assert_closed(client, state)
            self.engine.disconnect.side_effect = None
            self.engine.handle_tick.side_effect = None
            self.engine.disconnect_complete.side_effect = None

        for transport in async_client.TransportType:
            with self.subTest(transport=transport):
                asyncio.run(run(transport))

    def test_disconnect_timeout_still_closes_transport(self):
        async def run():
            client = self.make_client(async_client.TransportType.QUIC)
            with self.endpoints() as state:
                await client.connect("localhost", 1883)
                self.engine.disconnect_complete.return_value = False
                with self.assertRaises(asyncio.TimeoutError):
                    await client.disconnect(timeout=0.001)
                self.assert_closed(client, state)
                self.assertFalse(client._disconnecting)

        asyncio.run(run())

    def test_connect_can_return_connection_details(self):
        async def run():
            client = self.make_client()
            with self.endpoints():
                result = await client.connect("localhost", 1883, return_result=True)
                self.assertIsInstance(result, package.ConnectionResult)
                self.assertEqual(result.reason_code, 0)
                self.assertIs(client.connection_result, result)
                self.assertTrue(client.is_connected)
                self.assertEqual(client.mqtt_version, 5)
                await client.disconnect()
                self.assertFalse(client.is_connected)
        asyncio.run(run())

    def test_detailed_publish_preserves_rejection_and_properties(self):
        async def run():
            client = self.make_client()
            properties = [object(), object()]
            event = Event("published", SimpleNamespace(
                packet_id=7, qos=1, reason_code=0x87, properties=properties,
            ))
            client.protocol = SimpleNamespace(closed=False, pump=lambda: client._on_event(event))
            result = await client.publish("test/topic", b"data", 1, return_result=True)
            self.assertIsInstance(result, package.PublishResult)
            self.assertEqual(result.properties, properties)
            self.assertTrue(result.is_failure)
            with self.assertRaises(package.MqttAckError) as caught:
                result.raise_for_status()
            self.assertIs(caught.exception.result, result)
        asyncio.run(run())

    def test_callback_exception_does_not_drop_following_ack(self):
        async def run():
            client = self.make_client()
            client.on_message = Mock(side_effect=ValueError("application failed"))
            client.on_message_full = Mock()
            client.on_event = Mock()
            message = Event("message_received", SimpleNamespace(topic="test/topic", payload=b"data", qos=0))
            def pump():
                client._on_event(message)
                client._on_event(self.acknowledgement("subscribe"))
            client.protocol = SimpleNamespace(closed=False, pump=pump)
            result = await client.subscribe("test/topic")
            self.assertEqual(result.packet_id, 7)
            client.on_message_full.assert_called_once_with(message[0])
            self.assertEqual(client.on_event.call_count, 2)
        asyncio.run(run())

    def test_ping_timeout_and_connection_loss_clean_waiter(self):
        async def run():
            client = self.make_client()
            with self.endpoints():
                await client.connect("localhost", 1883)
                with self.assertRaises(asyncio.TimeoutError):
                    await client.ping(timeout=0.001)
                self.assertEqual(client._pending_ping, {})
                task = asyncio.create_task(client.ping(timeout=None))
                await asyncio.sleep(0)
                with self.assertRaisesRegex(RuntimeError, "already pending"):
                    await client.ping()
                client.protocol.connection_lost(None)
                with self.assertRaises(ConnectionError):
                    await task
                self.assertEqual(client._pending_ping, {})
        asyncio.run(run())


class QuicEventDeliveryTests(unittest.TestCase):
    def make_protocol(self, tick_events, queued_events=()):
        pending_events = list(queued_events)
        engine = Mock()

        def handle_tick(now_ms):
            # The FFI both returns these events and queues copies for take_events().
            pending_events.extend(tick_events)
            return list(tick_events)

        def take_events():
            events = list(pending_events)
            pending_events.clear()
            return events

        engine.handle_tick.side_effect = handle_tick
        engine.take_events.side_effect = take_events
        engine.take_outgoing_datagrams.return_value = []
        received = []
        protocol = async_client.FlowMqttDatagramProtocol(engine, Mock(), received.append)
        return protocol, engine, received

    def test_tick_delivers_queued_and_new_events_once_in_order(self):
        queued = object()
        message = Event("message_received", SimpleNamespace(topic="test/topic", payload=b"data"))
        published = Event("published", SimpleNamespace(packet_id=7, reason_code=0))
        protocol, engine, received = self.make_protocol([message, published], [queued])
        protocol.transport = Mock()
        protocol.remote_addr = ("127.0.0.1", 14567)
        engine.take_outgoing_datagrams.side_effect = [
            [SimpleNamespace(data=b"outgoing", addr="127.0.0.2:14567")], [],
        ]

        protocol._on_timer()
        self.assertEqual(received, [queued, message, published])
        protocol.loop.call_later.assert_called_once_with(0.01, protocol._on_timer)
        protocol.pump()
        self.assertEqual(received, [queued, message, published])
        protocol.transport.sendto.assert_called_once_with(b"outgoing", ("127.0.0.2", 14567))

    def test_ipv6_datagrams_preserve_destination_and_scope(self):
        protocol, engine, received = self.make_protocol([], [])
        protocol.transport = Mock()
        engine.take_outgoing_datagrams.return_value = [
            SimpleNamespace(data=b"v6", addr="[fe80::1%3]:14567")]
        protocol.datagram_received(b"incoming", ("fe80::2", 14567, 0, 3))
        engine.handle_datagram.assert_called_once_with(b"incoming", "[fe80::2%3]:14567", engine.elapsed_ms())
        protocol.transport.sendto.assert_called_once_with(b"v6", ("fe80::1", 14567, 0, 3))

    def test_tick_preserves_distinct_events_with_identical_content(self):
        events = [
            SimpleNamespace(topic="test/topic", payload=b"same message"),
            SimpleNamespace(topic="test/topic", payload=b"same message"),
        ]
        protocol, engine, received = self.make_protocol(events)
        protocol._on_timer()
        self.assertEqual(len(received), 2)
        self.assertIs(received[0], events[0])
        self.assertIs(received[1], events[1])


class EngineClockTests(ClientTestCase):
    def test_protocol_replacement_uses_existing_engine_clock(self):
        for transport in async_client.TransportType:
            with self.subTest(transport=transport):
                self.engine.elapsed_ms.return_value = 60000
                self.engine.next_tick_ms.return_value = 60010
                loop = Mock()
                for _ in range(2):
                    if transport == async_client.TransportType.QUIC:
                        protocol = async_client.FlowMqttDatagramProtocol(self.engine, loop, Mock())
                        protocol.datagram_received(b"data", ("127.0.0.1", 1883))
                        self.engine.handle_datagram.assert_called_with(b"data", "127.0.0.1:1883", 60000)
                    else:
                        protocol = async_client.FlowMqttProtocol(self.engine, transport, loop, Mock())
                    protocol._on_timer()
                    self.engine.handle_tick.assert_called_with(60000)
                    loop.call_later.assert_called_with(0.01, protocol._on_timer)
                    protocol.close()


if __name__ == "__main__":
    unittest.main()
