"""Transport failures must close once and stop driving the MQTT engine."""

import socket
from unittest.mock import Mock, patch

import test_async_client as harness


class ProtocolFailureTests(harness.ClientTestCase):
    def protocol(self, transport_type):
        loop, lost, events, transport = Mock(), Mock(), Mock(), Mock()
        transport.is_closing.return_value = False
        if transport_type == harness.async_client.TransportType.QUIC:
            protocol = harness.async_client.FlowMqttDatagramProtocol(
                self.engine, loop, events, lost)
        else:
            protocol = harness.async_client.FlowMqttProtocol(
                self.engine, transport_type, loop, events, lost)
        protocol.connection_made(transport)
        return protocol, transport, lost, events

    def test_incoming_errors_close_transport_and_cancel_timer(self):
        for kind in harness.async_client.TransportType:
            with self.subTest(transport=kind):
                protocol, transport, lost, events = self.protocol(kind)
                timer = protocol._tick_handle
                error = OSError("invalid incoming transport data")
                method = {"tcp": "handle_incoming", "tls": "handle_socket_data",
                          "quic": "handle_datagram"}[kind.value]
                with patch.object(self.engine, method, side_effect=error) as receive:
                    if kind == harness.async_client.TransportType.QUIC:
                        protocol.datagram_received(b"bad", ("127.0.0.1", 1883))
                        protocol.datagram_received(b"late", ("127.0.0.1", 1883))
                    else:
                        protocol.data_received(b"bad")
                        protocol.data_received(b"late")
                    receive.assert_called_once()
                self.assertTrue(protocol.closed)
                timer.cancel.assert_called_once()
                transport.close.assert_called_once()
                lost.assert_called_once_with(error)
                events.assert_not_called()
                protocol.connection_lost(None)
                protocol._on_timer()
                protocol._schedule_tick(0)
                protocol.pump()
                lost.assert_called_once()
                transport.close.assert_called_once()

    def test_output_failure_closes_transport_and_propagates_error(self):
        for kind in harness.async_client.TransportType:
            with self.subTest(transport=kind):
                protocol, transport, lost, events = self.protocol(kind)
                error = OSError("socket write failed")
                if kind == harness.async_client.TransportType.QUIC:
                    self.engine.take_outgoing_datagrams.return_value = [
                        harness.SimpleNamespace(data=b"packet", addr="127.0.0.1:1883")]
                    transport.sendto.side_effect = error
                else:
                    self.engine.take_outgoing.return_value = b"packet"
                    self.engine.take_socket_data.return_value = b"packet"
                    transport.write.side_effect = error
                with self.assertRaisesRegex(OSError, "socket write failed"):
                    protocol.pump()
                lost.assert_called_once_with(error)
                transport.close.assert_called_once()
                events.assert_not_called()

    def test_connect_error_and_late_socket_close_exactly_once(self):
        for kind in (harness.async_client.TransportType.TCP,
                     harness.async_client.TransportType.TLS):
            with self.subTest(transport=kind):
                error = OSError("engine cannot connect")
                with patch.object(self.engine, "connect", side_effect=error):
                    protocol, transport, lost, _ = self.protocol(kind)
                self.assertIsNone(protocol._tick_handle)
                lost.assert_called_once_with(error)
                transport.close.assert_called_once()
                late = Mock()
                protocol.connection_made(late)
                late.close.assert_called_once()
                lost.assert_called_once()

    def test_tick_and_deadline_errors_close_without_rescheduling(self):
        cases = [(kind, "handle_tick") for kind in harness.async_client.TransportType]
        cases.append((harness.async_client.TransportType.TCP, "next_tick_ms"))
        for kind, method in cases:
            with self.subTest(transport=kind, method=method):
                protocol, transport, lost, _ = self.protocol(kind)
                protocol.loop.reset_mock()
                error = RuntimeError("timer failure")
                with patch.object(self.engine, method, side_effect=error):
                    protocol._on_timer()
                lost.assert_called_once_with(error)
                transport.close.assert_called_once()
                protocol.loop.call_later.assert_not_called()

    def test_event_closure_stops_delivery_of_remaining_events(self):
        for kind in harness.async_client.TransportType:
            with self.subTest(transport=kind):
                protocol, transport, lost, events = self.protocol(kind)
                self.engine.take_events.return_value = ["close", "late"]
                events.side_effect = lambda event: protocol.close()
                protocol._on_timer()
                events.assert_called_once_with("close")
                transport.close.assert_called_once()
                lost.assert_called_once_with(None)

    def test_quic_peername_and_udp_errors_do_not_close_connection(self):
        protocol, transport, lost, _ = self.protocol(harness.async_client.TransportType.QUIC)
        transport.get_extra_info.side_effect = OSError("peer unknown")
        with self.assertLogs(harness.async_client.logger, level="DEBUG"):
            protocol.connection_made(transport)
        protocol.error_received(OSError("ICMP error"))
        self.assertFalse(protocol.closed)
        lost.assert_not_called()
        protocol.close()
        late = Mock()
        protocol.connection_made(late)
        late.close.assert_called_once()

    def test_named_ipv6_zone_is_resolved_and_preserved(self):
        with patch.object(socket, "if_nametoindex", return_value=7) as lookup:
            address = harness.async_client._format_socket_address(("fe80::1%eth0", 14567, 0, 0))
        lookup.assert_called_once_with("eth0")
        self.assertEqual(address, "[fe80::1%7]:14567")
        self.assertEqual(harness.async_client._parse_socket_address(address),
                         ("fe80::1", 14567, 0, 7))
