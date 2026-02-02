"""
Simple example showing minimal FlowMqttClient usage.

This demonstrates the most basic async MQTT client usage pattern.
"""

import asyncio
from flowsdk import FlowMqttClient


async def main():
    # Create and connect
    client = FlowMqttClient(
        "simple_client",
        on_message=lambda topic, payload, qos: print(f"📨 {topic}: {payload}")
    )
    
    await client.connect("broker.emqx.io", 1883)
    await client.subscribe("test/#", 1)
    await client.publish("test/hello", b"Hello World!", 1)
    
    # Keep running to receive messages
    await asyncio.sleep(5)
    await client.disconnect()


if __name__ == "__main__":
    asyncio.run(main())
