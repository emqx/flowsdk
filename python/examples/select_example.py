import socket
import selectors
import time
import flowsdk_ffi
import sys

def main():
    """
    Main function demonstrating a select-based MQTT client using FlowSDK FFI.
    This example shows how to use Python's selectors module to handle non-blocking
    I/O with an MQTT engine. It connects to a public MQTT broker, subscribes to a
    topic, publishes a message, and handles all I/O events asynchronously.
    The function performs the following steps:
    1. Creates an MQTT engine with configuration options
    2. Establishes a non-blocking TCP connection to the broker
    3. Uses a selector to monitor socket events (read/write availability)
    4. Manages the MQTT connection handshake
    5. Subscribes to a test topic
    6. Publishes a test message
    7. Processes incoming MQTT messages and protocol events
    8. Runs for 30 seconds or until disconnected
    The selector's EVENT_WRITE flag indicates when the socket is ready for writing,
    which occurs when:
    - The initial non-blocking connect() completes successfully
    - The socket's send buffer has space available for outgoing data
    Returns:
        None
    Raises:
        KeyboardInterrupt: Handled gracefully to allow clean disconnection
    """
    print("🚀 Select-based FlowSDK Example (Port of Rust no_io_mqtt_client_example.rs)")
    print("=" * 70)

    # 1. Create the engine
    client_id = f"python_select_no_io_{int(time.time() % 10000)}"
    opts = flowsdk_ffi.MqttOptionsFfi(
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
    engine = flowsdk_ffi.MqttEngineFfi.new_with_opts(opts)
    print(f"✅ Created MqttEngineFfi (Client ID: {client_id})")

    # 2. Establish TCP connection
    broker_host = "broker.emqx.io"
    broker_port = 1883
    print(f"📡 Connecting to {broker_host}:{broker_port}...")
    
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.setblocking(False)
    
    try:
        sock.connect((broker_host, broker_port))
    except BlockingIOError:
        pass # Expected for non-blocking connect

    # 3. Use Selector for I/O
    sel = selectors.DefaultSelector()
    sel.register(sock, selectors.EVENT_WRITE) # Wait for connect to complete

    # Initiate MQTT connection
    engine.connect()
    
    start_time = time.monotonic()
    end_time = start_time + 30
    
    connected = False
    mqtt_connected = False
    subscribed = False
    published = False
    
    print("🔄 Entering main loop...")
    
    outgoing_data = b""

    try:
        while time.monotonic() < end_time:
            now_ms = int((time.monotonic() - start_time) * 1000)
            
            # 1. Handle engine ticks
            engine.handle_tick(now_ms)
            
            # 2. Accumulate outgoing data from engine
            new_outgoing = engine.take_outgoing()
            if new_outgoing:
                outgoing_data += new_outgoing

            # 3. Handle network events
            timeout = 0.1
            next_tick = engine.next_tick_ms()
            if next_tick > 0:
                timeout = max(0, (next_tick - now_ms) / 1000.0)
                timeout = min(timeout, 0.1)

            events = sel.select(timeout=timeout)
            
            for key, mask in events:
                if mask & selectors.EVENT_WRITE:
                    if not connected:
                        # Check if connection was successful
                        err = sock.getsockopt(socket.SOL_SOCKET, socket.SO_ERROR)
                        if err == 0:
                            print("✅ TCP Socket connected")
                            connected = True
                        else:
                            print(f"❌ TCP Connection failed: {err}")
                            return

                    if outgoing_data:
                        try:
                            sent = sock.send(outgoing_data)
                            print(f"📤 Sent {sent} bytes")
                            outgoing_data = outgoing_data[sent:]
                        except (BlockingIOError, InterruptedError):
                            pass
                
                if mask & selectors.EVENT_READ:
                    try:
                        data = sock.recv(4096)
                        if data:
                            print(f"📥 Received {len(data)} bytes")
                            engine.handle_incoming(data)
                        else:
                            print("📥 EOF from server")
                            return
                    except (BlockingIOError, InterruptedError):
                        pass

            # 4. Update selector registration based on pending data
            if not connected:
                # Still waiting for connection
                sel.modify(sock, selectors.EVENT_WRITE)
            elif outgoing_data:
                # Have data to write
                sel.modify(sock, selectors.EVENT_READ | selectors.EVENT_WRITE)
            else:
                # Only waiting for read
                sel.modify(sock, selectors.EVENT_READ)

            # 5. Process protocol events
            ffi_events = engine.take_events()
            for ev in ffi_events:
                if ev.is_connected():
                    res = ev[0]
                    print(f"✅ MQTT Connected! (Reason: {res.reason_code})")
                    mqtt_connected = True
                elif ev.is_message_received():
                    msg = ev[0]
                    content = msg.payload.decode(errors='replace')
                    print(f"📨 Message on '{msg.topic}': {content} (QoS: {msg.qos})")
                elif ev.is_subscribed():
                    print(f"✅ Subscribed (PID: {ev[0].packet_id})")
                elif ev.is_published():
                    print(f"✅ Published (PID: {ev[0].packet_id})")
                elif ev.is_error():
                    print(f"❌ Error: {ev[0].message}")
                elif ev.is_disconnected():
                    print(f"💔 Disconnected (Reason: {ev.reason_code})")
                    return

            # 6. App Logic
            if mqtt_connected:
                if not subscribed:
                    pid = engine.subscribe("test/python/select", 1)
                    print(f"📑 Subscribing (PID: {pid})...")
                    subscribed = True
                elif not published:
                    pid = engine.publish("test/python/select", b"Hello from Select!", 1, None)
                    print(f"📤 Publishing (PID: {pid})...")
                    published = True

    except KeyboardInterrupt:
        pass
    finally:
        print("\n👋 Disconnecting...")
        engine.disconnect()
        final_outgoing = engine.take_outgoing()
        if final_outgoing:
            sock.setblocking(True)
            sock.sendall(final_outgoing)
        sock.close()
        sel.close()
        print("✅ Done")

if __name__ == "__main__":
    main()
