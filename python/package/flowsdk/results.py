"""MQTT acknowledgement results returned by the asyncio client."""

from dataclasses import dataclass, field
from typing import List, Union, Optional, Any


@dataclass
class ConnectionResult:
    reason_code: int
    session_present: bool
    properties: List[Any] = field(default_factory=list)


@dataclass
class PublishResult:
    packet_id: Optional[int]
    reason_code: Optional[int]
    qos: int
    properties: List[Any] = field(default_factory=list)

    @property
    def is_success(self) -> bool:
        return self.reason_code is None or self.reason_code < 0x80

    @property
    def is_failure(self) -> bool:
        return not self.is_success

    def raise_for_status(self) -> None:
        if self.is_failure:
            raise MqttAckError(self)


@dataclass
class SubscribeResult:
    """A SUBACK result, with reason codes in subscription order."""

    packet_id: int
    reason_codes: List[int]
    properties: List[Any] = field(default_factory=list)

    @property
    def is_success(self) -> bool:
        """Whether every subscription was granted QoS 0, 1, or 2."""
        return all(code in (0, 1, 2) for code in self.reason_codes)

    @property
    def is_failure(self) -> bool:
        """Whether at least one subscription failed."""
        return not self.is_success

    def raise_for_status(self) -> None:
        """Raise MqttAckError if any subscription failed."""
        if self.is_failure:
            raise MqttAckError(self)


@dataclass
class UnsubscribeResult:
    """An UNSUBACK result; MQTT 3.1.1 has an empty reason-code list."""

    packet_id: int
    reason_codes: List[int]
    properties: List[Any] = field(default_factory=list)

    @property
    def is_success(self) -> bool:
        """Whether all topics succeeded or had no existing subscription."""
        return all(code in (0, 0x11) for code in self.reason_codes)

    @property
    def is_failure(self) -> bool:
        """Whether at least one unsubscription failed."""
        return not self.is_success

    def raise_for_status(self) -> None:
        """Raise MqttAckError if any unsubscription failed."""
        if self.is_failure:
            raise MqttAckError(self)


class MqttAckError(Exception):
    """An explicitly checked broker rejection, with its complete result."""

    def __init__(self, result: Union[SubscribeResult, UnsubscribeResult, PublishResult]):
        self.result = result
        reason_codes = [result.reason_code] if isinstance(result, PublishResult) else result.reason_codes
        codes = ", ".join(f"0x{code:02x}" for code in reason_codes if code is not None)
        super().__init__(
            f"{type(result).__name__} reports failure for packet {result.packet_id} "
            f"(reason codes: {codes})"
        )


__all__ = ["ConnectionResult", "PublishResult", "SubscribeResult", "UnsubscribeResult", "MqttAckError"]
