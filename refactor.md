# TODO



### `src/mqtt_serde/mqttv5/connect.rs`

**Suggestions:**
3.  **Payload Parsing in `from_bytes`:**
    *   The `will_topic`, `will_message`, `username`, and `password` fields are currently `None` in the parsed `MqttConnect` struct, even if they are present in the payload. The `from_bytes` function needs to *parse* these fields from the `buffer` based on the `connect_flags` and populate them in the `MqttConnect` struct. This is a significant missing piece of the parser.


### `src/mqtt_serde/parser/stream.rs`

**Suggestions:**
1.  **Initial Capacity:** The `BytesMut::with_capacity(4096)` is a reasonable starting point, but consider if this should be configurable or dynamically adjusted based on typical packet sizes to optimize memory usage.
2.  **Error Handling:** Ensure that `MqttPacket::from_bytes` (and its underlying parsing functions) return precise `ParseError` variants, so `next_packet` can potentially distinguish between recoverable (e.g., `IncompletePacket`) and unrecoverable (e.g., `InvalidPacketFormat`) errors. The current implementation treats all `Err` as unrecoverable, which is generally fine for a stream parser.

### `src/mqtt_serde/parser/test.rs`

**Review:**
*   Contains tests for `parse_remaining_length`.
*   Covers various valid and invalid remaining length encodings.

**Suggestions:**
1.  **Comprehensive Testing:**
    *   **`parse_utf8_string`:** Add tests for valid UTF-8 strings, empty strings, strings with non-ASCII characters, and malformed UTF-8.
    *   **`parse_topic_name`:** Test valid topic names, empty topic names, topic names with invalid UTF-8, and topic names that are too long.
    *   **`vbi`:** While `parse_remaining_length` tests `vbi`, ensure direct tests for `vbi` cover all edge cases, especially the 4-byte limit and overflow.
    *   **`hdr_len`:** Test with buffers shorter than 2 bytes.
    *   **`MqttPacket::from_bytes`:** Add tests for malformed fixed headers, unknown packet types, and incomplete packets.
    *   **Property Parsing:** Ensure `parse_property` and `parse_properties` have comprehensive tests for all property types, including malformed data and incomplete data.

Overall, the `mqtt_serde` module has a good foundation for MQTT packet serialization and deserialization. The primary areas for improvement are:
*   **Robust Error Handling:** Consistently using `Result` and specific `ParseError` variants instead of `assert!` and `unwrap()`.
*   **Completeness of Parsers:** Fully implementing the parsing logic for all fields, especially the payload of `MqttConnect` and the properties in `MqttPublish`.
*   **Test Coverage:** Expanding unit tests to cover all edge cases and error conditions.
