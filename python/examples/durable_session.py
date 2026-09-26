"""Planned restart example; run with host, port and a local SQLite path.

Stopping abruptly may deliver the Will. This example deliberately leaves broker
and local sessions present. Credentials/Will belong in the new client options.
"""
import asyncio
import sys
from flowsdk import FlowMqttClient, RuntimeOptions, ConnectProperties, SqliteSessionStore


async def main(host, port, path):
    host = host.lower()
    authority = f"[{host}]:{port}" if ":" in host else f"{host}:{port}"
    peer = f"tcp://{authority}"
    key = f"{peer}/durable-example"  # Include account identity if applicable.
    with SqliteSessionStore(path) as store:
        saved = store.resume(key)  # Storage/decoding errors are never "missing".
        client = FlowMqttClient("durable-example", clean_start=False,
            connect_properties=ConnectProperties(session_expiry_interval=3600),
            runtime_options=RuntimeOptions(peer=peer), session_state=saved)
        await client.connect(host, port)
        await client.publish("durable/example", b"before restart", qos=1)
        checkpoint = await client.checkpoint_for_restart()
        if saved is None:
            store.create(key, checkpoint)
        else:
            store.update(key, checkpoint)
        # Only a successfully committed checkpoint permits starting a replacement.
        # store.delete(key) explicitly removes local state; it does not clear the broker.


if __name__ == "__main__":
    asyncio.run(main(sys.argv[1], int(sys.argv[2]), sys.argv[3]))
