# Protocol Compliance Features

This document describes the protocol compliance features implemented in the MQTT library with configurable strict validation.

## Feature Flag: `strict-protocol-compliance`

The library includes a feature flag `strict-protocol-compliance` that enables additional MQTT 5.0 protocol compliance validations. This feature is **enabled by default** but can be disabled if needed.

### Enabling/Disabling the Feature

```toml
# In Cargo.toml - enable by default (current behavior)
[features]
default = ["strict-protocol-compliance"]
strict-protocol-compliance = []

# To disable strict compliance
cargo build --no-default-features
cargo test --no-default-features
```

## Implemented Validations

### General Validations

#### UTF-8 String Validation
When `strict-protocol-compliance` is enabled, all UTF-8 strings are validated to ensure they comply with MQTT 5.0 specification:

- **Null characters (U+0000)** are rejected
- **UTF-16 surrogate pairs (U+D800-U+DFFF)** are rejected  
- **BOM (Byte Order Mark, U+FEFF)** at the beginning of strings is rejected

**Error Messages:**
- `"UTF-8 string contains null character (U+0000)"`
- `"UTF-8 string contains surrogate character (U+XXXX)"`
- `"UTF-8 string starts with BOM (U+FEFF)"`

#### Variable Byte Integer (VBI) Validation
Non-minimal VBI encodings are detected and rejected. A VBI is considered non-minimal if it uses more bytes than necessary to represent a value.

**Error Message:**
- `"Variable Byte Integer encoding is not minimal"`

### CONNECT Packet Validation

#### Reserved Flag Validation
The reserved flag in Connect Flags must be 0.

**Error Message:**
- `"CONNECT packet reserved flag is not 0"`

#### Will Flag Validation
When Will Flag is not set (0):
- Will QoS must be 0
- Will Retain must be 0

When Will Flag is set:
- Will QoS cannot be 3 (invalid/malformed packet)

**Error Messages:**
- `"Will QoS must be 0 if Will Flag is 0"`
- `"Will Retain must be 0 if Will Flag is 0"`
- `"Will QoS cannot be 3"`

### CONNACK Packet Validation

#### Reserved Bits Validation
The Connect Acknowledge Flags byte must have all reserved bits set to 0.

**Error Message:**
- `"CONNACK Connect Acknowledge Flags reserved bits must be 0"`

### PUBLISH Packet Validation

#### QoS Validation
- QoS level 3 (`11` in binary) is invalid and rejected
- DUP flag must be 0 for QoS 0 messages

**Error Messages:**
- `"PUBLISH QoS bits must not be set to 11 (invalid QoS 3)"`
- `"PUBLISH DUP flag must be 0 for QoS 0 messages"`

#### Topic Name Validation
Topic names in PUBLISH packets must not contain wildcard characters (`#` or `+`).

**Error Message:**
- `"PUBLISH Topic Name must not contain wildcard characters (# or +)"`

### SUBSCRIBE Packet Validation

#### Fixed Header Flags
SUBSCRIBE packets must have fixed header flags set to `0010` (bit 1 = 1, others = 0).

**Error Message:**
- `"SUBSCRIBE fixed header flags must be 0010"`

#### Packet Identifier Validation
Packet identifier must be greater than 0.

**Error Message:**
- `"SUBSCRIBE packet_id must be > 0"`

#### Payload Validation
The payload must contain at least one subscription.

**Error Message:**
- `"SUBSCRIBE payload must contain at least one subscription"`

#### Subscription Options Validation
Reserved bits in Subscription Options must be 0.

**Error Message:**
- `"SUBSCRIBE Subscription Options reserved bits must be 0"`

#### No Local Flag Validation
The No Local flag cannot be set on shared subscriptions (topics starting with `$share/`).

**Error Message:**
- `"SUBSCRIBE No Local flag must not be set on Shared Subscriptions"`

#### Topic Filter Validation
Comprehensive validation of topic filter syntax including:

- Empty topic filters are rejected
- Multi-level wildcards (`#`) must be the only character in the level and the last level
- Single-level wildcards (`+`) must be the only character in the level
- Maximum length validation (65535 bytes)

**Error Messages:**
- `"Topic filter cannot be empty"`
- `"Multi-level wildcard (#) must be the only character in topic level"`
- `"Multi-level wildcard (#) must be the last level in topic filter"`
- `"Single-level wildcard (+) must be the only character in topic level"`

#### Shared Subscription Validation
Shared subscriptions must follow the format `$share/ShareName/TopicFilter`:

- ShareName cannot be empty
- TopicFilter cannot be empty
- The TopicFilter part is validated according to normal topic filter rules

