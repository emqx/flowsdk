"""
async MQTT client implementation using flowsdk-ffi with asyncio.

This module provides an asyncio-based MQTT client that wraps the flowsdk_ffi
library, providing async/await support for MQTT operations. It implements a
custom Protocol for handling the network layer and manages the timing and
pumping of the MQTT engine.

Supports multiple transports:
    - TCP: Standard MQTT over TCP (default)
    - QUIC: MQTT over QUIC protocol (UDP-based, built-in encryption)
    - TLS: MQTT over TLS (encrypted TCP)

Classes:
    TransportType: Enum for selecting MQTT transport protocol
    FlowMqttProtocol: asyncio.Protocol implementation for TCP transport
    FlowMqttDatagramProtocol: asyncio.DatagramProtocol for QUIC transport
    FlowMqttClient: High-level async MQTT client supporting multiple transports

Example (TCP):
    >>> import asyncio
    >>> from flowsdk import FlowMqttClient
    >>> 
    >>> async def example():
    ...     client = FlowMqttClient("my_client_id")
    ...     await client.connect("broker.emqx.io", 1883)
    ...     subscription = await client.subscribe("test/topic", 1)
    ...     subscription.raise_for_status()
    ...     await client.publish("test/topic", b"Hello", 1)
    ...     await client.disconnect()
    >>> 
    >>> asyncio.run(example())

Example (QUIC):
    >>> import asyncio
    >>> from flowsdk import FlowMqttClient, TransportType
    >>> 
    >>> async def example():
    ...     client = FlowMqttClient("my_client_id", transport=TransportType.QUIC)
    ...     await client.connect("broker.emqx.io", 14567, server_name="broker.emqx.io")
    ...     subscription = await client.subscribe("test/topic", 1)
    ...     subscription.raise_for_status()
    ...     await client.publish("test/topic", b"Hello", 1)
    ...     await client.disconnect()
    >>> 
    >>> asyncio.run(example())
"""

import asyncio
import logging
import socket
from enum import Enum
from typing import Optional, Dict, Callable, Any, List, Union

from . import flowsdk_ffi
from .results import ConnectionResult, PublishResult, SubscribeResult, UnsubscribeResult
from .properties import property_list
from .options import Subscription, Will, EngineOptions, QuicZeroRttOptions, RuntimeOptions
from .quic import QuicControls


logger = logging.getLogger(__name__)
logger.addHandler(logging.NullHandler())


def _format_socket_address(addr):
    host, port = addr[:2]
    if ":" in host:
        host, _, zone = host.partition("%")
        scope = addr[3] if len(addr) > 3 else 0
        if zone:
            scope = int(zone) if zone.isdigit() else socket.if_nametoindex(zone)
        return "[{}{}]:{}".format(host, "%{}".format(scope) if scope else "", port)
    return "{}:{}".format(host, port)


def _parse_socket_address(value):
    if value.startswith("["):
        host, port = value[1:].split("]:", 1)
        host, _, scope = host.partition("%")
        return (host, int(port), 0, int(scope) if scope else 0)
    host, port = value.rsplit(":", 1)
    return (host, int(port))


class OperationFailedError(RuntimeError):
    """A single MQTT operation failed; its exchange may still await a late ACK."""
    def __init__(self, event):
        super().__init__(event.detail)
        self.operation = event.operation
        self.packet_id = event.packet_id
        self.kind = event.kind
        self.timeout_ms = event.timeout_ms


class TransportType(Enum):
    """MQTT transport protocol types."""
    TCP = "tcp"
    TLS = "tls"
    QUIC = "quic"


class _ProtocolLifecycle:
    def close(self, exc: Optional[Exception] = None, *, abort=False):
        """Stop driving this transport and notify its owner exactly once."""
        if self.closed:
            return
        self.closed = True
        if self._tick_handle:
            self._tick_handle.cancel()
            self._tick_handle = None
        try:
            self._notify_engine_connection_lost()
        finally:
            try:
                if self.transport:
                    if abort and hasattr(self.transport, "abort"):
                        self.transport.abort()
                    else:
                        self.transport.close()
            finally:
                if self.on_connection_lost_cb:
                    self.on_connection_lost_cb(exc)

    def _notify_engine_connection_lost(self):
        pass

    def connection_lost(self, exc: Optional[Exception]):
        if not self._transport_closed.done():
            self._transport_closed.set_result(None)
        self.close(exc)

    async def wait_closed(self):
        if self.transport is not None:
            await asyncio.shield(self._transport_closed)


