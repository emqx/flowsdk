"""Advanced QUIC controls exposed through FlowMqttClient.quic."""


class QuicControls:
    def __init__(self, client):
        self._client = client

    @property
    def zero_rtt_status(self):
        return self._client.engine.zero_rtt_status()

    @property
    def control_stream_id(self):
        return self._client.engine.control_stream_id()

    @property
    def data_stream_count(self):
        return self._client.engine.data_stream_count()

    def clear_session_cache(self):
        self._client.engine.clear_session_cache()

    def _call(self, method, *args):
        self._client._require_connection(early_data=True)
        result = getattr(self._client.engine, method)(*args)
        self._client.pump()
        return result

    def open_stream(self):
        return self._call("open_data_stream")

    def set_stream_priority(self, stream_id, priority):
        """Set a data stream's priority, 0..255; higher values are sent first."""
        return self._call("set_stream_priority", stream_id, priority)

    def finish_stream(self, stream_id):
        return self._call("finish_stream", stream_id)

    def reset_stream(self, stream_id, error_code=0):
        return self._call("reset_stream", stream_id, error_code)

    def stop_stream(self, stream_id, error_code=0):
        return self._call("stop_stream", stream_id, error_code)

    def ping(self):
        """Queue a QUIC transport PING; use client.ping() to await MQTT PINGRESP."""
        return self._call("quic_ping")

    def notify_local_address_changed(self):
        """Notify QUIC after an external local-address change; does not rebind UDP."""
        return self._call("notify_local_address_changed")

    async def publish(self, stream_id, topic, payload, qos=0, **options):
        return await self._client.publish(topic, payload, qos, stream_id=stream_id, **options)

    async def subscribe(self, stream_id, subscriptions, **options):
        return await self._client.subscribe_many(subscriptions, stream_id=stream_id, **options)

    async def unsubscribe(self, stream_id, topics, **options):
        return await self._client.unsubscribe_many(topics, stream_id=stream_id, **options)

    async def subscribe_on_control(self, subscriptions, *, properties=None, timeout=10.0):
        """Subscribe on the MQTT control stream; default subscribe uses a data stream."""
        from . import flowsdk_ffi
        from .options import Subscription
        from .properties import property_list
        client = self._client
        client._require_connection(early_data=True)
        options = flowsdk_ffi.MqttSubscribeOptionsFfi(
            subscriptions=[s.to_ffi() if isinstance(s, Subscription) else s for s in subscriptions],
            properties=property_list(properties))
        pid = client.engine.subscribe_on_control(options)
        return await client._wait_for_ack(client._pending_subscribe, pid, timeout)

    async def unsubscribe_on_control(self, topics, *, properties=None, timeout=10.0):
        from . import flowsdk_ffi
        from .properties import property_list
        client = self._client
        client._require_connection(early_data=True)
        options = flowsdk_ffi.MqttUnsubscribeOptionsFfi(topics=list(topics), properties=property_list(properties))
        pid = client.engine.unsubscribe_on_control(options)
        return await client._wait_for_ack(client._pending_unsubscribe, pid, timeout)

    def close_transport(self, error_code=0, reason=b"", *, silent=False):
        """Close QUIC directly, without MQTT DISCONNECT or automatic reconnection."""
        client = self._client
        client._require_connection()
        client._disconnecting = True
        client._reconnect_target = None
        try:
            if silent:
                client.engine.close_silent()
            else:
                client.engine.close_transport(error_code, reason)
                client.engine.handle_tick(client.engine.elapsed_ms())
                client.pump()
        finally:
            client._close_connection(ConnectionError("QUIC transport closed by client"))
            client._disconnecting = False