**Error Messages:**
- `"Invalid shared subscription format: must be $share/ShareName/TopicFilter"`
- `"Shared subscription ShareName cannot be empty"`
- `"Shared subscription TopicFilter cannot be empty"`

### SUBACK Packet Validation

#### Packet Identifier Validation
Packet identifier must be greater than 0.

**Error Message:**
- `"SUBACK packet_id must be > 0"`

#### Payload Validation
The payload must contain at least one reason code.

**Error Message:**
- `"SUBACK payload must contain at least one reason code"`

### UNSUBSCRIBE Packet Validation

#### Fixed Header Flags
UNSUBSCRIBE packets must have fixed header flags set to `0010` (bit 1 = 1, others = 0).

**Error Message:**
- `"UNSUBSCRIBE fixed header flags must be 0010"`

#### Packet Identifier Validation
Packet identifier must be greater than 0.

**Error Message:**
- `"UNSUBSCRIBE packet_id must be > 0"`

#### Payload Validation
The payload must contain at least one topic filter.

**Error Message:**
- `"UNSUBSCRIBE payload must contain at least one topic filter"`

### UNSUBACK Packet Validation

#### Packet Identifier Validation
Packet identifier must be greater than 0.

**Error Message:**
- `"UNSUBACK packet_id must be > 0"`

#### Payload Validation
The payload must contain at least one reason code.

**Error Message:**
- `"UNSUBACK payload must contain at least one reason code"`

### PUBREL Packet Validation

#### Fixed Header Flags
PUBREL packets must have fixed header flags set to `0010` (bit 1 = 1, others = 0).

**Error Message:**
- `"Invalid PUBREL flags: expected 0x02, got 0x{flags:02x}"`

### PINGREQ Packet Validation

#### Remaining Length Validation
PINGREQ packets must have remaining length of 0.

**Error Message:**
- `"PINGREQ packet must have remaining length of 0"`

#### Fixed Header Flags
PINGREQ packets must have fixed header flags set to `0000` (all reserved bits = 0).

**Error Message:**
- `"PINGREQ packet has invalid fixed header flags"`

### PINGRESP Packet Validation

#### Remaining Length Validation
PINGRESP packets must have remaining length of 0.

**Error Message:**
- `"PINGRESP packet must have remaining length of 0"`

#### Fixed Header Flags
PINGRESP packets must have fixed header flags set to `0000` (all reserved bits = 0).

**Error Message:**
- `"PINGRESP packet has invalid fixed header flags"`

### DISCONNECT Packet Validation

#### Fixed Header Flags
DISCONNECT packets must have fixed header flags set to `0000` (all reserved bits = 0).

**Error Message:**
- `"DISCONNECT packet has invalid fixed header flags"`

### AUTH Packet Validation

#### Fixed Header Flags
AUTH packets must have fixed header flags set to `0000` (all reserved bits = 0).

**Error Message:**
- `"AUTH packet has invalid fixed header flags"`

#### Reason Code Requirement
AUTH packets with remaining length > 0 must contain a reason code.

**Error Message:**
- `"AUTH packet must contain a reason code"`


## Unimplemented Features

### MQTTv5: 4.10: Request/Response

Do request/response in pub/sub system is error-prone, use http based request/response instead.


## Testing

The library includes comprehensive tests for all validation features:

```bash
# Run all tests with strict compliance (default)
cargo test

# Run all tests without strict compliance  
cargo test --no-default-features

# Run only protocol compliance tests
cargo test protocol_compliance_tests
```

## Backward Compatibility

When `strict-protocol-compliance` is **disabled**, the library maintains backward compatibility and only performs basic validations that were already implemented. This ensures that existing code continues to work without changes.

When `strict-protocol-compliance` is **enabled** (default), the library provides maximum compliance with the MQTT 5.0 specification, which may reject some packets that were previously accepted.

## Examples

```rust
use flowsdk::mqtt_serde::encode_utf8_string;

// With strict-protocol-compliance enabled (default):
let result = encode_utf8_string("hello\u{0000}world");
assert!(result.is_err()); // Rejected due to null character

// With strict-protocol-compliance disabled:
// The same string would be accepted (only length validation)
```

```rust
use flowsdk::mqtt_serde::mqttv5::subscribe::TopicSubscription;

// With strict-protocol-compliance enabled:
let result = TopicSubscription::from_bytes(b"$share/group/topic\x80"); // Invalid subscription options
assert!(result.is_err()); // Rejected due to reserved bits

// With strict-protocol-compliance disabled:
// Reserved bits validation is not performed
```