class FlowMqttProtocol(_ProtocolLifecycle, asyncio.Protocol):
    """
    asyncio Protocol implementation for MQTT engine (TCP/TLS).
    
    This class handles the network layer, manages engine ticks, and pumps
    data between the network transport and the MQTT engine.
    
    Args:
        engine: The MQTT engine instance
        transport_type: The transport protocol being used
        loop: The asyncio event loop
        on_event_cb: Callback function to handle MQTT events
        on_connection_lost_cb: Optional callback receiving the transport error
    """
    
    def __init__(
        self, engine, transport_type: TransportType, loop: asyncio.AbstractEventLoop,
        on_event_cb: Callable, on_connection_lost_cb: Optional[Callable] = None,
    ):
        self.engine = engine
        self.transport_type = transport_type
        self.loop = loop
        self.transport: Optional[asyncio.Transport] = None
        self.on_event_cb = on_event_cb
        self.on_connection_lost_cb = on_connection_lost_cb
        self._tick_handle: Optional[asyncio.TimerHandle] = None
        self.closed = False
        self._transport_closed = loop.create_future()
        self._write_paused = False

    def pause_writing(self):
        self._write_paused = True

    def resume_writing(self):
        self._write_paused = False
        self.pump()

    def connection_made(self, transport: asyncio.Transport):
        """Called when connection is established."""
        self.transport = transport
        if self.closed:
            transport.close()
            return
        
        # Start the MQTT connection
        if hasattr(self.engine, 'connect'):
            try:
                self.engine.reset_for_new_transport()
                self.engine.connect_checked()
            except Exception as exc:
                self.close(exc)
                return
        
        # Start the tick loop immediately
        self._schedule_tick(0)

    def data_received(self, data: bytes):
        """Called when data is received from the network."""
        if self.closed:
            return
        # Feed network data to engine
        try:
            if self.transport_type == TransportType.TLS:
                self.engine.handle_socket_data(data)
            else:
                self.engine.handle_incoming(data)
        except Exception as exc:
            self.close(exc)
            return
            
        # Trigger an immediate pump to process potential responses
        self.pump()

    def _notify_engine_connection_lost(self):
        if hasattr(self.engine, 'handle_connection_lost'):
            self.engine.handle_connection_lost()
        
    def _schedule_tick(self, delay_sec: float):
        """Schedule the next engine tick."""
        if self.closed:
            return
        
        if self._tick_handle:
            self._tick_handle.cancel()
            
        self._tick_handle = self.loop.call_later(delay_sec, self._on_timer)

    def _on_timer(self):
        """Handle scheduled engine tick."""
        if self.closed:
            return
        
        # Run protocol tick
        now_ms = self.engine.elapsed_ms()
        try:
            self.engine.handle_tick(now_ms)
        except Exception as exc:
            self.close(exc)
            return
        
        self.pump()
        if self.closed:
            return
        
        # Schedule next tick
        if self.transport_type == TransportType.TCP:
            try:
                next_tick_ms = self.engine.next_tick_ms()
            except Exception as exc:
                self.close(exc)
                return
            if next_tick_ms < 0:
                delay = 0.1
            else:
                delay = max(0, (next_tick_ms - now_ms) / 1000.0)
        else:
            # Fixed 10ms for TLS/QUIC
            delay = 0.01
            
        self._schedule_tick(delay)

    def pump(self):
        """Pump data between engine and network."""
        if self.closed:
            return
        # 1. Send outgoing data
        try:
            if self._write_paused:
                outgoing = b""
            elif self.transport_type == TransportType.TLS:
                outgoing = self.engine.take_socket_data()
            else:
                outgoing = self.engine.take_outgoing()

            if outgoing and self.transport and not self.transport.is_closing():
                self.transport.write(outgoing)
            events = self.engine.take_events()
        except Exception as exc:
            self.close(exc)
            raise
        
        # 2. Process events
        for ev in events:
            if self.closed:
                break
            self.on_event_cb(ev)


class FlowMqttDatagramProtocol(_ProtocolLifecycle, asyncio.DatagramProtocol):
    """
    asyncio DatagramProtocol implementation for QUIC MQTT engine.
    
    This class handles UDP datagram transport for QUIC, managing engine ticks
    and pumping data between the network transport and the MQTT engine.
    
    Args:
        engine: The QUIC MQTT engine instance from flowsdk_ffi
        loop: The asyncio event loop
        on_event_cb: Callback function to handle MQTT events
        on_connection_lost_cb: Optional callback receiving the transport error
    """
    
    def __init__(
        self, engine, loop: asyncio.AbstractEventLoop, on_event_cb: Callable,
        on_connection_lost_cb: Optional[Callable] = None,
    ):
        self.engine = engine
        self.loop = loop
        self.transport: Optional[asyncio.DatagramTransport] = None
        self.on_event_cb = on_event_cb
        self.on_connection_lost_cb = on_connection_lost_cb
        self._tick_handle: Optional[asyncio.TimerHandle] = None
        self.closed = False
        self._transport_closed = loop.create_future()
        self.remote_addr = None

    def connection_made(self, transport: asyncio.DatagramTransport):
        """Called when UDP socket is ready."""
        self.transport = transport
        if self.closed:
            transport.close()
            return
        try:
            self.remote_addr = transport.get_extra_info('peername')
        except Exception:
            logger.debug("Could not get peername from transport", exc_info=True)
        
        # Don't call engine.connect() here - QUIC requires parameters
        # Connection will be initiated from FlowMqttClient.connect()
        
        # Start the tick loop immediately
        self._schedule_tick(0)

    def datagram_received(self, data: bytes, addr):
        """Called when a datagram is received."""
        if self.closed:
            return
        # Feed datagram to QUIC engine
        now_ms = self.engine.elapsed_ms()
        addr_str = _format_socket_address(addr)
        try:
            self.engine.handle_datagram(data, addr_str, now_ms)
        except Exception as exc:
            self.close(exc)
            return
        # Trigger an immediate pump to process responses
        self.pump()

    def _notify_engine_connection_lost(self):
        self.engine.close_silent()

    def error_received(self, exc: Exception):
        """Called when an error is received."""
        pass  # UDP errors are generally non-fatal

    def _schedule_tick(self, delay_sec: float):
        """Schedule the next engine tick."""
        if self.closed:
            return
        
        if self._tick_handle:
            self._tick_handle.cancel()
            
        self._tick_handle = self.loop.call_later(delay_sec, self._on_timer)

    def _on_timer(self):
        """Handle scheduled engine tick."""
        if self.closed:
            return
        
        # Run protocol tick
        now_ms = self.engine.elapsed_ms()
        # The FFI also queues tick events; pump() dispatches them via take_events().
        try:
            self.engine.handle_tick(now_ms)
        except Exception as exc:
            self.close(exc)
            return
        
        self.pump()
        
        # Fixed 10ms interval for QUIC
        delay = 0.01
        self._schedule_tick(delay)

    def pump(self):
        """Pump data between engine and network."""
        if self.closed:
            return
        # 1. Send outgoing datagrams
        try:
            datagrams = self.engine.take_outgoing_datagrams()
            if datagrams and self.transport:
                for datagram in datagrams:
                    self.transport.sendto(datagram.data, _parse_socket_address(datagram.addr))
            events = self.engine.take_events()
        except Exception as exc:
            self.close(exc)
            raise
        
        # 2. Process events
        for ev in events:
            if self.closed:
                break
            self.on_event_cb(ev)



