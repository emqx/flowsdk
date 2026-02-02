import asyncio
"""
A proper async MQTT client implementation using flowsdk-ffi with asyncio.
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
    >>> async def example():
    ...     client = FlowMqttClient("my_client_id")
    ...     await client.connect("broker.emqx.io", 1883)
    ...     await client.subscribe("test/topic", 1)
    ...     await client.publish("test/topic", b"Hello", 1)
    ...     await client.disconnect()
"""
import flowsdk_ffi
import time
import socket
from typing import Optional, Dict, Any




class FlowMqttProtocol(asyncio.Protocol):
    def __init__(self, engine: flowsdk_ffi.MqttEngineFfi, loop: asyncio.AbstractEventLoop, on_event_cb):
        self.engine = engine
        self.loop = loop
        self.transport: Optional[asyncio.Transport] = None
        self.on_event_cb = on_event_cb
        self.start_time = time.monotonic()
        self._tick_handle: Optional[asyncio.TimerHandle] = None
        self.closed = False

    def connection_made(self, transport: asyncio.Transport):
        self.transport = transport
        print("Protocol: Connection made")
        
        # Start the engine
        self.engine.connect()
        
        # Start the tick loop immediately
        self._schedule_tick(0)

    def data_received(self, data: bytes):
        # Feed network data to engine
        self.engine.handle_incoming(data)
        # Trigger an immediate pump to process potential responses
        self._pump()

    def connection_lost(self, exc: Optional[Exception]):
        print(f"Protocol: Connection lost: {exc}")
        self.closed = True
        self.engine.handle_connection_lost()
        if self._tick_handle:
            self._tick_handle.cancel()
        
    def _schedule_tick(self, delay_sec: float):
        if self.closed:
            return
        # Cancel previous handle if it exists to avoid overlaps (though logic shouldn't allow it)
        if self._tick_handle:
            self._tick_handle.cancel()
            
        self._tick_handle = self.loop.call_later(delay_sec, self._on_timer)

    def _on_timer(self):
        if self.closed:
            return
        
        # Run protocol tick
        now_ms = int((time.monotonic() - self.start_time) * 1000)
        self.engine.handle_tick(now_ms)
        
        self._pump()
        
        # Schedule next tick
        next_tick_ms = self.engine.next_tick_ms()
        if next_tick_ms < 0:
            # Should not happen typically unless engine is dead
            delay = 0.1
        else:
            delay = max(0, (next_tick_ms - now_ms) / 1000.0)
            
        self._schedule_tick(delay)

    def _pump(self):
        # 1. Send outgoing data
        outgoing = self.engine.take_outgoing()
        if outgoing and self.transport and not self.transport.is_closing():
            self.transport.write(outgoing)
        
        # 2. Process events
        events = self.engine.take_events()
        for ev in events:
            self.on_event_cb(ev)

class FlowMqttClient:
    def __init__(self, client_id: str):
        self.opts = flowsdk_ffi.MqttOptionsFfi(
            client_id=client_id,
            mqtt_version=5,
            clean_start=True,
            keep_alive=30,
            username=None,
            password=None,
            reconnect_base_delay_ms=1000,
            reconnect_max_delay_ms=30000,
            max_reconnect_attempts=0
        )
        self.engine = flowsdk_ffi.MqttEngineFfi.new_with_opts(self.opts)
        self.loop = asyncio.get_running_loop()
        self.protocol: Optional[FlowMqttProtocol] = None
        
        # Futures for pending operations
        # Connect is special, tracked by a single future
        self._connect_future: asyncio.Future = self.loop.create_future()
        
        # Map PacketID -> Future
        self._pending_publish: Dict[int, asyncio.Future] = {}
        self._pending_subscribe: Dict[int, asyncio.Future] = {}

    async def connect(self, host: str, port: int):
        print(f"Client: Connecting to {host}:{port}...")
        
        transport, protocol = await self.loop.create_connection(
            lambda: FlowMqttProtocol(self.engine, self.loop, self._on_event),
            host, port
        )
        self.protocol = protocol
        
        # Wait for actual MQTT connection
        await self._connect_future
        print("Client: MQTT Connected!")

    async def subscribe(self, topic: str, qos: int) -> int:
        pid = self.engine.subscribe(topic, qos)
        fut = self.loop.create_future()
        self._pending_subscribe[pid] = fut
        
        # Force a pump to send the packet immediately
        if self.protocol:
            self.protocol._pump()
            
        print(f"Client: Awaiting Ack for Subscribe (PID: {pid})...")
        await fut
        return pid

    async def publish(self, topic: str, payload: bytes, qos: int) -> int:
        pid = self.engine.publish(topic, payload, qos, None)
        fut = self.loop.create_future()
        self._pending_publish[pid] = fut
        
        # Force a pump to send the packet immediately
        if self.protocol:
            self.protocol._pump()
            
        print(f"Client: Awaiting Ack for Publish (PID: {pid})...")
        await fut
        return pid
    
    async def disconnect(self):
        print("Client: Disconnecting...")
        self.engine.disconnect()
        if self.protocol:
            self.protocol._pump() # Flush DISCONNECT packet
            if self.protocol.transport:
                self.protocol.transport.close()
                self.protocol.closed = True

    def _on_event(self, ev):
        if ev.is_connected():
            if not self._connect_future.done():
                self._connect_future.set_result(True)
        
        elif ev.is_published():
            # Packet Acked
            res = ev[0]
            pid = res.packet_id
            if pid in self._pending_publish:
                fut = self._pending_publish.pop(pid)
                if not fut.done():
                    fut.set_result(res)
                print(f"Client: Future resolved for Publish PID {pid}")

        elif ev.is_subscribed():
            # Sub Acked
            res = ev[0]
            pid = res.packet_id
            if pid in self._pending_subscribe:
                fut = self._pending_subscribe.pop(pid)
                if not fut.done():
                    fut.set_result(res)
                print(f"Client: Future resolved for Subscribe PID {pid}")

        elif ev.is_message_received():
            msg = ev[0]
            print(f"******** MSG RECEIVED ********")
            print(f"Topic: {msg.topic}")
            print(f"Payload: {msg.payload.decode(errors='replace')}")
            print(f"******************************")
            
        elif ev.is_disconnected():
            pass

async def main():
    client_id = f"python_proper_{int(time.time() % 10000)}"
    client = FlowMqttClient(client_id)
    
    try:
        await client.connect("broker.emqx.io", 1883)
        
        # Subscribe and wait for ack
        await client.subscribe("test/python/proper", 1)
        print("✅ Subscribe completed!")
        
        # Publish and wait for ack
        await client.publish("test/python/proper", b"Hello from Proper Client!", 1)
        print("✅ Publish completed!")
        
        # Wait a bit to receive the message back
        await asyncio.sleep(2)
        
    finally:
        await client.disconnect()

if __name__ == "__main__":
    asyncio.run(main())
