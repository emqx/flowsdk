"""Options for the high-level asyncio client."""

from dataclasses import dataclass, field
from typing import List, Any, Optional
from . import flowsdk_ffi
from .properties import property_list


@dataclass
class Subscription:
    topic_filter: str
    qos: int = 0
    no_local: bool = False
    retain_as_published: bool = False
    retain_handling: int = 0

    def to_ffi(self):
        return flowsdk_ffi.MqttSubscriptionFfi(
            topic_filter=self.topic_filter, qos=self.qos, no_local=self.no_local,
            retain_as_published=self.retain_as_published, retain_handling=self.retain_handling,
        )


@dataclass
class Will:
    topic: str
    payload: bytes
    qos: int = 0
    retain: bool = False
    properties: List[Any] = field(default_factory=list)

    def to_ffi(self):
        return flowsdk_ffi.MqttWillFfi(topic=self.topic, payload=self.payload,
            qos=self.qos, retain=self.retain, properties=property_list(self.properties))


@dataclass
class EngineOptions:
    retransmission_timeout_ms: Optional[int] = None
    ping_timeout_multiplier: Optional[int] = None
    max_outgoing_packet_count: Optional[int] = None
    max_event_count: Optional[int] = None
    parser_buffer_size: Optional[int] = None
    max_inflight: Optional[int] = None
    auto_keepalive: Optional[bool] = None
    auto_ack: Optional[bool] = None
    sessionless: Optional[bool] = None
    subscriptions: List[Subscription] = field(default_factory=list)

    def to_ffi(self):
        return flowsdk_ffi.MqttEngineOptionsFfi(
            retransmission_timeout_ms=self.retransmission_timeout_ms,
            ping_timeout_multiplier=self.ping_timeout_multiplier,
            max_outgoing_packet_count=self.max_outgoing_packet_count,
            max_event_count=self.max_event_count, parser_buffer_size=self.parser_buffer_size,
            max_inflight=self.max_inflight, auto_keepalive=self.auto_keepalive,
            auto_ack=self.auto_ack, sessionless=self.sessionless,
            subscriptions=[s.to_ffi() if isinstance(s, Subscription) else s for s in self.subscriptions])


@dataclass
class QuicZeroRttOptions:
    session_cache_size: int = 256
    replay_on_reject: bool = True

    def to_ffi(self):
        return flowsdk_ffi.QuicZeroRttOptionsFfi(session_cache_size=self.session_cache_size,
            replay_on_reject=self.replay_on_reject)
