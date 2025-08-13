
# Type Mappings

## UTF-8 Strings:
MQTT uses UTF-8 strings (e.g., topic, client_id); mapped to string in Protocol Buffers.

## Integers:
MQTT uses 16-bit unsigned integers for message IDs and keep-alive; 
mapped to uint32 for simplicity (since uint16 isn’t a native ProtoBuf type).

## Binary Data: 
MQTT passwords and will payloads are binary; mapped to bytes.

## Booleans: 
MQTT flags (e.g., clean_session, retain, dup) are single-bit; mapped to bool.
Payload for PUBLISH: include a sample PublishPayload with device_id (string), timestamp (int64), and value (double). If your PUBLISH payload has a different structure, please specify.

## Enums: 
QoS levels are defined as an enum to ensure type safety.
Repeated Fields: Used for SUBSCRIBE’s topic subscriptions and SUBACK’s return codes to match MQTT’s list-based structures.

## MQTTv5 Properties

The `Properties` message in `mqttv5.proto` is a comprehensive collection of all possible properties defined in the MQTTv5 specification. This single message is used across different packet types (like `CONNECT`, `CONNACK`, `PUBLISH`, etc.) for simplicity.

**Important Note:** Not all properties are valid for every packet type. The application logic on both the client and server is responsible for validating and using only the properties that are appropriate for a given packet, as defined by the MQTTv5 standard.

This design choice was made to simplify the Protobuf schema, avoiding the need for separate property messages for each packet type (e.g., `ConnectProperties`, `ConnackProperties`). The `Properties` message was recently updated to include all standard MQTTv5 properties to ensure full feature support.