class FlowMqttClient:
    """
    High-level async MQTT client supporting multiple transports (TCP, QUIC).
    
    This client provides a simple async/await API for MQTT operations,
    handling connection management, subscriptions, and publishing.
    
    Args:
        client_id: MQTT client identifier
        transport: Transport type (TransportType.TCP or TransportType.QUIC, default: TCP)
        mqtt_version: MQTT protocol version (3 or 5, default: 5)
        clean_start: Whether to start a clean session (default: True)
        keep_alive: Keep-alive interval in seconds (default: 30)
        username: Optional username for authentication
        password: Optional password for authentication
        reconnect_base_delay_ms: Base delay for reconnection attempts (default: 1000)
        reconnect_max_delay_ms: Maximum delay for reconnection (default: 30000)
        max_reconnect_attempts: Maximum reconnection attempts, 0 for infinite (default: 0)
        on_message: Optional callback for received messages: fn(topic: str, payload: bytes, qos: int)
        ca_cert_file: Path to CA certificate file for QUIC TLS (QUIC only)
        insecure_skip_verify: Skip TLS verification for QUIC (QUIC only, default: False)
        alpn_protocols: ALPN protocols for QUIC (QUIC only, default: ["mqtt"])
    
    Examples:
        TCP:
        >>> async def tcp_example():
        ...     client = FlowMqttClient("my_client", transport=TransportType.TCP)
        ...     await client.connect("broker.emqx.io", 1883)
        ...     await client.publish("test/topic", b"Hello", 1)
        ...     await client.disconnect()
        
        QUIC:
        >>> async def quic_example():
        ...     client = FlowMqttClient("my_client", transport=TransportType.QUIC, insecure_skip_verify=True)
        ...     await client.connect("broker.emqx.io", 14567, server_name="broker.emqx.io")
        ...     await client.publish("test/topic", b"Hello", 1)
        ...     await client.disconnect()
    """
    
    def __init__(
        self,
        client_id: str,
        transport: TransportType = TransportType.TCP,
        mqtt_version: int = 5,
        clean_start: bool = True,
        keep_alive: int = 30,
        username: Optional[str] = None,
        password: Optional[Union[str, bytes]] = None,
        reconnect_base_delay_ms: int = 1000,
        reconnect_max_delay_ms: int = 30000,
        max_reconnect_attempts: int = 0,
        on_message: Optional[Callable[[str, bytes, int], None]] = None,
        ca_cert_file: Optional[str] = None,
        client_cert_file: Optional[str] = None,
        client_key_file: Optional[str] = None,
        insecure_skip_verify: bool = False,
        alpn_protocols: Optional[List[str]] = None,
        server_name: Optional[str] = None,
        enable_key_log: bool = False,
        *, on_message_full: Optional[Callable] = None, on_event: Optional[Callable] = None,
        connect_properties=None, will: Optional[Will] = None, engine_options: Optional[EngineOptions] = None, auto_reconnect: bool = False,
        quic_zero_rtt: Optional[QuicZeroRttOptions] = None,
        runtime_options: Optional[RuntimeOptions] = None,
        session_state: Optional[bytes] = None,
    ):
        if password is not None and not isinstance(password, (str, bytes)):
            raise TypeError("password must be str, bytes, or None")
        self.transport_type = transport
        if transport == TransportType.QUIC and not hasattr(flowsdk_ffi, "QuicMqttEngineFfi"):
            raise flowsdk_ffi.MqttErrorFfi.Unsupported(detail="QUIC support is not enabled")
        if quic_zero_rtt is not None and transport != TransportType.QUIC:
            raise ValueError("0-RTT options require QUIC")
        self._zero_rtt_options = quic_zero_rtt.to_ffi() if isinstance(quic_zero_rtt, QuicZeroRttOptions) else quic_zero_rtt
        self.server_name = server_name
        self.opts = flowsdk_ffi.MqttOptionsFfi(
            client_id=client_id,
            mqtt_version=mqtt_version,
            clean_start=clean_start,
            keep_alive=keep_alive,
            username=username,
            password=password if isinstance(password, str) else None,
            reconnect_base_delay_ms=reconnect_base_delay_ms,
            reconnect_max_delay_ms=reconnect_max_delay_ms,
            max_reconnect_attempts=max_reconnect_attempts
        )

        self._retired = False
        self._runtime_options = (runtime_options.to_ffi() if isinstance(runtime_options, RuntimeOptions)
                                 else runtime_options)
        self._session_peer = getattr(self._runtime_options, "peer", None)
        if session_state is not None:
            self._require_durable_session()
        if session_state is not None and not self._session_peer:
            raise ValueError("Restoration requires runtime_options.peer")
        if self._runtime_options is not None and self._runtime_options.reconnect:
            raise ValueError("FlowMqttClient owns reconnect scheduling; use auto_reconnect")
        if session_state is not None and not self.opts.client_id:
            self.opts.client_id = flowsdk_ffi.inspect_session_state(session_state).client_id
        extended_options = None
        if connect_properties is not None or will is not None or isinstance(password, bytes) or engine_options is not None or self._runtime_options is not None:
            extended_options = flowsdk_ffi.MqttConnectOptionsFfi(
                options=self.opts, properties=property_list(connect_properties),
                will=will.to_ffi() if isinstance(will, Will) else will,
                binary_password=password if isinstance(password, bytes) else None,
                engine_options=engine_options.to_ffi() if isinstance(engine_options, EngineOptions) else engine_options,
            )
        
        # Common TLS/QUIC options
        self.tls_opts = flowsdk_ffi.MqttTlsOptionsFfi(
            ca_cert_file=ca_cert_file,
            client_cert_file=client_cert_file,
            client_key_file=client_key_file,
            insecure_skip_verify=insecure_skip_verify,
            alpn_protocols=alpn_protocols or ["mqtt"],
            enable_key_log=enable_key_log
        )
        
        # Create appropriate engine based on transport type
        if self._runtime_options is not None:
            if transport == TransportType.TCP:
                self.engine = flowsdk_ffi.MqttEngineFfi.new_with_runtime_options(extended_options, self._runtime_options)
            elif transport == TransportType.TLS:
                if not server_name:
                    raise ValueError("server_name (SNI) is required for TLS transport")
                self.engine = flowsdk_ffi.TlsMqttEngineFfi.new_with_runtime_options(
                    extended_options, self._runtime_options, self.tls_opts, server_name)
            elif transport == TransportType.QUIC:
                self.engine = flowsdk_ffi.QuicMqttEngineFfi.new_with_runtime_options(extended_options, self._runtime_options)
            else:
                raise ValueError(f"Unsupported transport type: {transport}")
        elif transport == TransportType.TCP:
            self.engine = (flowsdk_ffi.MqttEngineFfi.new_with_options(extended_options)
                           if extended_options is not None else flowsdk_ffi.MqttEngineFfi.new_with_opts(self.opts))
        elif transport == TransportType.TLS:
            if not server_name:
                raise ValueError("server_name (SNI) is required for TLS transport")
            self.engine = (flowsdk_ffi.TlsMqttEngineFfi.new_with_options(extended_options, self.tls_opts, server_name)
                           if extended_options is not None else flowsdk_ffi.TlsMqttEngineFfi(self.opts, self.tls_opts, server_name))
        elif transport == TransportType.QUIC:
            self.engine = (flowsdk_ffi.QuicMqttEngineFfi.new_with_options(extended_options)
                           if extended_options is not None else flowsdk_ffi.QuicMqttEngineFfi(self.opts))
        else:
            raise ValueError(f"Unsupported transport type: {transport}")
        
        # Exactly one retry scheduler owns this driver.
        self.engine.set_reconnect(False)
        if session_state is not None:
            self.engine.restore_session_state(session_state)
        self.protocol: Optional[Union[FlowMqttProtocol, FlowMqttDatagramProtocol]] = None
        self.on_message = on_message
        self.on_message_full = on_message_full
        self.on_event = on_event
        self.connection_result: Optional[ConnectionResult] = None
        self._disconnecting = False
        self._auto_reconnect = auto_reconnect
        self.reconnect_error: Optional[Exception] = None
        self._reconnect_target = None
        self._reconnect_task: Optional[asyncio.Task] = None
        self._connect_task: Optional[asyncio.Task] = None
        
        # Futures for pending operations
        self._connect_future: Optional[asyncio.Future] = None
        self._pending_publish: Dict[int, asyncio.Future] = {}
        self._pending_subscribe: Dict[int, asyncio.Future] = {}
        self._pending_unsubscribe: Dict[int, asyncio.Future] = {}
        self._pending_ping: Dict[int, asyncio.Future] = {}

    async def connect(self, host: str, port: int, server_name: Optional[str] = None, timeout: float = 10.0, *, return_result: bool = False):
        """
        Connect to MQTT broker.
        
        Args:
            host: Broker hostname or IP address
            port: Broker port number
            server_name: Server name for TLS SNI (required for QUIC)
            timeout: Connection timeout in seconds (default: 10.0)
            
        Raises:
            ConnectionError: If connection fails or is already active
            asyncio.TimeoutError: If connection times out
        """
        if self._retired:
            raise ConnectionError("This client was retired for restart; create a new client")
        if self._disconnecting:
            raise ConnectionError("Client shutdown is in progress")
        if self._session_peer is not None:
            # Keep the configured hostname/IPv6 zone, not a resolved address or
            # machine-local interface index, in the durable identity.
            name, separator, zone = host.partition("%")
            canonical_host = name.lower() + separator + zone
            authority = f"[{canonical_host}]" if ":" in name else canonical_host
            target_peer = f"{self.transport_type.value}://{authority}:{port}"
            if self._session_peer != target_peer:
                raise ValueError("Connection target does not match runtime_options.peer")
        if self._reconnect_task is not None and self._reconnect_task is not asyncio.current_task():
            raise ConnectionError("Automatic reconnection is already in progress")
        if self.protocol is not None or self._connect_future is not None:
            raise ConnectionError("Already connected or connecting")
        if self._reconnect_task is not asyncio.current_task():
            self._reconnect_target = None
            self.reconnect_error = None
        loop = asyncio.get_running_loop()
        connect_future = loop.create_future()
        self._connect_future = connect_future

        def protocol_factory():
            def on_event(ev):
                if self.protocol is protocol:
                    self._on_event(ev)

            def on_connection_lost(exc):
                if self.protocol is protocol:
                    error = ConnectionError("MQTT transport connection lost")
                    error.__cause__ = exc
                    self._close_connection(error)

            if self.transport_type == TransportType.QUIC:
                protocol = FlowMqttDatagramProtocol(
                    self.engine, loop, on_event, on_connection_lost,
                )
            else:
                protocol = FlowMqttProtocol(
                    self.engine, self.transport_type, loop, on_event, on_connection_lost,
                )
            # Own the protocol before asyncio calls connection_made, including
            # when endpoint creation is cancelled before returning a transport.
            self.protocol = protocol
            return protocol
        
        async def _do_connect():
            if self.transport_type in (TransportType.TCP, TransportType.TLS):
                # Stream-based connection (TCP or TLS)
                transport, protocol = await loop.create_connection(
                    protocol_factory,
                    host, port
                )
                
            elif self.transport_type == TransportType.QUIC:
                # QUIC connection using FlowMqttDatagramProtocol
                if not server_name:
                    _server_name = host  # Default to host if not specified
                else:
                    _server_name = server_name
                
                # Resolve hostname to IP address for QUIC
                addr_info = await loop.getaddrinfo(
                    host, port,
                    family=socket.AF_UNSPEC,
                    type=socket.SOCK_DGRAM
                )
                if not addr_info:
                    raise ConnectionError(f"Could not resolve {host}")
                
                family = addr_info[0][0]
                server_addr = _format_socket_address(addr_info[0][4])
                
                # Create UDP socket and protocol
                transport, protocol = await loop.create_datagram_endpoint(
                    protocol_factory,
                    family=family,
                    local_addr=("::" if family == socket.AF_INET6 else "0.0.0.0", 0)
                )
                protocol.remote_addr = addr_info[0][4]
                
                # Initiate QUIC connection with required parameters
                if not protocol.closed:
                    now_ms = self.engine.elapsed_ms()
                    if self._zero_rtt_options is None:
                        self.engine.connect(server_addr, _server_name, self.tls_opts, now_ms)
                    else:
                        self.engine.connect_with_zero_rtt(server_addr, _server_name,
                            self.tls_opts, self._zero_rtt_options, now_ms)
            
            # Wait for actual MQTT connection
            return await connect_future

        task = self._connect_task = asyncio.create_task(_do_connect())
        try:
            result = await asyncio.wait_for(task, timeout=timeout)
            self._reconnect_target = (host, port, server_name, timeout)
            self.reconnect_error = None
            return result if return_result else None
        except (Exception, asyncio.CancelledError):
            self._close_connection(ConnectionError("MQTT connection attempt failed"))
            raise
        finally:
            self._finish_future(connect_future)
            self._connect_future = None
            if self._connect_task is task:
                self._connect_task = None

    async def _run_reconnect(self):
        delay = min(self.opts.reconnect_base_delay_ms, self.opts.reconnect_max_delay_ms) / 1000.0
        attempt = 0
        try:
            while self._reconnect_target is not None:
                if self.opts.max_reconnect_attempts and attempt >= self.opts.max_reconnect_attempts:
                    self._fail_pending(self.reconnect_error or ConnectionError("Reconnect attempts exhausted"))
                    return
                attempt += 1
                self._call_callback(self.on_event, flowsdk_ffi.MqttEventFfi.RECONNECT_SCHEDULED(
                    attempt=attempt, delay_ms=int(delay * 1000)))
                await asyncio.sleep(delay)
                if self._reconnect_target is None:
                    return
                try:
                    await self.connect(*self._reconnect_target)
                    return
                except Exception as exc:
                    self.reconnect_error = exc
                    delay = min(delay * 2, self.opts.reconnect_max_delay_ms / 1000.0)
        finally:
            if self._reconnect_task is asyncio.current_task():
                self._reconnect_task = None
            if not self.is_connected:
                self._fail_pending(self.reconnect_error or ConnectionError("Reconnection stopped"))

    async def subscribe(
        self, topic: str, qos: int = 0, *, timeout: Optional[float] = 10.0,
        no_local: bool = False, retain_as_published: bool = False,
        retain_handling: int = 0, properties=None,
    ) -> SubscribeResult:
        """
        Subscribe to a topic.
        
        Args:
            topic: MQTT topic to subscribe to
            qos: Quality of Service level (0, 1, or 2)
            timeout: Seconds to wait for SUBACK; None disables the timeout
            
        Returns:
            SubscribeResult with the packet ID and broker reason codes, including
            rejections. Call result.raise_for_status() to raise on rejection.
            
        Raises:
            ConnectionError: If not connected or the connection fails
            RuntimeError: If the engine cannot queue the subscription
            asyncio.TimeoutError: If the acknowledgement times out
        """
        self._require_connection()
        if properties is not None or no_local or retain_as_published or retain_handling:
            return await self.subscribe_many(
                [Subscription(topic, qos, no_local, retain_as_published, retain_handling)],
                properties=properties, timeout=timeout,
            )
        pid = self.engine.subscribe(topic, qos)
        if pid < 0:
            raise RuntimeError("MQTT subscription could not be queued")
        return await self._wait_for_ack(self._pending_subscribe, pid, timeout)

    async def unsubscribe(
        self, topic: str, *, timeout: Optional[float] = 10.0, properties=None,
    ) -> UnsubscribeResult:
        """
        Unsubscribe from a topic.
        
        Args:
            topic: MQTT topic to unsubscribe from
            timeout: Seconds to wait for UNSUBACK; None disables the timeout
            
        Returns:
            UnsubscribeResult with the packet ID and broker reason codes,
            including rejections. Call result.raise_for_status() to raise on
            rejection. MQTT 3.1.1 acknowledgements have no reason codes.
            
        Raises:
            ConnectionError: If not connected or the connection fails
            RuntimeError: If the engine cannot queue the unsubscription
            asyncio.TimeoutError: If the acknowledgement times out
        """
        self._require_connection()
        if properties is not None:
            return await self.unsubscribe_many([topic], properties=properties, timeout=timeout)
        pid = self.engine.unsubscribe(topic)
        if pid < 0:
            raise RuntimeError("MQTT unsubscription could not be queued")
        return await self._wait_for_ack(self._pending_unsubscribe, pid, timeout)

    async def subscribe_many(self, subscriptions, *, properties=None, timeout: Optional[float] = 10.0, stream_id=None, early_data: bool = False) -> SubscribeResult:
        """Subscribe to ordered Subscription records in one MQTT packet."""
        self._require_connection(early_data=early_data)
        self._check_stream(stream_id)
        options = flowsdk_ffi.MqttSubscribeOptionsFfi(
            subscriptions=[sub.to_ffi() if isinstance(sub, Subscription) else sub for sub in subscriptions],
            properties=property_list(properties),
        )
        pid = (self.engine.subscribe_with_options(options) if stream_id is None
               else self.engine.subscribe_on(stream_id, options))
        return await self._wait_for_ack(self._pending_subscribe, pid, timeout)

    async def unsubscribe_many(self, topics, *, properties=None, timeout: Optional[float] = 10.0, stream_id=None, early_data: bool = False) -> UnsubscribeResult:
        """Unsubscribe from ordered topic filters in one MQTT packet."""
        self._require_connection(early_data=early_data)
        self._check_stream(stream_id)
        options = flowsdk_ffi.MqttUnsubscribeOptionsFfi(topics=list(topics), properties=property_list(properties))
        pid = (self.engine.unsubscribe_with_options(options) if stream_id is None
               else self.engine.unsubscribe_on(stream_id, options))
        return await self._wait_for_ack(self._pending_unsubscribe, pid, timeout)

    async def publish(
        self, topic: str, payload: bytes, qos: int = 0, retain: Optional[bool] = None,
        *, timeout: Optional[float] = 10.0, properties=None, priority: Optional[int] = None,
        return_result: bool = False, stream_id=None, early_data: bool = False,
    ) -> Union[int, PublishResult]:
        """
        Publish a message.
        
        Args:
            topic: MQTT topic to publish to
            payload: Message payload as bytes
            qos: Quality of Service level (0, 1, or 2)
            retain: Whether to retain the message (default: False)
            properties: PublishProperties or an ordered list of MqttPropertyFfi
            priority: Scheduling priority, 0..255 (default: 128)
            timeout: Seconds to wait for a QoS 1/2 acknowledgement; None disables
                the timeout. QoS 0 does not wait for an acknowledgement.
            
        Returns:
            Packet ID of the publish (0 for QoS 0)
            
        Raises:
            ConnectionError: If not connected or the connection fails
            RuntimeError: If the engine cannot queue or the broker rejects the publish
            asyncio.TimeoutError: If the acknowledgement times out
            
        """
        self._require_connection(early_data=early_data)
        self._check_stream(stream_id)
        
        # Handle different engine publish signatures
        if retain is not None or properties is not None or priority is not None or stream_id is not None:
            options = flowsdk_ffi.MqttPublishOptionsFfi(
                qos=qos, retain=bool(retain), priority=priority,
                properties=property_list(properties),
            )
            pid = (self.engine.publish_with_options(topic, payload, options) if stream_id is None
                   else self.engine.publish_on(stream_id, topic, payload, options))
            if pid is None:
                pid = 0
        elif self.transport_type == TransportType.TCP:
            # TCP engine takes 4 args: topic, payload, qos, priority
            # Note: priority is currently passed as None as it's not exposed in high-level API yet
            pid = self.engine.publish(topic, payload, qos, None)
        else:
            # TLS and QUIC engines take 3 args: topic, payload, qos
            pid = self.engine.publish(topic, payload, qos)
        if pid < 0:
            raise RuntimeError("MQTT publish could not be queued")
        
        if qos > 0:
            result = await self._wait_for_ack(self._pending_publish, pid, timeout)
        else:
            # QoS 0: just send immediately, no ack needed
            self.protocol.pump()
            result = PublishResult(None, None, 0)

        if return_result:
            return result
        if result.is_failure:
            raise RuntimeError(f"MQTT publish rejected (reason code 0x{result.reason_code:02x})")
            
        return pid

    @property
    def quic(self):
        """QUIC stream and transport controls."""
        if self.transport_type != TransportType.QUIC:
            raise ValueError("QUIC controls require a QUIC client")
        return QuicControls(self)

    def _check_stream(self, stream_id):
        if stream_id is not None and self.transport_type != TransportType.QUIC:
            raise ValueError("Stream IDs require QUIC")

    def _require_connection(self, *, early_data=False):
        if (early_data and not self._disconnecting and self.transport_type == TransportType.QUIC
                and self.protocol is not None and not self.protocol.closed
                and self.engine.zero_rtt_status() == flowsdk_ffi.QuicZeroRttStatusFfi.ATTEMPTED):
            return
        if not self.is_connected:
            raise ConnectionError("Not connected")

    @property
    def is_connected(self) -> bool:
        return bool(not self._disconnecting and self.protocol is not None
                    and not self.protocol.closed and self.engine.is_connected())

    @property
    def mqtt_version(self) -> int:
        return self.opts.mqtt_version

    @staticmethod
    def _finish_future(fut):
        if not fut.done():
            fut.cancel()
        elif not fut.cancelled():
            # A synchronous pump/setup exception may bypass awaiting the future.
            fut.exception()

    async def _wait_for_ack(self, pending, pid, timeout):
        fut = asyncio.get_running_loop().create_future()
        pending[pid] = fut
        try:
            self.protocol.pump()
            return await asyncio.wait_for(fut, timeout)
        finally:
            if pending.get(pid) is fut:
                del pending[pid]
            self._finish_future(fut)

    def _fail_pending(self, error, *, retain_ack=False):
        futures = []
        if self._connect_future is not None:
            futures.append(self._connect_future)
        groups = (self._pending_ping,) if retain_ack else (
            self._pending_publish, self._pending_subscribe, self._pending_unsubscribe, self._pending_ping)
        for pending in groups:
            futures.extend(pending.values())
            pending.clear()
        for fut in futures:
            if not fut.done():
                fut.set_exception(error)

    def _close_connection(self, error):
        protocol, self.protocol = self.protocol, None
        retry = bool(self.auto_reconnect and self._reconnect_target is not None
                     and not self._disconnecting and not self._retired)
        self._fail_pending(error, retain_ack=retry)
        if protocol is not None:
            protocol.close()
        if retry and self._reconnect_task is None:
            self._reconnect_task = asyncio.create_task(self._run_reconnect())
    
    @property
    def auto_reconnect(self) -> bool:
        return self._auto_reconnect

    @auto_reconnect.setter
    def auto_reconnect(self, enabled: bool):
        self.set_auto_reconnect(enabled)

    def set_auto_reconnect(self, enabled: bool):
        """Change the host retry policy; disabling cancels an outstanding retry."""
        self._auto_reconnect = enabled
        if not enabled and self._reconnect_task is not None:
            task, self._reconnect_task = self._reconnect_task, None
            # A task canceled before its first turn never enters its finally block.
            task.cancel()
            self._fail_pending(ConnectionError("Automatic reconnection disabled"))

    @staticmethod
    def _require_durable_session():
        if not hasattr(flowsdk_ffi, "inspect_session_state"):
            raise flowsdk_ffi.MqttErrorFfi.Unsupported(
                detail="Durable sessions require a build with the durable-session feature")

    async def checkpoint_for_restart(self) -> bytes:
        """Retire this client abruptly and return a stable checkpoint.

        This can trigger the Will. Persist the returned bytes before constructing
        the replacement client. Pending Python futures are failed, not persisted.
        This is planned restart support, not automatic crash-safe processing.
        """
        self._require_durable_session()
        if not self._session_peer:
            raise ValueError("Checkpointing requires runtime_options.peer")
        if self._retired or self._disconnecting:
            raise ConnectionError("Client is already retiring or retired")
        # Validate checkpoint eligibility before retiring a healthy connection.
        self.engine.snapshot_session()
        self._retired = True
        self._disconnecting = True
        self._reconnect_target = None
        tasks = {task for task in (self._reconnect_task, self._connect_task)
                 if task is not None and task is not asyncio.current_task()}
        for task in tasks:
            task.cancel()
        protocol, self.protocol = self.protocol, None
        self._fail_pending(ConnectionError("Client retired for restart"))
        try:
            if protocol is not None:
                protocol.close(abort=True)
            if tasks:
                await asyncio.gather(*tasks, return_exceptions=True)
            return bytes(self.engine.snapshot_session())
        finally:
            self._disconnecting = False

    def pump(self):
        """
        Force data transmission and process pending events.
        
        This can be useful if the automatic tick loop is not running or if
        you want to ensure data is sent immediately.
        """
        if self.protocol:
            self.protocol.pump()

    def set_parse_level(self, level):
        """Select Full, HeadersParsed, or TypeOnly after connecting; raw bodies are unsupported."""
        self._require_connection()
        if any((self._pending_publish, self._pending_subscribe, self._pending_unsubscribe)):
            raise RuntimeError("Cannot change parser level while acknowledgements are pending")
        self.engine.set_parse_level(level)

    async def acknowledge(self, packet_id: int, qos: int, *, stream_id=None,
                          reason_code: int = 0, properties=None):
        """Send PUBACK (QoS 1) or PUBREC (QoS 2) with auto_ack=False."""
        self._require_connection()
        if qos not in (1, 2):
            raise ValueError("Manual acknowledgement QoS must be 1 or 2")
        kind = (flowsdk_ffi.MqttAcknowledgementFfi.PUB_ACK if qos == 1
                else flowsdk_ffi.MqttAcknowledgementFfi.PUB_REC)
        self.engine.acknowledge(kind, packet_id, reason_code, property_list(properties), stream_id)
        self.protocol.pump()

    async def complete_qos2(self, packet_id: int, *, stream_id=None,
                            reason_code: int = 0, properties=None):
        """Send PUBCOMP after the matching PubRelReceived event with auto_ack=False."""
        self._require_connection()
        self.engine.acknowledge(flowsdk_ffi.MqttAcknowledgementFfi.PUB_COMP,
            packet_id, reason_code, property_list(properties), stream_id)
        self.protocol.pump()

    async def auth(self, reason_code: int = 0x18, *, properties=None):
        """Send MQTT 5 AUTH during CONNECT or reauthentication.

        Challenges arrive through on_event as AuthReceived events. CONNECT must
        include an authentication method; this call only waits for local submission.
        """
        if self.protocol is None or self.protocol.closed or self._disconnecting:
            raise ConnectionError("No active connection attempt or session")
        self.engine.auth_with_properties(reason_code, property_list(properties))
        self.protocol.pump()

    async def ping(self, *, timeout: Optional[float] = 10.0) -> bool:
        """Send MQTT PINGREQ and wait for PINGRESP. Only one ping may wait at a time."""
        self._require_connection()
        if self._pending_ping:
            raise RuntimeError("An MQTT ping is already pending")
        self.engine.ping()
        return await self._wait_for_ack(self._pending_ping, 0, timeout)

    async def disconnect(self, reason_code: int = 0, *, properties=None, timeout: Optional[float] = 5.0):
        """
        Drive MQTT DISCONNECT and transport output before closing the socket.

        Raises asyncio.TimeoutError if transport flow control prevents flushing
        within timeout seconds. The socket is closed even on error/cancellation.
        This does not wait for broker acknowledgement or outstanding publishes.
        """
        self._disconnecting = True
        self._reconnect_target = None
        tasks = {task for task in (self._reconnect_task, self._connect_task)
                 if task is not None and task is not asyncio.current_task()}
        for task in tasks:
            task.cancel()
        protocol = self.protocol
        self._fail_pending(ConnectionError("MQTT client disconnected"))

        async def flush():
            if reason_code or properties is not None:
                self.engine.disconnect_with_options(flowsdk_ffi.MqttDisconnectOptionsFfi(
                    reason_code=reason_code, properties=property_list(properties)))
            else:
                self.engine.disconnect()
            while not protocol.closed:
                self.engine.handle_tick(self.engine.elapsed_ms())
                protocol.pump()
                transport = getattr(protocol, "transport", None)
                buffered = getattr(transport, "get_write_buffer_size", lambda: 0)()
                if self.engine.disconnect_complete() and not buffered:
                    protocol.close()
                    break
                await asyncio.sleep(0.01)
            await protocol.wait_closed()

        try:
            if tasks:
                await asyncio.gather(*tasks, return_exceptions=True)
            if protocol is not None and not protocol.closed:
                # Local shutdown always has a finite bound, even when protocol
                # ACK waits are disabled by timeout=None.
                await asyncio.wait_for(flush(), 5.0 if timeout is None else timeout)
        finally:
            self._close_connection(ConnectionError("MQTT client disconnected"))
            if protocol is not None and protocol.transport is not None and not protocol._transport_closed.done():
                protocol.transport.abort()
            if self._reconnect_task in tasks:
                self._reconnect_task = None
            if self._connect_task in tasks:
                self._connect_task = None
            self._disconnecting = False

    def _on_event(self, ev):
        """Internal event handler."""
        try:
            self._handle_event(ev)
        finally:
            self._call_callback(self.on_event, ev)

    @staticmethod
    def _call_callback(callback, *args):
        if callback is not None:
            try:
                callback(*args)
            except Exception:
                logger.exception("MQTT callback failed")

    def _handle_event(self, ev):
        if ev.is_connected():
            res = ev[0]
            self.connection_result = ConnectionResult(
                res.reason_code, res.session_present, list(getattr(res, "properties", [])),
            )
            if self._connect_future and not self._connect_future.done():
                if res.reason_code == 0:
                    self._connect_future.set_result(self.connection_result)
                else:
                    self._connect_future.set_exception(ConnectionError(
                        f"MQTT connection rejected (reason code 0x{res.reason_code:02x})"
                    ))
        
        elif ev.is_published():
            res = ev[0]
            pid = res.packet_id
            if pid in self._pending_publish:
                fut = self._pending_publish.pop(pid)
                if not fut.done():
                    fut.set_result(PublishResult(res.packet_id, res.reason_code, res.qos,
                        list(getattr(res, "properties", []))))

        elif ev.is_subscribed():
            res = ev[0]
            pid = res.packet_id
            if pid in self._pending_subscribe:
                fut = self._pending_subscribe.pop(pid)
                if not fut.done():
                    fut.set_result(SubscribeResult(res.packet_id, list(res.reason_codes),
                        list(getattr(res, "properties", []))))

        elif ev.is_unsubscribed():
            res = ev[0]
            pid = res.packet_id
            if pid in self._pending_unsubscribe:
                fut = self._pending_unsubscribe.pop(pid)
                if not fut.done():
                    fut.set_result(UnsubscribeResult(res.packet_id, list(res.reason_codes),
                        list(getattr(res, "properties", []))))

        elif ev.is_message_received():
            msg = ev[0]
            self._call_callback(self.on_message, msg.topic, msg.payload, msg.qos)
            self._call_callback(self.on_message_full, msg)
            
        elif ev.is_ping_response():
            fut = self._pending_ping.pop(0, None)
            if fut is not None and not fut.done():
                fut.set_result(ev.success)

        elif ev.is_operation_failed():
            error = OperationFailedError(ev)
            operation = flowsdk_ffi.MqttOperationKindFfi
            if ev.operation == operation.CONNECT:
                self._close_connection(error)
            else:
                pending = {operation.PUBLISH: self._pending_publish,
                           operation.SUBSCRIBE: self._pending_subscribe,
                           operation.UNSUBSCRIBE: self._pending_unsubscribe}[ev.operation]
                fut = pending.pop(ev.packet_id, None)
                if fut is not None and not fut.done():
                    fut.set_exception(error)

        elif ev.is_error():
            self._close_connection(ConnectionError(f"MQTT connection error: {ev.message}"))
            logger.error(f"MQTT Error: {ev.message}")

        elif ev.is_transport_closed():
            self._close_connection(ConnectionError("QUIC transport closed: " + ev.reason))

        elif ev.is_disconnected():
            self._close_connection(ConnectionError("MQTT disconnected"))

        elif ev.is_reconnect_needed():
            self._close_connection(ConnectionError("MQTT connection lost; reconnect required"))


__all__ = ['FlowMqttClient', 'FlowMqttProtocol', 'TransportType', 'FlowMqttDatagramProtocol', 'OperationFailedError']
