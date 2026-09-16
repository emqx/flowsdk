"""Opt-in MQTT 5 broker workflows; see python/examples/README.md for setup."""

import asyncio
import os
import unittest
import uuid

HOST = os.environ.get("FLOWSDK_BROKER_HOST")
if HOST:
    import flowsdk


@unittest.skipUnless(HOST, "Set FLOWSDK_BROKER_HOST to run live-broker tests")
class BrokerTests(unittest.IsolatedAsyncioTestCase):
    def transports(self):
        return os.environ.get("FLOWSDK_BROKER_TRANSPORTS", "tcp").split(",")

    def client(self, transport, **options):
        defaults = dict(
            client_id="flowsdk-python-test-" + uuid.uuid4().hex,
            transport=flowsdk.TransportType(transport), server_name=HOST,
            username=os.environ.get("FLOWSDK_BROKER_USERNAME"),
            password=os.environ.get("FLOWSDK_BROKER_PASSWORD"),
            ca_cert_file=os.environ.get("FLOWSDK_BROKER_CA"),
            client_cert_file=os.environ.get("FLOWSDK_BROKER_CERT"),
            client_key_file=os.environ.get("FLOWSDK_BROKER_KEY"),
        )
        defaults.update(options)
        return flowsdk.FlowMqttClient(**defaults)

    async def connect(self, client):
        transport = client.transport_type.value
        default_port = {"tcp": 1883, "tls": 8883, "quic": 14567}[transport]
        port = int(os.environ.get("FLOWSDK_BROKER_" + transport.upper() + "_PORT", default_port))
        return await client.connect(HOST, port, server_name=HOST, return_result=True)

    async def test_publish_properties_retain_and_batch_subscription(self):
        for transport in self.transports():
            with self.subTest(transport=transport):
                messages = asyncio.Queue()
                topic = "flowsdk-python-test/" + uuid.uuid4().hex
                client = self.client(transport, on_message_full=messages.put_nowait)
                try:
                    await self.connect(client)
                    result = await client.subscribe_many([
                        flowsdk.Subscription(topic, qos=2, retain_as_published=True),
                        flowsdk.Subscription(topic + "/unused", qos=1),
                    ])
                    result.raise_for_status()
                    properties = flowsdk.PublishProperties(content_type="application/octet-stream",
                        correlation_data=b"\x00\xff", user_properties=[("source", "one"), ("source", "two")])
                    for qos in (0, 1, 2):
                        result = await client.publish(topic, b"payload", qos=qos, retain=True,
                            priority=42, properties=properties, return_result=True)
                        result.raise_for_status()
                        message = await asyncio.wait_for(messages.get(), 10)
                        self.assertEqual(message.payload, b"payload")
                        self.assertTrue(message.retain)
                        self.assertEqual(message.qos, qos)
                        for prop in properties.to_ffi():
                            self.assertIn(prop, message.properties)
                    (await client.unsubscribe_many([topic, topic + "/unused"])).raise_for_status()
                finally:
                    if client.is_connected:
                        await client.publish(topic, b"", retain=True)
                    await client.disconnect()

    async def test_persistent_session_restores_subscription(self):
        for transport in self.transports():
            with self.subTest(transport=transport):
                messages = asyncio.Queue()
                topic = "flowsdk-python-test/" + uuid.uuid4().hex
                client = self.client(transport, clean_start=False,
                    connect_properties=flowsdk.ConnectProperties(session_expiry_interval=60),
                    on_message_full=messages.put_nowait)
                try:
                    self.assertFalse((await self.connect(client)).session_present)
                    (await client.subscribe(topic, qos=1)).raise_for_status()
                    await client.disconnect()
                    self.assertTrue((await self.connect(client)).session_present)
                    await client.publish(topic, b"resumed", qos=1)
                    self.assertEqual((await asyncio.wait_for(messages.get(), 10)).payload, b"resumed")
                finally:
                    await client.disconnect(properties=[flowsdk.MqttPropertyFfi.SESSION_EXPIRY_INTERVAL(value=0)])

    async def test_will_is_delivered_after_transport_loss(self):
        for transport in self.transports():
            with self.subTest(transport=transport):
                messages = asyncio.Queue()
                topic = "flowsdk-python-test/" + uuid.uuid4().hex
                observer = self.client(transport, on_message_full=messages.put_nowait)
                subject = self.client(transport, will=flowsdk.Will(topic, b"offline", qos=1))
                try:
                    await self.connect(observer)
                    (await observer.subscribe(topic, qos=1)).raise_for_status()
                    await self.connect(subject)
                    if transport == "quic":
                        subject.quic.close_transport()
                    else:
                        subject.protocol.close()
                    self.assertEqual((await asyncio.wait_for(messages.get(), 10)).payload, b"offline")
                finally:
                    await subject.disconnect()
                    await observer.disconnect()
