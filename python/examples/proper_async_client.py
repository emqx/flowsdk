"""
Example demonstrating the high-level FlowMqttClient for asyncio applications.

This example shows how to use the FlowMqttClient class from the flowsdk package
to easily connect, subscribe, publish, and disconnect from an MQTT broker using
async/await syntax.
"""

import asyncio
import time
from flowsdk import FlowMqttClient


def on_message(topic: str, payload: bytes, qos: int):
    """Callback for received messages."""
    print(f"******** MSG RECEIVED ********")
    print(f"Topic: {topic}")
    print(f"Payload: {payload.decode(errors='replace')}")
    print(f"QoS: {qos}")
    print(f"******************************")


async def main():
    # Create client with message callback
    client_id = f"python_proper_{int(time.time() % 10000)}"
    client = FlowMqttClient(client_id, on_message=on_message)
    
    try:
        # Connect to broker
        print(f"Connecting to broker.emqx.io:1883...")
        await client.connect("broker.emqx.io", 1883)
        print("✅ Connected!")
        
        # Subscribe and wait for ack
        print("Subscribing to test/python/proper...")
        await client.subscribe("test/python/proper", 1)
        print("✅ Subscribe completed!")
        
        # Publish and wait for ack
        print("Publishing message...")
        await client.publish("test/python/proper", b"Hello from Proper Client!", 1)
        print("✅ Publish completed!")
        
        # Wait a bit to receive the message back
        print("Waiting for messages...")
        await asyncio.sleep(2)
        
    finally:
        print("Disconnecting...")
        await client.disconnect()
        print("✅ Disconnected!")


if __name__ == "__main__":
    asyncio.run(main())


if __name__ == "__main__":
    asyncio.run(main())
