import asyncio

import flowsdk_ffi
import time
import socket

"""
Asynchronous MQTT Client using flowsdk-ffi with asyncio integration.
This module demonstrates how to integrate the flowsdk-ffi MQTT engine with Python's
asyncio event loop using low-level socket operations and asyncio's add_reader/add_writer
callbacks.
Example:
    Run the async interop client::
        $ python async_interop_example.py
Classes:
    AsyncInteropClient: An asyncio-integrated MQTT client that uses non-blocking sockets
                       and the asyncio event loop for I/O operations.
The implementation showcases:
    - Non-blocking TCP socket connection using asyncio.loop.sock_connect()
    - Event-driven I/O using loop.add_reader() and loop.add_writer()
    - Integration of flowsdk-ffi's tick-based engine with asyncio's event loop
    - Handling MQTT events (connection, subscription, message reception, publishing)
    - Proper resource cleanup and socket lifecycle management
Note:
    This is an example demonstrating the interoperability pattern between
    FlowSDK FFI MQTT engine and Python's asyncio framework.
"""

class AsyncInteropClient:
    def __init__(self, client_id):
        self.opts = flowsdk_ffi.MqttOptionsFfi(
            client_id=client_id,
            mqtt_version=5,
            clean_start=True,
            keep_alive=60,
            username=None,
            password=None,
            reconnect_base_delay_ms=1000,
            reconnect_max_delay_ms=30000,
            max_reconnect_attempts=0
        )
        self.engine = flowsdk_ffi.MqttEngineFfi.new_with_opts(self.opts)
        self.start_time = time.monotonic()
        self.outgoing_buffer = b""
        self.sock = None
        self.loop = asyncio.get_running_loop()
        self.connected_event = asyncio.Event()

    async def connect(self, host, port):
        print(f"📡 Connecting to {host}:{port}...")
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.sock.setblocking(False)
        
        try:
            self.sock.connect((host, port))
        except BlockingIOError:
            pass

        # Use asyncio to wait for the socket to be writable (connection complete)
        await self.loop.sock_connect(self.sock, (host, port))
        print("✅ TCP Connected")

        # Start the engine logic
        self.engine.connect()
        
        # Register reader
        self.loop.add_reader(self.sock, self._on_read)
        self._pump_logic()

    def _on_read(self):
        try:
            data = self.sock.recv(4096)
            if data:
                print(f"📥 Received {len(data)} bytes")
                self.engine.handle_incoming(data)
                self._pump_logic()
            else:
                print("📥 EOF")
                self.stop()
        except Exception as e:
            print(f"❌ Read error: {e}")
            self.stop()

    def _on_write(self):
        if not self.outgoing_buffer:
            self.loop.remove_writer(self.sock)
            return
        
        try:
            sent = self.sock.send(self.outgoing_buffer)
            print(f"📤 Sent {sent} bytes")
            self.outgoing_buffer = self.outgoing_buffer[sent:]
            if not self.outgoing_buffer:
                self.loop.remove_writer(self.sock)
        except (BlockingIOError, InterruptedError):
            pass
        except Exception as e:
            print(f"❌ Write error: {e}")
            self.stop()

    def _pump_logic(self):
        # 1. Ticks
        now_ms = int((time.monotonic() - self.start_time) * 1000)
        self.engine.handle_tick(now_ms)
        
        # 2. Outgoing
        new_data = self.engine.take_outgoing()
        if new_data:
            self.outgoing_buffer += new_data
            self.loop.add_writer(self.sock, self._on_write)

        # 3. Events
        events = self.engine.take_events()
        for ev in events:
            if ev.is_connected():
                print("✅ MQTT Connected!")
                self.connected_event.set()
            elif ev.is_message_received():
                m = ev[0]
                print(f"📨 Message: {m.topic} -> {m.payload.decode()}")
            elif ev.is_subscribed():
                print(f"✅ Subscribed (PID: {ev[0].packet_id})")
            elif ev.is_published():
                print(f"✅ Published (PID: {ev[0].packet_id})")

        # 4. Schedule next tick
        next_ms = self.engine.next_tick_ms()
        if next_ms > 0:
            delay = max(0, (next_ms - now_ms) / 1000.0)
            self.loop.call_later(delay, self._pump_logic)

    def stop(self):
        if self.sock:
            self.loop.remove_reader(self.sock)
            self.loop.remove_writer(self.sock)
            self.sock.close()
            self.sock = None
        print("🛑 Client stopped")

async def main():
    print("🚀 asyncio Interop Example (Using add_reader/add_writer)")
    print("=" * 70)
    
    client = AsyncInteropClient(f"python_async_interop_{int(time.time() % 10000)}")
    await client.connect("broker.emqx.io", 1883)
    
    await client.connected_event.wait()
    
    # Example operations
    client.engine.subscribe("test/python/interop", 1)
    client._pump_logic() # Trigger immediate pump to send subscribe
    
    client.engine.publish("test/python/interop", b"Hello from Interop!", 1, None)
    client._pump_logic() # Trigger immediate pump to send publish
    
    # Run for a bit to receive messages
    await asyncio.sleep(5)
    client.stop()

if __name__ == "__main__":
    asyncio.run(main())
