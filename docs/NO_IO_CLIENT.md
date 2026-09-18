# Rust sans-I/O MQTT client

`NoIoMqttClient` handles MQTT 3.1.1 (protocol selection 3 or 4) and MQTT 5
(selection 5). It owns protocol/session state and performs no socket I/O.
`TlsMqttEngine` adds rustls encryption; `QuicMqttEngine` adds QUIC and stream routing.
All use the shared MQTT engine. State is in memory, not persisted across process restarts.

To build the protocol core without optional runtimes or transports while retaining
strict packet validation:

```sh
cargo build --no-default-features --features strict-protocol-compliance
```

## Driving the engine

1. Establish your transport and call `connect()?`.
2. Feed received bytes to `handle_incoming()` and process **its returned events**.
3. Call `handle_tick(now)` at `next_tick_at()` and process its returned events.
4. Drain `take_outgoing()`. Keep an offset into the returned bytes until every byte
   is written; `WouldBlock` does not transfer ownership back to the engine.
5. Drain `take_events()` after commands/output draining. `handle_incoming()` and
   `handle_tick()` already drain their events; do not discard their return values.

Output draining also progresses buffered protocol responses and queued publications.
If application-event capacity pauses parsing, consume the events and call
`handle_incoming(&[])` to process buffered input without waiting for another read.

EOF is a lost connection. Discard the transport's unwritten tail and call
`handle_connection_lost()`. The engine discards old wire bytes, partial input and
connection-scoped aliases, while retaining accepted queued publications and eligible
QoS state. With `reconnect(true)`, it schedules a backoff and emits `ReconnectNeeded`
when the deadline expires. Create a fresh transport, call `reset_for_new_transport()`,
then `connect()?`. Explicit `schedule_reconnect(now)` is idempotent while a retry is
pending. Intentional `disconnect()?` cancels retries; flush its bytes before closing.

A successful CONNACK with Session Present resumes QoS state subject to the new
broker limits. Session Present without locally established session state is rejected
with an error and `Disconnected`; the transport owner must close the connection.
Without a resumed session, outstanding operations emit
`OperationFailed`; the application decides whether to publish again. Only the
configured `subscription_topics` are automatically restored. Runtime subscriptions
are not stored as a durable subscription registry.

## Configuration and delivery

Dedicated MQTT 5 CONNECT options are merged with `connect_properties`. Equal
singleton values are deduplicated; conflicts, invalid values and wrong-context
properties return errors. User properties may repeat. `receive_maximum` limits
outgoing QoS publications locally; `incoming_receive_maximum` advertises the receive
limit to the broker. The engine honors negotiated keep-alive, packet-size, QoS,
retain and subscription capabilities. Topic aliases are scoped to the connection;
resumed publications carry their full topic names.

On an active connection, a QoS 2 PUBLISH holds its send quota until PUBCOMP.
After session resumption, pending PUBREL packets are replayed with their original
identifiers ahead of PUBLISH replay and do not consume the new connection's
PUBLISH quota. Transport buffer limits still apply to replay output.

With `auto_ack(false)`, use `puback(id, reason, properties)`,
`pubrec(id, reason, properties)` and `pubcomp(id, reason, properties)` after accepting
or persisting a message. Wait for `PubRelReceived` before PUBCOMP. Failed ACK commands
leave the receive state available for retry. MQTT 3 acknowledgments require reason
zero and no properties. Incoming QoS 2 duplicates are suppressed until the exchange
completes; this does not make external application side effects transactional.

`OperationTimeouts::default()` disables operation deadlines. Opt in with
`.operation_timeouts(OperationTimeouts::cloud())` for 30-second CONNECT and
10-second publish/SUBSCRIBE/UNSUBSCRIBE deadlines, or supply individual durations.
Deadlines begin when CONNECT is queued or the operation enters outgoing protocol
processing, not when the socket reports a completed write. A timeout emits one
`OperationFailed` event, but an ACK timeout does not cancel the MQTT exchange or
release its packet ID. A late valid acknowledgment can still complete it. MQTT 5
publications are not retransmitted on the same live connection.

Packet-count limits still apply. Optional `max_incoming_packet_size`,
`max_incoming_buffer_bytes`, and `max_outgoing_buffer_bytes` add byte limits;
`parser_buffer_size` is only initial allocation. The examples select 1 MiB incoming
packet/buffer limits and 8 MiB outgoing capacity. Limits should reflect your payloads
and transport. Publish admission reserves space for protocol responses, CONNECT and
the largest outstanding publication, so queued data cannot block session recovery.
The publication reserve is released once all tracked operations complete.
Input that cannot be retained within the configured limit terminates
that connection with an error. Applications must also bound their own extracted
wire buffers and event queues.

## Breaking API migration

- `MqttEngine` and `NoIoMqttClient` CONNECT, PING, AUTH and DISCONNECT commands return
  `Result`; use `?` or explicitly handle errors. TLS/QUIC delegation propagates them.
- Use `disconnect_with(reason, properties)` on `NoIoMqttClient` for MQTT 5 properties;
  existing engine `try_*` methods remain checked aliases.
- Handle `MqttEvent::OperationFailed { operation, packet_id, error }`. An operation
  timeout means acknowledgment is unknown, not that the broker rejected the message.
- AUTH requires the CONNECT authentication method and a client AUTH reason (0x18
  or 0x19). A broker-sent successful AUTH uses reason zero.
- `NoIoMqttClient` is `Send + Sync`; mutations still require exclusive access.

## Examples

For local plain TCP:

```sh
cargo run --example no_io_mqtt_client_example -- localhost:1883
```

For TLS, set `MQTT_PEER=host:8883` and `MQTT_SERVER_NAME=host`, then:

```sh
cargo run --features rustls-tls --example no_io_tls_client_example
```

Optional `MQTT_CA_FILE` supplies a CA bundle. Set both `MQTT_CERT_FILE` and
`MQTT_KEY_FILE` for mutual TLS. `MQTT_CLIENT_ID`, `MQTT_USERNAME`, and `MQTT_PASSWORD`
configure broker identity without embedding credentials in source. Certificates
and hostnames are verified. The examples exchange data for 30 seconds and allow
five seconds to flush a graceful disconnect. Their polling loop can be replaced
by a reactor using socket readiness and engine deadlines.
