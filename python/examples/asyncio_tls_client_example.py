"""
Example demonstrating the high-level FlowMqttClient for asyncio applications.

This example shows how to use the FlowMqttClient class from the flowsdk package
to easily connect, subscribe, publish, and disconnect from an MQTT broker using
async/await syntax.
"""
import asyncio
import time
import os
import sys

# Add the package to path
sys.path.append(os.path.join( "python", "package"))

from flowsdk import FlowMqttClient, TransportType

async def test_tls():
    print("🚀 Testing TLS Transport Support...")
    
    msg_received = asyncio.Event()

    def on_message(topic: str, payload: bytes, qos: int):
        print(f"📨 Received message on {topic}: {payload.decode()}")
        if topic == "test/python/tls" and payload == b"Hello TLS!":
            msg_received.set()

    # Check if TLS key logging is requested via environment variable
    keylog_file = os.environ.get("SSLKEYLOGFILE")
    enable_key_log = keylog_file is not None
    if enable_key_log:
        print(f"🔑 TLS key logging enabled → {keylog_file}")

    # We'll use broker.emqx.io:8883 which supports TLS
    client = FlowMqttClient(
        client_id=f"python_tls_test_{int(time.time() % 10000)}",
        transport=TransportType.TLS,
        insecure_skip_verify=False,  # Skip verification for simplicity in test
        server_name="broker.emqx.io",
        enable_key_log=enable_key_log,
        on_message=on_message
    )
    
    try:
        host = "broker.emqx.io"
        port = 8883
        print(f"📡 Connecting to {host}:{port} (TLS)...")
        await client.connect(host, port)
        print("✅ TLS Connected!")
        
        await client.subscribe("test/python/tls", 1)
        print("✅ Subscribed!")
        
        await client.publish("test/python/tls", b"Hello TLS!", 1)
        print("✅ Published!")
        
        # Wait for the message with a timeout
        print("⏳ Waiting for message reception...")
        try:
            await asyncio.wait_for(msg_received.wait(), timeout=5.0)
            print("✨ Message verification successful!")
        except asyncio.TimeoutError:
            print("❌ Timeout waiting for message!")
        
    except Exception as e:
        print(f"❌ TLS Test Failed: {e}")
        import traceback
        traceback.print_exc()
    finally:
        print("👋 Disconnecting...")
        await client.disconnect()
        print("✅ Disconnected!")

if __name__ == "__main__":
    asyncio.run(test_tls())
