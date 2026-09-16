"""Convenience properties; ordered native property lists are also accepted."""

from dataclasses import dataclass, field
from typing import List, Optional, Tuple

from . import flowsdk_ffi


@dataclass
class PublishProperties:
    payload_format_indicator: Optional[int] = None
    message_expiry_interval: Optional[int] = None
    content_type: Optional[str] = None
    response_topic: Optional[str] = None
    correlation_data: Optional[bytes] = None
    topic_alias: Optional[int] = None
    user_properties: List[Tuple[str, str]] = field(default_factory=list)

    def to_ffi(self):
        values = []
        for name in (
            "payload_format_indicator", "message_expiry_interval", "content_type",
            "response_topic", "correlation_data", "topic_alias",
        ):
            value = getattr(self, name)
            if value is not None:
                variant = getattr(flowsdk_ffi.MqttPropertyFfi, name.upper())
                values.append(variant(value=value))
        values.extend(flowsdk_ffi.MqttPropertyFfi.USER_PROPERTY(key=key, value=value)
                      for key, value in self.user_properties)
        return values


def property_list(properties):
    if properties is None:
        return []
    if hasattr(properties, "to_ffi"):
        return properties.to_ffi()
    return list(properties)


@dataclass
class ConnectProperties:
    session_expiry_interval: Optional[int] = None
    receive_maximum: Optional[int] = None
    maximum_packet_size: Optional[int] = None
    topic_alias_maximum: Optional[int] = None
    request_response_information: Optional[int] = None
    request_problem_information: Optional[int] = None
    authentication_method: Optional[str] = None
    authentication_data: Optional[bytes] = None
    user_properties: List[Tuple[str, str]] = field(default_factory=list)

    def to_ffi(self):
        values = []
        for name in self.__dataclass_fields__:
            value = getattr(self, name)
            if name != "user_properties" and value is not None:
                values.append(getattr(flowsdk_ffi.MqttPropertyFfi, name.upper())(value=value))
        values.extend(flowsdk_ffi.MqttPropertyFfi.USER_PROPERTY(key=key, value=value)
                      for key, value in self.user_properties)
        return values
