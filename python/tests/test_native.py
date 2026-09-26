"""Broker-free tests against freshly generated or installed native bindings."""

import asyncio
import importlib.util
import unittest

if importlib.util.find_spec("flowsdk") is not None:
    import flowsdk
else:
    flowsdk = None


@unittest.skipIf(flowsdk is None, "Build bindings and set PYTHONPATH=python/package")
class NativeBindingsTests(unittest.TestCase):
    def test_public_exports_exist(self):
        for name in flowsdk.__all__:
            with self.subTest(name=name):
                self.assertTrue(hasattr(flowsdk, name))
        self.assertEqual(flowsdk.__version__, "0.6.1")

    def test_tcp_packet_and_acknowledgement_round_trip(self):
        engine = flowsdk.MqttEngineFfi("native-test", 5)
        engine.connect()
        self.assertEqual(engine.take_outgoing()[0], 0x10)
        engine.handle_incoming(b"\x20\x03\x00\x00\x00")
        self.assertTrue(engine.is_connected())
        connected = engine.take_events()[0]
        self.assertTrue(connected.is_connected())
        self.assertEqual(connected[0].reason_code, 0)
        for operation, kind, header in (
            (lambda: engine.subscribe("test/topic", 1), "subscribed", 0x90),
            (lambda: engine.unsubscribe("test/topic"), "unsubscribed", 0xb0),
        ):
            pid = operation()
            self.assertGreater(pid, 0)
            self.assertTrue(engine.take_outgoing())
            engine.handle_incoming(bytes([header, 4, pid >> 8, pid & 255, 0, 0]))
            events = engine.take_events()
            self.assertEqual(len(events), 1)
            self.assertTrue(getattr(events[0], "is_" + kind)())
            self.assertEqual(events[0][0].packet_id, pid)
        engine.disconnect()
        self.assertEqual(engine.take_outgoing()[0], 0xe0)
        self.assertTrue(engine.disconnect_complete())

    def test_async_wrapper_handles_native_acknowledgements(self):
        async def run():
            client = flowsdk.FlowMqttClient("native-async")
            client.engine.connect()
            client.engine.take_outgoing()
            client.engine.handle_incoming(b"\x20\x03\x00\x00\x00")
            protocol = flowsdk.FlowMqttProtocol(
                client.engine, flowsdk.TransportType.TCP,
                asyncio.get_running_loop(), client._on_event,
            )
            client.protocol = protocol
            task = asyncio.create_task(client.subscribe("test/topic", 1))
            await asyncio.sleep(0)
            pid = next(iter(client._pending_subscribe))
            protocol.data_received(bytes([0x90, 4, pid >> 8, pid & 255, 0, 0x87]))
            result = await task
            self.assertEqual(result.reason_codes, [0x87])
            self.assertTrue(result.is_failure)
            await client.disconnect()
        asyncio.run(run())

    def test_invalid_client_key_configuration_raises_native_error(self):
        with self.assertRaises(flowsdk.MqttErrorFfi.Configuration):
            flowsdk.FlowMqttClient(
                "native-invalid-tls", transport=flowsdk.TransportType.TLS,
                server_name="localhost", insecure_skip_verify=True,
                client_cert_file="/nonexistent/cert.pem",
            )

    def test_transport_engines_export_clock_and_disconnect_state(self):
        for transport in flowsdk.TransportType:
            with self.subTest(transport=transport):
                client = flowsdk.FlowMqttClient(
                    "native-clock", transport=transport, server_name="localhost",
                    insecure_skip_verify=True,
                )
                self.assertIsInstance(client.engine.elapsed_ms(), int)
                self.assertIsInstance(client.engine.disconnect_complete(), bool)

    def test_publish_properties_and_retain_are_forwarded(self):
        async def run():
            from types import SimpleNamespace
            client = flowsdk.FlowMqttClient("native-properties")
            client.engine.connect()
            client.engine.take_outgoing()
            client.engine.handle_incoming(b"\x20\x03\x00\x00\x00")
            client.engine.take_events()
            client.protocol = SimpleNamespace(closed=False, pump=lambda: None)
            self.assertEqual(await client.publish("test/topic", b"data", retain=True, priority=42,
                properties=flowsdk.PublishProperties(content_type="text/plain", correlation_data=b"\x00\xff",
                    user_properties=[("source", "one"), ("source", "two")])), 0)
            packet = bytes(client.engine.take_outgoing())
            self.assertEqual(packet[0], 0x31)
            self.assertIn(b"text/plain", packet)
            self.assertIn(b"\x09\x00\x02\x00\xff", packet)
            self.assertLess(packet.index(b"one"), packet.index(b"two"))
            self.assertEqual(packet.count(b"source"), 2)
        asyncio.run(run())

    def test_received_message_keeps_properties_retain_dup_and_packet_id(self):
        async def run():
            full, legacy, events = [], [], []
            client = flowsdk.FlowMqttClient("native-message", on_message_full=full.append,
                on_message=lambda *args: legacy.append(args), on_event=events.append)
            client.engine.connect()
            client.engine.take_outgoing()
            client.engine.handle_incoming(b"\x20\x03\x00\x00\x00")
            client.engine.take_events()
            properties = flowsdk.PublishProperties(user_properties=[("key", "one"), ("key", "two")]).to_ffi()
            pid = client.engine.publish_with_options("test/topic", b"data", flowsdk.MqttPublishOptionsFfi(
                qos=1, retain=True, priority=None, properties=properties))
            packet = bytearray(client.engine.take_outgoing())
            packet[0] |= 8  # Retransmitted PUBLISH.
            protocol = flowsdk.FlowMqttProtocol(client.engine, flowsdk.TransportType.TCP,
                asyncio.get_running_loop(), client._on_event)
            client.protocol = protocol
            protocol.data_received(bytes(packet))
            self.assertEqual(legacy, [("test/topic", b"data", 1)])
            self.assertEqual(len(full), 1)
            self.assertEqual(full[0].packet_id, pid)
            self.assertTrue(full[0].retain)
            self.assertTrue(full[0].dup)
            self.assertEqual(full[0].properties, properties)
            self.assertEqual(len(events), 2)
            self.assertTrue(events[0].is_publish_received())
            self.assertIsNone(full[0].stream_id)
            await client.disconnect()
        asyncio.run(run())

    def test_batch_subscription_options_and_mixed_ack(self):
        async def run():
            from types import SimpleNamespace
            client = flowsdk.FlowMqttClient("native-batch")
            client.engine.connect()
            client.engine.take_outgoing()
            client.engine.handle_incoming(b"\x20\x03\x00\x00\x00")
            client.engine.take_events()
            client.protocol = SimpleNamespace(closed=False, pump=lambda: None)
            task = asyncio.create_task(client.subscribe_many([
                flowsdk.Subscription("a/+", 1, True, True, 2), flowsdk.Subscription("b/#", 2),
            ], properties=[flowsdk.MqttPropertyFfi.SUBSCRIPTION_IDENTIFIER(value=3)]))
            await asyncio.sleep(0)
            packet = bytes(client.engine.take_outgoing())
            self.assertIn(b"\x00\x03a/+\x2d", packet)
            self.assertIn(b"\x00\x03b/#\x02", packet)
            self.assertIn(b"\x0b\x03", packet)
            pid = next(iter(client._pending_subscribe))
            client.engine.handle_incoming(bytes([0x90, 5, pid >> 8, pid & 255, 0, 1, 0x87]))
            for event in client.engine.take_events(): client._on_event(event)
            result = await task
            self.assertEqual(result.reason_codes, [1, 0x87])
            self.assertTrue(result.is_failure)
            task = asyncio.create_task(client.unsubscribe_many(["a/+", "b/#"],
                properties=[flowsdk.MqttPropertyFfi.USER_PROPERTY(key="key", value="value")]))
            await asyncio.sleep(0)
            packet = bytes(client.engine.take_outgoing())
            self.assertIn(b"key", packet)
            self.assertIn(b"a/+", packet)
            self.assertIn(b"b/#", packet)
            pid = next(iter(client._pending_unsubscribe))
            client.engine.handle_incoming(bytes([0xb0, 5, pid >> 8, pid & 255, 0, 0, 0x11]))
            for event in client.engine.take_events(): client._on_event(event)
            self.assertTrue((await task).is_success)
        asyncio.run(run())

    def test_connect_properties_will_and_binary_password(self):
        for transport in flowsdk.TransportType:
            with self.subTest(transport=transport):
                client = flowsdk.FlowMqttClient("native-connect", transport=transport,
                    server_name="localhost", insecure_skip_verify=True,
                    username="test", password=b"\x00\xff", clean_start=False,
                    connect_properties=flowsdk.ConnectProperties(session_expiry_interval=60,
                        receive_maximum=10, maximum_packet_size=1000),
                    will=flowsdk.Will("status", b"offline", qos=1, retain=True,
                        properties=[flowsdk.MqttPropertyFfi.WILL_DELAY_INTERVAL(value=30)]))
                if transport == flowsdk.TransportType.TCP:
                    client.engine.connect()
                    packet = bytes(client.engine.take_outgoing())
                    self.assertIn(b"\x00\x02\x00\xff", packet)
                    self.assertIn(b"offline", packet)
                    self.assertIn(b"\x11\x00\x00\x00\x3c", packet)
                    self.assertIn(b"\x18\x00\x00\x00\x1e", packet)
                with self.assertRaises(flowsdk.MqttErrorFfi.InvalidArgument):
                    flowsdk.FlowMqttClient("bad-version", transport=transport,
                        server_name="localhost", insecure_skip_verify=True, mqtt_version=0)
        with self.assertRaises(flowsdk.MqttErrorFfi.InvalidArgument):
            flowsdk.FlowMqttClient("v3", mqtt_version=3,
                connect_properties=flowsdk.ConnectProperties(session_expiry_interval=60))

    def test_enhanced_authentication_challenge_before_connack(self):
        async def run():
            from unittest.mock import Mock
            events = []
            client = flowsdk.FlowMqttClient("native-auth", on_event=events.append,
                connect_properties=flowsdk.ConnectProperties(authentication_method="test-method"))
            client.engine.connect()
            client.engine.take_outgoing()
            protocol = flowsdk.FlowMqttProtocol(client.engine, flowsdk.TransportType.TCP,
                asyncio.get_running_loop(), client._on_event)
            transport = Mock(spec=asyncio.Transport)
            transport.is_closing.return_value = False
            transport.get_write_buffer_size.return_value = 0
            transport.close.side_effect = lambda: asyncio.get_running_loop().call_soon(protocol.connection_lost, None)
            protocol.transport = transport
            client.protocol = protocol
            protocol.data_received(b"\xf0\x15\x18\x13\x15\x00\x0btest-method\x16\x00\x02\x00\xff")
            self.assertEqual(len(events), 1)
            self.assertTrue(events[0].is_auth_received())
            self.assertEqual(events[0][0].reason_code, 0x18)
            self.assertEqual(events[0][0].properties[1].value, b"\x00\xff")
            await client.auth(properties=[flowsdk.MqttPropertyFfi.AUTHENTICATION_DATA(value=b"response")])
            self.assertEqual(transport.write.call_count, 1)
            outgoing = bytes(transport.write.call_args[0][0])
            self.assertEqual(outgoing[0], 0xf0)
            self.assertIn(b"response", outgoing)
            self.assertIn(b"test-method", outgoing)
            for code in (0, 0x19):
                with self.assertRaises(flowsdk.MqttErrorFfi.Engine):
                    await client.auth(code)
            self.assertFalse(client.is_connected)
            with self.assertRaises(flowsdk.MqttErrorFfi.Engine):
                await client.auth(properties=[flowsdk.MqttPropertyFfi.AUTHENTICATION_METHOD(value="different")])
            # Successful CONNACK must repeat CONNECT's Authentication Method.
            protocol.data_received(b"\x20\x11\x00\x00\x0e\x15\x00\x0btest-method")
            self.assertEqual(len(events), 2)
            self.assertTrue(events[1].is_connected())
            self.assertEqual(events[1][0].reason_code, 0)
            self.assertEqual(events[1][0].properties[0].value, "test-method")
            self.assertTrue(client.is_connected)
            await client.disconnect()
        asyncio.run(run())

    def test_manual_receive_acknowledgements_complete_qos1_and_qos2(self):
        async def run(version):
            from types import SimpleNamespace
            client = flowsdk.FlowMqttClient("manual", mqtt_version=version,
                engine_options=flowsdk.EngineOptions(auto_ack=False))
            engine = client.engine
            engine.connect()
            engine.take_outgoing()
            engine.handle_incoming(b"\x20\x03\x00\x00\x00" if version == 5 else b"\x20\x02\x00\x00")
            engine.take_events()
            client.protocol = SimpleNamespace(closed=False, pump=lambda: None)
            for qos in (1, 2, 1):
                body = b"\x00\x01t\x00\x07" + (b"\x00" if version == 5 else b"") + b"data"
                engine.handle_incoming(bytes([0x30 | qos << 1, len(body)]) + body)
                events = engine.take_events()
                self.assertTrue(events[0].is_publish_received())
                self.assertEqual(events[1][0].packet_id, 7)
                self.assertEqual(bytes(engine.take_outgoing()), b"")
                await client.acknowledge(7, qos)
                outgoing = bytes(engine.take_outgoing())
                self.assertEqual(outgoing[0], 0x40 if qos == 1 else 0x50)
                self.assertEqual(outgoing[2:4], b"\x00\x07")
                if qos == 2:
                    with self.assertRaises(flowsdk.MqttErrorFfi.Engine):
                        await client.complete_qos2(7)
                    engine.handle_incoming(b"\x62\x02\x00\x07")
                    event = engine.take_events()[0]
                    self.assertTrue(event.is_pub_rel_received())
                    self.assertEqual(bytes(engine.take_outgoing()), b"")
                    await client.complete_qos2(event.packet_id, stream_id=event.stream_id)
                    self.assertEqual(bytes(engine.take_outgoing())[0], 0x70)
            with self.assertRaises(flowsdk.MqttErrorFfi.InvalidArgument):
                await client.acknowledge(0, 1)
            with self.assertRaises(flowsdk.MqttErrorFfi.InvalidArgument):
                await client.acknowledge(7, 1, stream_id=4)
        for version in (3, 4, 5): asyncio.run(run(version))

    def test_reduced_parser_reports_metadata_and_rejects_ack_waiting_operations(self):
        client = flowsdk.FlowMqttClient("parse-level")
        engine = client.engine
        with self.assertRaises(flowsdk.MqttErrorFfi.Engine):
            engine.set_parse_level(flowsdk.MqttParseLevelFfi.HEADERS_PARSED)
        engine.connect()
        engine.take_outgoing()
        engine.handle_incoming(b"\x20\x03\x00\x00\x00")
        engine.take_events()
        for level in (flowsdk.MqttParseLevelFfi.HEADERS_PARSED, flowsdk.MqttParseLevelFfi.TYPE_ONLY):
            engine.set_parse_level(level)
            engine.handle_incoming(b"\x30\x05\x00\x01t\x00x")
            events = engine.take_events()
            self.assertEqual(len(events), 1)
            self.assertTrue(events[0].is_publish_received())
            self.assertEqual(engine.subscribe("test", 0), -1)
            self.assertEqual(engine.publish("test", b"payload", 1, None), -1)
        engine.set_parse_level(flowsdk.MqttParseLevelFfi.FULL)
        self.assertGreater(engine.subscribe("test", 0), 0)

    def test_engine_limits_apply_and_buffered_packets_resume_on_tick(self):
        client = flowsdk.FlowMqttClient("tuning", keep_alive=1,
            engine_options=flowsdk.EngineOptions(max_outgoing_packet_count=1,
                max_inflight=1, max_event_count=1, parser_buffer_size=1, auto_keepalive=False))
        engine = client.engine
        engine.connect()
        engine.take_outgoing()
        engine.handle_incoming(b"\x20\x03\x00\x00\x00")
        engine.take_events()
        first = engine.publish("one", b"1", 1, None)
        engine.take_outgoing()
        second = engine.publish("two", b"2", 1, None)
        self.assertEqual(bytes(engine.take_outgoing()), b"")
        self.assertEqual(engine.publish("three", b"3", 1, None), -1)
        engine.handle_incoming(bytes([0x40, 2, first >> 8, first & 255]))
        self.assertIn(b"two", bytes(engine.take_outgoing()))
        engine.take_events()
        engine.handle_incoming(b"\xd0\x00\xd0\x00")
        self.assertEqual(len(engine.take_events()), 1)
        engine.handle_tick(engine.elapsed_ms() + 2000)
        self.assertEqual(len(engine.take_events()), 1)
        self.assertEqual(bytes(engine.take_outgoing()), b"")
        self.assertNotEqual(first, second)
        for field in ("max_inflight", "max_event_count", "max_outgoing_packet_count",
                      "parser_buffer_size", "retransmission_timeout_ms", "ping_timeout_multiplier"):
            with self.assertRaises(flowsdk.MqttErrorFfi.InvalidArgument):
                flowsdk.FlowMqttClient("invalid", engine_options=flowsdk.EngineOptions(**{field: 0}))

    def test_configured_subscriptions_only_run_for_a_new_session(self):
        client = flowsdk.FlowMqttClient("configured", clean_start=False,
            connect_properties=flowsdk.ConnectProperties(session_expiry_interval=60),
            engine_options=flowsdk.EngineOptions(subscriptions=[flowsdk.Subscription("configured/topic", 1)]))
        for session_present in (False, True, False):
            client.engine.connect()
            client.engine.take_outgoing()
            client.engine.handle_incoming(bytes([0x20, 3, int(session_present), 0, 0]))
            outgoing = bytes(client.engine.take_outgoing())
            self.assertEqual(b"configured/topic" in outgoing, not session_present)
            # Complete automatic SUBSCRIBE before testing session resumption.
            if outgoing:
                packet_id = outgoing[2:4]
                client.engine.handle_incoming(b"\x90\x04" + packet_id + b"\x00\x01")
            client.engine.take_events()
        with self.assertRaises(flowsdk.MqttErrorFfi.InvalidArgument):
            flowsdk.FlowMqttClient("conflicting", clean_start=False,
                engine_options=flowsdk.EngineOptions(sessionless=True))

    def test_quic_controls_and_zero_rtt_use_generated_contract(self):
        client = flowsdk.FlowMqttClient("advanced-quic", transport=flowsdk.TransportType.QUIC,
            insecure_skip_verify=True, quic_zero_rtt=flowsdk.QuicZeroRttOptions(session_cache_size=8))
        engine = client.engine
        self.assertEqual(client.quic.zero_rtt_status, flowsdk.QuicZeroRttStatusFfi.DISABLED)
        self.assertIsNone(client.quic.control_stream_id)
        self.assertEqual(client.quic.data_stream_count, 0)
        for action in (lambda: engine.open_data_stream(), lambda: engine.finish_stream(12),
                       lambda: engine.reset_stream(12, 1), lambda: engine.stop_stream(12, 1),
                       lambda: engine.reconnect(engine.elapsed_ms()), lambda: engine.quic_ping()):
            with self.assertRaises(flowsdk.MqttErrorFfi.Engine): action()
        engine.connect_with_zero_rtt("127.0.0.1:14567", "localhost", client.tls_opts,
            client._zero_rtt_options, engine.elapsed_ms())
        self.assertEqual(client.quic.zero_rtt_status, flowsdk.QuicZeroRttStatusFfi.UNAVAILABLE)
        self.assertTrue(any(event.is_zero_rtt_status_changed() for event in engine.take_events()))
        client.quic.clear_session_cache()
        engine.close_silent()
        with self.assertRaises(ValueError): flowsdk.FlowMqttClient("tcp").quic

    def test_quic_rejects_malformed_datagram_address(self):
        client = flowsdk.FlowMqttClient("quic-address", transport=flowsdk.TransportType.QUIC)
        with self.assertRaises(flowsdk.MqttErrorFfi.InvalidArgument):
            client.engine.handle_datagram(b"invalid", "invalid address", client.engine.elapsed_ms())
        client.engine.handle_datagram(b"ignored", "[::1]:14567", client.engine.elapsed_ms())

    def test_tls_reuse_starts_a_fresh_handshake(self):
        client = flowsdk.FlowMqttClient("tls-reuse", transport=flowsdk.TransportType.TLS,
            server_name="localhost", insecure_skip_verify=True)
        client.engine.connect()
        first = bytes(client.engine.take_socket_data())
        client.engine.handle_connection_lost()
        client.engine.connect()
        second = bytes(client.engine.take_socket_data())
        self.assertEqual(first[0], 22)  # TLS handshake record
        self.assertEqual(second[0], 22)
        self.assertNotEqual(first, second)  # Fresh ClientHello random

    def test_auth_rejects_missing_or_changed_server_method(self):
        for packet in (b"\xf0\x02\x18\x00", b"\xf0\x09\x18\x07\x15\x00\x04else"):
            client = flowsdk.FlowMqttClient("auth-validation",
                connect_properties=flowsdk.ConnectProperties(authentication_method="method"))
            client.engine.connect()
            client.engine.handle_incoming(packet)
            self.assertTrue(any(event.is_error() for event in client.engine.take_events()))

    def test_explicit_ping_waits_for_native_pingresp(self):
        async def run():
            from types import SimpleNamespace
            client = flowsdk.FlowMqttClient("native-ping")
            client.engine.connect()
            client.engine.take_outgoing()
            client.engine.handle_incoming(b"\x20\x03\x00\x00\x00")
            client.engine.take_events()
            client.protocol = SimpleNamespace(closed=False, pump=lambda: None)
            task = asyncio.create_task(client.ping())
            await asyncio.sleep(0)
            self.assertEqual(bytes(client.engine.take_outgoing()), b"\xc0\x00")
            self.assertFalse(task.done())
            client.engine.handle_incoming(b"\xd0\x00")
            for event in client.engine.take_events(): client._on_event(event)
            self.assertTrue(await task)
            self.assertEqual(client._pending_ping, {})
        asyncio.run(run())

    def test_local_disconnect_preserves_properties_and_closes_input(self):
        engine = flowsdk.MqttEngineFfi("native-disconnect", 5)
        engine.connect()
        engine.take_outgoing()
        engine.handle_incoming(b"\x20\x03\x00\x00\x00")
        engine.take_events()
        with self.assertRaises(flowsdk.MqttErrorFfi.Engine):
            engine.disconnect_with_options(flowsdk.MqttDisconnectOptionsFfi(reason_code=0x89, properties=[]))
        self.assertTrue(engine.is_connected())
        with self.assertRaises(flowsdk.MqttErrorFfi.Engine):
            engine.disconnect_with_options(flowsdk.MqttDisconnectOptionsFfi(reason_code=0,
                properties=[flowsdk.MqttPropertyFfi.SESSION_EXPIRY_INTERVAL(value=10)]))
        properties = [flowsdk.MqttPropertyFfi.REASON_STRING(value="bye")]
        engine.disconnect_with_options(flowsdk.MqttDisconnectOptionsFfi(reason_code=4, properties=properties))
        packet = bytes(engine.take_outgoing())
        self.assertEqual(packet, b"\xe0\x08\x04\x06\x1f\x00\x03bye")
        self.assertFalse(engine.is_connected())
        self.assertEqual(engine.handle_incoming(b"\xe0\x08\x80\x06\x1f\x00\x03bye"), [])
        self.assertEqual(engine.take_events(), [])

    def test_peer_disconnect_preserves_properties(self):
        engine = flowsdk.MqttEngineFfi("native-peer-disconnect", 5)
        engine.connect()
        engine.take_outgoing()
        engine.handle_incoming(b"\x20\x03\x00\x00\x00")
        engine.take_events()
        self.assertTrue(engine.is_connected())
        engine.handle_incoming(b"\xe0\x08\x80\x06\x1f\x00\x03bye")
        events = engine.take_events()
        self.assertEqual(len(events), 1)
        event = events[0]
        self.assertTrue(event.is_disconnected())
        self.assertEqual(event.reason_code, 0x80)
        self.assertEqual(event.properties, [flowsdk.MqttPropertyFfi.REASON_STRING(value="bye")])
        self.assertFalse(engine.is_connected())


if __name__ == "__main__":
    unittest.main()
