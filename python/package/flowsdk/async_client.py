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
    ...     await client.subscribe("test/topic", 1)
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
    ...     await client.subscribe("test/topic", 1)
    ...     await client.publish("test/topic", b"Hello", 1)
    ...     await client.disconnect()
    >>> 
    >>> asyncio.run(example())
"""

import asyncio
import logging
import socket
import time
from enum import Enum
from typing import Optional, Dict, Callable, Any, List, Union

try:
    from . import flowsdk_ffi
except ImportError:
    import flowsdk_ffi


logger = logging.getLogger(__name__)
logger.addHandler(logging.NullHandler())


class TransportType(Enum):
    """MQTT transport protocol types."""
    TCP = "tcp"
    TLS = "tls"
    QUIC = "quic"


class FlowMqttProtocol(asyncio.Protocol):
    """
    asyncio Protocol implementation for MQTT engine (TCP/TLS).
    
    This class handles the network layer, manages engine ticks, and pumps
    data between the network transport and the MQTT engine.
    
    Args:
        engine: The MQTT engine instance
        transport_type: The transport protocol being used
        loop: The asyncio event loop
        on_event_cb: Callback function to handle MQTT events
    """
    
    def __init__(self, engine, transport_type: TransportType, loop: asyncio.AbstractEventLoop, on_event_cb: Callable):
        self.engine = engine
        self.transport_type = transport_type
        self.loop = loop
        self.transport: Optional[asyncio.Transport] = None
        self.on_event_cb = on_event_cb
        self.start_time = time.monotonic()
        self._tick_handle: Optional[asyncio.TimerHandle] = None
        self.closed = False

    def connection_made(self, transport: asyncio.Transport):
        """Called when connection is established."""
        self.transport = transport
        
        # Start the MQTT connection
        if hasattr(self.engine, 'connect'):
            self.engine.connect()
        
        # Start the tick loop immediately
        self._schedule_tick(0)

    def data_received(self, data: bytes):
        """Called when data is received from the network."""
        # Feed network data to engine
        if self.transport_type == TransportType.TLS:
            self.engine.handle_socket_data(data)
        else:
            self.engine.handle_incoming(data)
            
        # Trigger an immediate pump to process potential responses
        self.pump()

    def connection_lost(self, exc: Optional[Exception]):
        """Called when the connection is closed."""
        self.closed = True
        # Only TCP engine has handle_connection_lost
        if self.transport_type == TransportType.TCP and hasattr(self.engine, 'handle_connection_lost'):
            self.engine.handle_connection_lost()
            
        if self._tick_handle:
            self._tick_handle.cancel()
        
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
        now_ms = int((time.monotonic() - self.start_time) * 1000)
        self.engine.handle_tick(now_ms)
        
        self.pump()
        
        # Schedule next tick
        if self.transport_type == TransportType.TCP:
            next_tick_ms = self.engine.next_tick_ms()
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
        # 1. Send outgoing data
        if self.transport_type == TransportType.TLS:
            outgoing = self.engine.take_socket_data()
        else:
            outgoing = self.engine.take_outgoing()
            
        if outgoing and self.transport and not self.transport.is_closing():
            self.transport.write(outgoing)
        
        # 2. Process events
        events = self.engine.take_events()
        for ev in events:
            self.on_event_cb(ev)


class FlowMqttDatagramProtocol(asyncio.DatagramProtocol):
    """
    asyncio DatagramProtocol implementation for QUIC MQTT engine.
    
    This class handles UDP datagram transport for QUIC, managing engine ticks
    and pumping data between the network transport and the MQTT engine.
    
    Args:
        engine: The QUIC MQTT engine instance from flowsdk_ffi
        loop: The asyncio event loop
        on_event_cb: Callback function to handle MQTT events
    """
    
    def __init__(self, engine, loop: asyncio.AbstractEventLoop, on_event_cb: Callable):
        self.engine = engine
        self.loop = loop
        self.transport: Optional[asyncio.DatagramTransport] = None
        self.on_event_cb = on_event_cb
        self.start_time = time.monotonic()
        self._tick_handle: Optional[asyncio.TimerHandle] = None
        self.closed = False
        self.remote_addr = None

    def connection_made(self, transport: asyncio.DatagramTransport):
        """Called when UDP socket is ready."""
        self.transport = transport
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
        # Feed datagram to QUIC engine
        self.remote_addr = addr
        now_ms = int((time.monotonic() - self.start_time) * 1000)
        addr_str = f"{addr[0]}:{addr[1]}"
        self.engine.handle_datagram(data, addr_str, now_ms)
        # Trigger an immediate pump to process responses
        self.pump()

    def error_received(self, exc: Exception):
        """Called when an error is received."""
        pass  # UDP errors are generally non-fatal

    def connection_lost(self, exc: Optional[Exception]):
        """Called when the connection is closed."""
        self.closed = True
        if self._tick_handle:
            self._tick_handle.cancel()

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
        now_ms = int((time.monotonic() - self.start_time) * 1000)
        # QUIC engine handle_tick returns events
        events = self.engine.handle_tick(now_ms)
        for ev in events:
            self.on_event_cb(ev)
        
        self.pump()
        
        # Fixed 10ms interval for QUIC
        delay = 0.01
        self._schedule_tick(delay)

    def pump(self):
        """Pump data between engine and network."""
        # 1. Send outgoing datagrams
        datagrams = self.engine.take_outgoing_datagrams()
        if datagrams and self.transport and self.remote_addr:
            for datagram in datagrams:
                # Extract bytes from MqttDatagramFfi
                self.transport.sendto(datagram.data, self.remote_addr)
        
        # 2. Process events
        events = self.engine.take_events()
        for ev in events:
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
        password: Optional[str] = None,
        reconnect_base_delay_ms: int = 1000,
        reconnect_max_delay_ms: int = 30000,
        max_reconnect_attempts: int = 0,
        on_message: Optional[Callable[[str, bytes, int], None]] = None,
        ca_cert_file: Optional[str] = None,
        client_cert_file: Optional[str] = None,
        client_key_file: Optional[str] = None,
        insecure_skip_verify: bool = False,
        alpn_protocols: Optional[List[str]] = None,
        server_name: Optional[str] = None
    ):
        self.transport_type = transport
        self.server_name = server_name
        self.opts = flowsdk_ffi.MqttOptionsFfi(
            client_id=client_id,
            mqtt_version=mqtt_version,
            clean_start=clean_start,
            keep_alive=keep_alive,
            username=username,
            password=password,
            reconnect_base_delay_ms=reconnect_base_delay_ms,
            reconnect_max_delay_ms=reconnect_max_delay_ms,
            max_reconnect_attempts=max_reconnect_attempts
        )
        
        # Common TLS/QUIC options
        self.tls_opts = flowsdk_ffi.MqttTlsOptionsFfi(
            ca_cert_file=ca_cert_file,
            client_cert_file=client_cert_file,
            client_key_file=client_key_file,
            insecure_skip_verify=insecure_skip_verify,
            alpn_protocols=alpn_protocols or ["mqtt"]
        )
        
        # Create appropriate engine based on transport type
        if transport == TransportType.TCP:
            self.engine = flowsdk_ffi.MqttEngineFfi.new_with_opts(self.opts)
        elif transport == TransportType.TLS:
            if not server_name:
                raise ValueError("server_name (SNI) is required for TLS transport")
            self.engine = flowsdk_ffi.TlsMqttEngineFfi(self.opts, self.tls_opts, server_name)
        elif transport == TransportType.QUIC:
            self.engine = flowsdk_ffi.QuicMqttEngineFfi(self.opts)
        else:
            raise ValueError(f"Unsupported transport type: {transport}")
        
        self.protocol: Optional[Union[FlowMqttProtocol, FlowMqttDatagramProtocol]] = None
        self.on_message = on_message
        
        # Futures for pending operations
        self._connect_future: Optional[asyncio.Future] = None
        self._pending_publish: Dict[int, asyncio.Future] = {}
        self._pending_subscribe: Dict[int, asyncio.Future] = {}
        self._pending_unsubscribe: Dict[int, asyncio.Future] = {}

    async def connect(self, host: str, port: int, server_name: Optional[str] = None, timeout: float = 10.0):
        """
        Connect to MQTT broker.
        
        Args:
            host: Broker hostname or IP address
            port: Broker port number
            server_name: Server name for TLS SNI (required for QUIC)
            timeout: Connection timeout in seconds (default: 10.0)
            
        Raises:
            ConnectionError: If connection fails
            asyncio.TimeoutError: If connection times out
        """
        loop = asyncio.get_running_loop()
        self._connect_future = loop.create_future()
        
        async def _do_connect():
            if self.transport_type in (TransportType.TCP, TransportType.TLS):
                # Stream-based connection (TCP or TLS)
                transport, protocol = await loop.create_connection(
                    lambda: FlowMqttProtocol(self.engine, self.transport_type, loop, self._on_event),
                    host, port
                )
                self.protocol = protocol
                
            elif self.transport_type == TransportType.QUIC:
                # QUIC connection using FlowMqttDatagramProtocol
                if not server_name:
                    _server_name = host  # Default to host if not specified
                else:
                    _server_name = server_name
                
                # Resolve hostname to IP address for QUIC
                addr_info = await loop.getaddrinfo(
                    host, port,
                    family=socket.AF_INET,
                    type=socket.SOCK_DGRAM
                )
                if not addr_info:
                    raise ConnectionError(f"Could not resolve {host}")
                
                server_addr = f"{addr_info[0][4][0]}:{port}"
                
                # Create UDP socket and protocol
                transport, protocol = await loop.create_datagram_endpoint(
                    lambda: FlowMqttDatagramProtocol(self.engine, loop, self._on_event),
                    remote_addr=addr_info[0][4]
                )
                self.protocol = protocol
                protocol.remote_addr = addr_info[0][4]
                
                # Initiate QUIC connection with required parameters
                now_ms = int((time.monotonic() - protocol.start_time) * 1000)
                self.engine.connect(server_addr, _server_name, self.tls_opts, now_ms)
            
            # Wait for actual MQTT connection
            if self._connect_future:
                await self._connect_future

        try:
            await asyncio.wait_for(_do_connect(), timeout=timeout)
        except Exception:
            # Clean up on failure
            self.protocol = None
            if self._connect_future and not self._connect_future.done():
                self._connect_future.cancel()
            raise

    async def subscribe(self, topic: str, qos: int = 0) -> int:
        """
        Subscribe to a topic.
        
        Args:
            topic: MQTT topic to subscribe to
            qos: Quality of Service level (0, 1, or 2)
            
        Returns:
            Packet ID of the subscription
            
        Raises:
            RuntimeError: If not connected
        """
        if not self.protocol:
            raise RuntimeError("Not connected")
            
        loop = asyncio.get_running_loop()
        pid = self.engine.subscribe(topic, qos)
        fut = loop.create_future()
        self._pending_subscribe[pid] = fut
        
        # Force a pump to send the packet immediately
        self.protocol.pump()
        
        await fut
        return pid

    async def unsubscribe(self, topic: str) -> int:
        """
        Unsubscribe from a topic.
        
        Args:
            topic: MQTT topic to unsubscribe from
            
        Returns:
            Packet ID of the unsubscription
            
        Raises:
            RuntimeError: If not connected
        """
        if not self.protocol:
            raise RuntimeError("Not connected")
            
        loop = asyncio.get_running_loop()
        pid = self.engine.unsubscribe(topic)
        fut = loop.create_future()
        self._pending_unsubscribe[pid] = fut
        
        # Force a pump to send the packet immediately
        self.protocol.pump()
        
        await fut
        return pid

    async def publish(self, topic: str, payload: bytes, qos: int = 0, retain: Optional[bool] = None) -> int:
        """
        Publish a message.
        
        Args:
            topic: MQTT topic to publish to
            payload: Message payload as bytes
            qos: Quality of Service level (0, 1, or 2)
            retain: Whether to retain the message (None uses default, ignored for QUIC)
            
        Returns:
            Packet ID of the publish (0 for QoS 0)
            
        Raises:
            RuntimeError: If not connected
            
        Note:
            QUIC transport does not support the retain parameter and priority.
        """
        if not self.protocol:
            raise RuntimeError("Not connected")
            
        loop = asyncio.get_running_loop()
        
        # Handle different engine publish signatures
        if self.transport_type == TransportType.TCP:
            # TCP engine takes 4 args: topic, payload, qos, priority
            # Note: priority is currently passed as None as it's not exposed in high-level API yet
            pid = self.engine.publish(topic, payload, qos, None)
        else:
            # TLS and QUIC engines take 3 args: topic, payload, qos
            pid = self.engine.publish(topic, payload, qos)
        
        if qos > 0:
            fut = loop.create_future()
            self._pending_publish[pid] = fut
            
            # Force a pump to send the packet immediately
            self.protocol.pump()
            
            await fut
        else:
            # QoS 0: just send immediately, no ack needed
            self.protocol.pump()
            
        return pid
    
    def pump(self):
        """
        Force data transmission and process pending events.
        
        This can be useful if the automatic tick loop is not running or if
        you want to ensure data is sent immediately.
        """
        if self.protocol:
            self.protocol.pump()

    async def disconnect(self):
        """
        Disconnect from the broker gracefully.
        """
        if not self.protocol:
            return
            
        self.engine.disconnect()
        self.protocol.pump()  # Flush DISCONNECT packet
        
        if self.protocol.transport:
            self.protocol.transport.close()
            self.protocol.closed = True

    def _on_event(self, ev):
        """Internal event handler."""
        if ev.is_connected():
            if self._connect_future and not self._connect_future.done():
                self._connect_future.set_result(True)
        
        elif ev.is_published():
            res = ev[0]
            pid = res.packet_id
            if pid in self._pending_publish:
                fut = self._pending_publish.pop(pid)
                if not fut.done():
                    fut.set_result(res)

        elif ev.is_subscribed():
            res = ev[0]
            pid = res.packet_id
            if pid in self._pending_subscribe:
                fut = self._pending_subscribe.pop(pid)
                if not fut.done():
                    fut.set_result(res)

        elif ev.is_unsubscribed():
            res = ev[0]
            pid = res.packet_id
            if pid in self._pending_unsubscribe:
                fut = self._pending_unsubscribe.pop(pid)
                if not fut.done():
                    fut.set_result(res)

        elif ev.is_message_received():
            msg = ev[0]
            if self.on_message:
                self.on_message(msg.topic, msg.payload, msg.qos)
            
        elif ev.is_error():
            if self._connect_future and not self._connect_future.done():
                self._connect_future.set_exception(ConnectionError(f"MQTT connection error: {ev.message}"))
            logger.error(f"MQTT Error: {ev.message}")

        elif ev.is_disconnected():
            if self._connect_future and not self._connect_future.done():
                self._connect_future.set_exception(ConnectionError("MQTT disconnected during connection process"))


__all__ = ['FlowMqttClient', 'FlowMqttProtocol','TransportType', 'FlowMqttDatagramProtocol']
