"""
Example demonstrating MQTT over QUIC using flowsdk-ffi with asyncio.

QUIC provides a modern UDP-based transport with built-in encryption and multiplexing.
This example shows how to use the FlowMqttClient with QUIC transport.

Note: Requires a QUIC-enabled MQTT broker (e.g., EMQX with QUIC support).

TLS Key Logging (for Wireshark):
    Set the SSLKEYLOGFILE environment variable to capture TLS secrets:
        SSLKEYLOGFILE=$PWD/sslkeylog.txt python asyncio_quic_client_example.py
    Then open the capture in Wireshark and configure the keylog file under:
        Edit > Preferences > Protocols > TLS > (Pre)-Master-Secret log filename
"""
import asyncio
import os
from flowsdk import FlowMqttClient, TransportType


def on_message(topic: str, payload: bytes, qos: int):
    """Callback for received messages."""
    print(f"📨 Received: {topic} -> {payload.decode('utf-8')} (QoS {qos})")


async def main():
    """Main example demonstrating QUIC MQTT client."""
    print("🚀 Starting QUIC MQTT Client Example...")
    
    # Check if TLS key logging is requested via environment variable
    keylog_file = os.environ.get("SSLKEYLOGFILE")
    enable_key_log = keylog_file is not None
    if enable_key_log:
        print(f"🔑 TLS key logging enabled → {keylog_file}")
    
    # Create QUIC client
    client = FlowMqttClient(
        client_id="quic_test_client",
        transport=TransportType.QUIC,
        mqtt_version=5,
        insecure_skip_verify=True,  # Skip TLS verification for demo
        enable_key_log=enable_key_log,
        on_message=on_message
    )
    
    try:
        # Connect to broker using QUIC
        host = "broker.emqx.io"
        port = 14567
        print(f"🔌 Connecting to {host}:{port} via QUIC...")
        await client.connect(host, port, server_name="broker.emqx.io")
        print("✅ Connected!")
        
        # Subscribe to a test topic
        topic = "flowsdk/quic/test"
        print(f"📡 Subscribing to {topic}...")
        await client.subscribe(topic, qos=1)
        print("✅ Subscribed!")
        
        # Publish some test messages
        for i in range(3):
            message = f"QUIC message {i+1}"
            print(f"📤 Publishing: {message}")
            await client.publish(topic, message.encode(), qos=1)
            await asyncio.sleep(0.5)
        
        # Wait for messages
        print("⏳ Waiting for messages (5 seconds)...")
        await asyncio.sleep(5)
        
        # Disconnect
        print("👋 Disconnecting...")
        await client.disconnect()
        print("✅ Disconnected!")
        
    except Exception as e:
        print(f"❌ Error: {e}")
        import traceback
        traceback.print_exc()


if __name__ == "__main__":
    asyncio.run(main())
