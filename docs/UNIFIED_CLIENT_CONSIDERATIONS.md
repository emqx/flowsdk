# Unified Client Design Considerations

This document outlines design considerations and decisions made when implementing unified TCP/QUIC transport support in the Python async client.

## 1. Tick Timing Strategy

### Issue
TCP and QUIC transports use different tick scheduling strategies:

- **TCP**: Adaptive scheduling using `engine.next_tick_ms()` to calculate optimal delay
- **QUIC**: Fixed 10ms tick interval for responsiveness

### Decision
**Use transport-specific defaults** - Each protocol implements its own tick scheduling strategy that's optimized for the transport characteristics.

**Rationale:**
- QUIC requires more frequent ticking for connection handshake and keep-alive
- TCP can optimize battery/CPU usage with adaptive scheduling
- Users shouldn't need to tune timing parameters manually
- Future transports can implement their own optimal strategies

**Implementation:**
```python
# FlowMqttProtocol (TCP)
def _on_timer(self):
    # Use next_tick_ms() for adaptive scheduling
    next_tick_ms = self.engine.next_tick_ms()
    delay = max(0, (next_tick_ms - now_ms) / 1000.0)

# FlowMqttDatagramProtocol (QUIC)  
def _on_timer(self):
    # Fixed 10ms for QUIC responsiveness
    delay = 0.01
```

**Future Enhancement:**
Could add optional `tick_interval_ms` parameter to constructor for advanced users who want to tune performance.

---

## 2. Missing QUIC APIs

### Issue
`QuicMqttEngineFfi` doesn't implement all methods available in `MqttEngineFfi`:

**Missing Methods:**
- `handle_connection_lost()` - Notifies engine of connection loss
- `next_tick_ms()` - Returns optimal next tick time

### Current Approach
**Document limitations and provide fallback behavior:**

1. **handle_connection_lost()**: Not called for QUIC since UDP is connectionless
   - QUIC engine detects connection loss through timeout internally
   - No action needed in protocol's `connection_lost()`

2. **next_tick_ms()**: Not available for QUIC
   - Use fixed 10ms interval (see Tick Timing Strategy above)
   - QUIC's `handle_tick()` returns events directly without needing separate timing

### Rationale
These differences reflect fundamental protocol differences:
- UDP is connectionless, so "connection lost" is detected differently
- QUIC's internal state machine handles timing optimization

### Future Enhancement
Could add these methods to the Rust FFI if adaptive QUIC timing proves beneficial, but current fixed approach works well.

---

## 3. TLS Transport Support
 
 ### Current State
The unified client architecture now fully supports `TransportType.TLS` for explicit TLS over TCP transport. This allows for fine-grained control over TLS configuration, including certificate validation and SNI.
 
 ### FFI Support
`TlsMqttEngineFfi` is used to handle the MQTT protocol over an encrypted TCP stream. It follows a similar API to the standard TCP engine but requires explicit calls to `handle_socket_data` and `take_socket_data` for BIOS-like data pumping.

### Implementation Details

**Key Features:**
- **Unified Adapter**: Transparently handle different method names across engines (`handle_incoming` vs `handle_socket_data`).
- **SNI Support**: Uses `server_name` parameter for TLS Server Name Indication.
- **Certificate Validation**: Supports standard CA files, client certificates, and insecure mode (skip verify).

**Usage Example:**
```python
from flowsdk import FlowMqttClient, TransportType

client = FlowMqttClient(
    "my_client",
    transport=TransportType.TLS,
    server_name="broker.emqx.io",
    ca_cert_file="emqxsl-ca.crt"
)
await client.connect("broker.emqx.io", 8883)
```

### Decision
**Implement explicit TLS transport support** - To provide full flexibility for production environments where custom CA bundles or client certificates are required.

**Rationale:**
- Many industrial environments use internal PKI.
- Client certificate authentication is a common requirement for high-security IoT.
- Provides a consistent API across TCP, TLS, and QUIC.

---

## Summary of Decisions

| Consideration | Decision | Status |
|--------------|----------|--------|
| Tick Timing | Transport-specific defaults | ✅ Implemented |
| Missing QUIC APIs | Document limitations, use fallbacks | ✅ Implemented |
| TLS Transport | Explicit unified support | ✅ Implemented |

---

## Related Documentation

- [Python Async Client API](../python/package/README.md)
- [MQTT Session Management](MQTT_SESSION.md)
- [Async Client Architecture](ASYNC_CLIENT.md)

---

*Last Updated: February 3, 2026*
