"""Publish MQTT 5 properties over TCP, TLS, or QUIC to a configured broker."""

import argparse
import asyncio
import os

from flowsdk import ConnectProperties, FlowMqttClient, PublishProperties, Subscription, TransportType, Will


async def run(args):
    client = FlowMqttClient(
        args.client_id, transport=TransportType(args.transport), server_name=args.host,
        username=os.environ.get("FLOWSDK_BROKER_USERNAME"),
        password=os.environ.get("FLOWSDK_BROKER_PASSWORD"), ca_cert_file=args.ca,
        clean_start=False, connect_properties=ConnectProperties(session_expiry_interval=60),
        will=Will(args.topic + "/status", b"offline", qos=1),
        on_message_full=lambda message: print(message.topic, message.payload, message.properties),
    )
    try:
        connection = await client.connect(args.host, args.port, return_result=True)
        print("Session resumed:", connection.session_present)
        subscription = await client.subscribe_many([Subscription(args.topic, qos=1)])
        subscription.raise_for_status()
        result = await client.publish(
            args.topic, b'{"value":23.5}', qos=1, retain=True, return_result=True,
            properties=PublishProperties(content_type="application/json", message_expiry_interval=60,
                correlation_data=b"request-42", user_properties=[("source", "sensor"), ("source", "gateway")]),
        )
        result.raise_for_status()
        print("Published packet:", result.packet_id)
        await client.ping()
    finally:
        await client.disconnect()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="localhost")
    parser.add_argument("--port", type=int, default=1883)
    parser.add_argument("--transport", choices=["tcp", "tls", "quic"], default="tcp")
    parser.add_argument("--ca", help="CA certificate for a private TLS/QUIC broker")
    parser.add_argument("--client-id", default="flowsdk-python-properties")
    parser.add_argument("--topic", default="flowsdk/examples/properties")
    asyncio.run(run(parser.parse_args()))
