
import asyncio
import flowsdk
import time

from flowsdk import FlowMqttClient, TransportType, MqttOptionsFfi, MqttEngineFfi

async def main():
    print("🚀 FlowSDK No-IO Async Example")
    print("=" * 60)

    # 1. Create the engine
    import random
    client_id = f"python_async_no_io_{random.randint(1000, 9999)}"
    opts = MqttOptionsFfi(
        client_id=client_id,
        mqtt_version=5,
        clean_start=True,
        keep_alive=60,
        username=None,
        password=None,
        reconnect_base_delay_ms=1000,
        reconnect_max_delay_ms=30000,
        max_reconnect_attempts=0,
    )
    engine = MqttEngineFfi.new_with_opts(opts)
    print(f"✅ Created MqttEngineFfi (Client ID: {client_id})")

    # 2. Establish TCP connection using asyncio
    broker_host = "broker.emqx.io"
    broker_port = 1883
    print(f"📡 Connecting to {broker_host}:{broker_port}...")
    try:
        reader, writer = await asyncio.open_connection(broker_host, broker_port)
    except Exception as e:
        print(f"❌ Connection failed: {e}")
        return

    print("✅ TCP Connected")

    # 3. Initiate MQTT connection
    engine.connect()
    
    # 4. Main orchestration loop
    try:
        # Run for 15 sconds
        start_time = time.monotonic()
        end_time = start_time + 15
        
        # Subscribe/Publish state
        subscribed = False
        published = False

        while time.monotonic() < end_time:
            now_ms = engine.elapsed_ms()
            
            # 1. Handle protocol timers
            engine.handle_tick(now_ms)
            
            # 2. Handle network input (non-blocking-ish)
            try:
                # Use a small timeout to keep the loop pumping
                data = await asyncio.wait_for(reader.read(4096), timeout=0.05)
                if data:
                    print(f"📥 Received {len(data)} bytes from network")
                    engine.handle_incoming(data)
                elif reader.at_eof():
                    print("📥 Reader got EOF")
                    engine.handle_connection_lost()
                    break
            except asyncio.TimeoutError:
                # No data available in this tick, that's fine
                pass
            except Exception as e:
                print(f"❌ Read error: {e}")
                engine.handle_connection_lost()
                break

            # 3. Handle network output
            outgoing = engine.take_outgoing()
            if outgoing:
                print(f"📤 Sending {len(outgoing)} bytes to network...")
                writer.write(outgoing)
                await writer.drain()
            
            # 4. Process all accumulated events
            events = engine.take_events()
            for event in events:
                if event.is_connected():
                    res = event[0]
                    print(f"✅ MQTT Connected! (Reason: {res.reason_code}, Session Present: {res.session_present})")
                elif event.is_disconnected():
                    print(f"💔 MQTT Disconnected! (Reason: {event.reason_code})")
                elif event.is_message_received():
                    msg = event[0]
                    print(f"📨 Message on '{msg.topic}': {msg.payload.decode()} (QoS: {msg.qos})")
                elif event.is_subscribed():
                    res = event[0]
                    print(f"✅ Subscribed (PID: {res.packet_id}, Reasons: {res.reason_codes})")
                elif event.is_published():
                    res = event[0]
                    print(f"✅ Published (PID: {res.packet_id}, Reason: {res.reason_code})")
                elif event.is_error():
                    print(f"❌ Engine Error: {event[0].message}")
                elif event.is_reconnect_needed():
                    print("🔄 Engine signaled ReconnectNeeded")
                elif event.is_reconnect_scheduled():
                    print(f"⏰ Engine scheduled reconnect (Attempt {event.attempt}, Delay {event.delay_ms}ms)")
                elif event.is_ping_response():
                    print(f"🏓 Ping Response (Success: {event.success})")

            # 5. Application logic
            if engine.is_connected():
                if not subscribed:
                    pid = engine.subscribe("test/python/no_io", 1)
                    print(f"📑 API -> subscribe('test/python/no_io', qos=1) -> PID: {pid}")
                    subscribed = True
                elif not published:
                    pid = engine.publish("test/python/no_io", b"Hello from Python No-IO!", 1, None)
                    print(f"📤 API -> publish('test/python/no_io', payload='...', qos=1) -> PID: {pid}")
                    published = True

    except Exception as e:
        print(f"❌ Loop error: {e}")
    finally:
        # 6. Disconnect gracefully
        print("\n👋 Disconnecting...")
        if not writer.is_closing():
            engine.disconnect()
            outgoing = engine.take_outgoing()
            if outgoing:
                writer.write(outgoing)
                await writer.drain()
            
            writer.close()
            await writer.wait_closed()
        print("✅ Done")

if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        pass
