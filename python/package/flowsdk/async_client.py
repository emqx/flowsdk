"""
A production-ready async MQTT client implementation using flowsdk-ffi with asyncio.

This module provides an asyncio-based MQTT client that wraps the flowsdk_ffi
library, providing async/await support for MQTT operations. It implements a
custom Protocol for handling the network layer and manages the timing and
pumping of the MQTT engine.

Classes:
    FlowMqttProtocol: asyncio.Protocol implementation that manages the network
                      layer and handles engine ticks and data pumping.
    FlowMqttClient: High-level async MQTT client providing connect, subscribe,
                    publish, and disconnect operations.

Example:
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
"""

import asyncio
import time
from typing import Optional, Dict, Callable, Any

try:
    from . import flowsdk_ffi
except ImportError:
    import flowsdk_ffi


class FlowMqttProtocol(asyncio.Protocol):
    """
    asyncio Protocol implementation for MQTT engine.
    
    This class handles the network layer, manages engine ticks, and pumps
    data between the network transport and the MQTT engine.
    
    Args:
        engine: The MQTT engine instance from flowsdk_ffi
        loop: The asyncio event loop
        on_event_cb: Callback function to handle MQTT events
    """
    
    def __init__(self, engine, loop: asyncio.AbstractEventLoop, on_event_cb: Callable):
        self.engine = engine
        self.loop = loop
        self.transport: Optional[asyncio.Transport] = None
        self.on_event_cb = on_event_cb
        self.start_time = time.monotonic()
        self._tick_handle: Optional[asyncio.TimerHandle] = None
        self.closed = False

    def connection_made(self, transport: asyncio.Transport):
        """Called when TCP connection is established."""
        self.transport = transport
        
        # Start the MQTT connection
        self.engine.connect()
        
        # Start the tick loop immediately
        self._schedule_tick(0)

    def data_received(self, data: bytes):
        """Called when data is received from the network."""
        # Feed network data to engine
        self.engine.handle_incoming(data)
        # Trigger an immediate pump to process potential responses
        self._pump()

    def connection_lost(self, exc: Optional[Exception]):
        """Called when the connection is closed."""
        self.closed = True
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
        
        self._pump()
        
        # Schedule next tick
        next_tick_ms = self.engine.next_tick_ms()
        if next_tick_ms < 0:
            delay = 0.1
        else:
            delay = max(0, (next_tick_ms - now_ms) / 1000.0)
            
        self._schedule_tick(delay)

    def _pump(self):
        """Pump data between engine and network."""
        # 1. Send outgoing data
        outgoing = self.engine.take_outgoing()
        if outgoing and self.transport and not self.transport.is_closing():
            self.transport.write(outgoing)
        
        # 2. Process events
        events = self.engine.take_events()
        for ev in events:
            self.on_event_cb(ev)


class FlowMqttClient:
    """
    High-level async MQTT client.
    
    This client provides a simple async/await API for MQTT operations,
    handling connection management, subscriptions, and publishing.
    
    Args:
        client_id: MQTT client identifier
        mqtt_version: MQTT protocol version (3 or 5, default: 5)
        clean_start: Whether to start a clean session (default: True)
        keep_alive: Keep-alive interval in seconds (default: 30)
        username: Optional username for authentication
        password: Optional password for authentication
        reconnect_base_delay_ms: Base delay for reconnection attempts (default: 1000)
        reconnect_max_delay_ms: Maximum delay for reconnection (default: 30000)
        max_reconnect_attempts: Maximum reconnection attempts, 0 for infinite (default: 0)
        on_message: Optional callback for received messages: fn(topic: str, payload: bytes, qos: int)
    
    Example:
        >>> async def example():
        ...     client = FlowMqttClient("my_client", on_message=lambda t, p, q: print(f"{t}: {p}"))
        ...     await client.connect("broker.emqx.io", 1883)
        ...     await client.subscribe("test/topic", 1)
        ...     await client.publish("test/topic", b"Hello", 1)
        ...     await asyncio.sleep(1)  # Wait for message
        ...     await client.disconnect()
    """
    
    def __init__(
        self,
        client_id: str,
        mqtt_version: int = 5,
        clean_start: bool = True,
        keep_alive: int = 30,
        username: Optional[str] = None,
        password: Optional[str] = None,
        reconnect_base_delay_ms: int = 1000,
        reconnect_max_delay_ms: int = 30000,
        max_reconnect_attempts: int = 0,
        on_message: Optional[Callable[[str, bytes, int], None]] = None
    ):
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
        self.engine = flowsdk_ffi.MqttEngineFfi.new_with_opts(self.opts)
        self.protocol: Optional[FlowMqttProtocol] = None
        self.on_message = on_message
        
        # Futures for pending operations
        self._connect_future: Optional[asyncio.Future] = None
        self._pending_publish: Dict[int, asyncio.Future] = {}
        self._pending_subscribe: Dict[int, asyncio.Future] = {}
        self._pending_unsubscribe: Dict[int, asyncio.Future] = {}

    async def connect(self, host: str, port: int):
        """
        Connect to MQTT broker.
        
        Args:
            host: Broker hostname or IP address
            port: Broker port number
            
        Raises:
            ConnectionError: If connection fails
        """
        loop = asyncio.get_running_loop()
        self._connect_future = loop.create_future()
        
        transport, protocol = await loop.create_connection(
            lambda: FlowMqttProtocol(self.engine, loop, self._on_event),
            host, port
        )
        self.protocol = protocol
        
        # Wait for actual MQTT connection
        await self._connect_future

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
        self.protocol._pump()
        
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
        self.protocol._pump()
        
        await fut
        return pid

    async def publish(self, topic: str, payload: bytes, qos: int = 0, retain: Optional[bool] = None) -> int:
        """
        Publish a message.
        
        Args:
            topic: MQTT topic to publish to
            payload: Message payload as bytes
            qos: Quality of Service level (0, 1, or 2)
            retain: Whether to retain the message (None uses default)
            
        Returns:
            Packet ID of the publish (0 for QoS 0)
            
        Raises:
            RuntimeError: If not connected
        """
        if not self.protocol:
            raise RuntimeError("Not connected")
            
        loop = asyncio.get_running_loop()
        pid = self.engine.publish(topic, payload, qos, retain)
        
        if qos > 0:
            fut = loop.create_future()
            self._pending_publish[pid] = fut
            
            # Force a pump to send the packet immediately
            self.protocol._pump()
            
            await fut
        else:
            # QoS 0: just send immediately, no ack needed
            self.protocol._pump()
            
        return pid
    
    async def disconnect(self):
        """
        Disconnect from the broker gracefully.
        """
        if not self.protocol:
            return
            
        self.engine.disconnect()
        self.protocol._pump()  # Flush DISCONNECT packet
        
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
            
        elif ev.is_disconnected():
            pass


__all__ = ['FlowMqttClient', 'FlowMqttProtocol']
