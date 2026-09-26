/* SPDX-License-Identifier: MPL-2.0
 * Public FlowSDK C ABI. See ../C_API.md for checked calls and JSON v1.
 * QUIC symbols require the quic build feature. TLS constructors return
 * MQTT_UNSUPPORTED when TLS is disabled. Legacy layouts and tags are stable.
 */
#ifndef FLOWSDK_H
#define FLOWSDK_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Cargo durable-session implies json. */
#if defined(FLOWSDK_DURABLE_SESSION) && !defined(FLOWSDK_JSON)
#define FLOWSDK_JSON
#endif
#ifdef __cplusplus
extern "C" {
#endif

enum { MQTT_OK = 0, MQTT_INVALID_ARGUMENT = 1, MQTT_CONFIGURATION = 2,
       MQTT_ENGINE_ERROR = 3, MQTT_UNSUPPORTED = 4 };
enum { MQTT_EVENT_OPERATION_FAILED = 19 };
typedef struct MqttEngineFFI MqttEngineFFI;
typedef struct TlsMqttEngineFFI TlsMqttEngineFFI;
typedef struct QuicMqttEngineFFI QuicMqttEngineFFI;
typedef struct MqttEventListFFI MqttEventListFFI;

typedef struct MqttOptionsC {
    const char * client_id;
    uint8_t mqtt_version;
    bool clean_start;
    uint16_t keep_alive;
    const char * username;
    const char * password;
    uint64_t reconnect_base_delay_ms;
    uint64_t reconnect_max_delay_ms;
    uint32_t max_reconnect_attempts;
} MqttOptionsC;

typedef struct MqttTlsOptionsC {
    const char * ca_cert_file;
    const char * client_cert_file;
    const char * client_key_file;
    const char * alpn;
    uint8_t insecure_skip_verify;
    uint8_t enable_key_log;
} MqttTlsOptionsC;

typedef struct MqttDatagramC {
    char * addr;
    uint8_t * data;
    size_t data_len;
} MqttDatagramC;

/* Borrow inputs only for each call. Free returned bytes/strings exactly once. */
MqttEngineFFI * mqtt_engine_new(const char * client_id, uint8_t mqtt_version);
MqttEngineFFI * mqtt_engine_new_with_opts(const MqttOptionsC * opts);
void mqtt_engine_free(MqttEngineFFI * ptr);
void mqtt_engine_connect(MqttEngineFFI * ptr);
void mqtt_engine_handle_incoming(MqttEngineFFI * ptr, const uint8_t * data, size_t len);
void mqtt_engine_handle_tick(MqttEngineFFI * ptr, uint64_t now_ms);
int64_t mqtt_engine_next_tick_ms(MqttEngineFFI * ptr);
uint8_t * mqtt_engine_take_outgoing(MqttEngineFFI * ptr, size_t * out_len);
void mqtt_engine_free_bytes(uint8_t * ptr, size_t len);
int32_t mqtt_engine_publish(MqttEngineFFI * ptr, const char * topic, const uint8_t * payload, size_t payload_len, uint8_t qos);
int32_t mqtt_engine_subscribe(MqttEngineFFI * ptr, const char * topic_filter, uint8_t qos);
int32_t mqtt_engine_unsubscribe(MqttEngineFFI * ptr, const char * topic_filter);
void mqtt_engine_disconnect(MqttEngineFFI * ptr);
int mqtt_engine_is_connected(MqttEngineFFI * ptr);
uint8_t mqtt_engine_get_version(MqttEngineFFI * ptr);
void mqtt_engine_auth(MqttEngineFFI * ptr, uint8_t reason_code);
void mqtt_engine_handle_connection_lost(MqttEngineFFI * ptr);
void mqtt_engine_free_string(char * ptr);
TlsMqttEngineFFI * mqtt_tls_engine_new(const char * client_id, uint8_t mqtt_version, const char * server_name, const MqttTlsOptionsC * tls_opts);
void mqtt_tls_engine_free(TlsMqttEngineFFI * ptr);
void mqtt_tls_engine_connect(TlsMqttEngineFFI * ptr);
void mqtt_tls_engine_handle_socket_data(TlsMqttEngineFFI * ptr, const uint8_t * data, size_t len);
uint8_t * mqtt_tls_engine_take_socket_data(TlsMqttEngineFFI * ptr, size_t * out_len);
void mqtt_tls_engine_handle_tick(TlsMqttEngineFFI * ptr, uint64_t now_ms);
int32_t mqtt_tls_engine_publish(TlsMqttEngineFFI * ptr, const char * topic, const uint8_t * payload, size_t payload_len, uint8_t qos);
int32_t mqtt_tls_engine_subscribe(TlsMqttEngineFFI * ptr, const char * topic_filter, uint8_t qos);
int32_t mqtt_tls_engine_unsubscribe(TlsMqttEngineFFI * ptr, const char * topic_filter);
void mqtt_tls_engine_disconnect(TlsMqttEngineFFI * ptr);
int32_t mqtt_tls_engine_is_connected(TlsMqttEngineFFI * ptr);
#ifdef FLOWSDK_JSON
char * mqtt_engine_take_events(MqttEngineFFI * ptr);
char * mqtt_tls_engine_take_events(TlsMqttEngineFFI * ptr);
#endif
QuicMqttEngineFFI * mqtt_quic_engine_new(const char * client_id, uint8_t mqtt_version);
void mqtt_quic_engine_free(QuicMqttEngineFFI * ptr);
int32_t mqtt_quic_engine_connect(QuicMqttEngineFFI * ptr, const char * server_addr, const char * server_name, const MqttTlsOptionsC * tls_opts);
void mqtt_quic_engine_handle_datagram(QuicMqttEngineFFI * ptr, const uint8_t * data, size_t len, const char * remote_addr);
MqttDatagramC * mqtt_quic_engine_take_outgoing_datagrams(QuicMqttEngineFFI * ptr, size_t * out_count);
void mqtt_quic_engine_free_datagrams(MqttDatagramC * ptr, size_t count);
void mqtt_quic_engine_handle_tick(QuicMqttEngineFFI * ptr, uint64_t now_ms);
#ifdef FLOWSDK_JSON
char * mqtt_quic_engine_take_events(QuicMqttEngineFFI * ptr);
#endif
int32_t mqtt_quic_engine_publish(QuicMqttEngineFFI * ptr, const char * topic, const uint8_t * payload, size_t payload_len, uint8_t qos);
int32_t mqtt_quic_engine_subscribe(QuicMqttEngineFFI * ptr, const char * topic_filter, uint8_t qos);
int32_t mqtt_quic_engine_unsubscribe(QuicMqttEngineFFI * ptr, const char * topic_filter);
void mqtt_quic_engine_disconnect(QuicMqttEngineFFI * ptr);
int32_t mqtt_quic_engine_is_connected(QuicMqttEngineFFI * ptr);
MqttEventListFFI * mqtt_engine_take_events_list(MqttEngineFFI * ptr);
void mqtt_event_list_free(MqttEventListFFI * ptr);
size_t mqtt_event_list_len(const MqttEventListFFI * ptr);
uint8_t mqtt_event_list_get_tag(const MqttEventListFFI * ptr, size_t index);
uint8_t mqtt_event_list_get_connected_rc(const MqttEventListFFI * ptr, size_t index);
char * mqtt_event_list_get_message_topic(const MqttEventListFFI * ptr, size_t index);
uint8_t * mqtt_event_list_get_message_payload(const MqttEventListFFI * ptr, size_t index, size_t * out_len);
int32_t mqtt_event_list_get_published_pid(const MqttEventListFFI * ptr, size_t index);
int32_t mqtt_event_list_get_subscribed_pid(const MqttEventListFFI * ptr, size_t index);
char * mqtt_event_list_get_error_message(const MqttEventListFFI * ptr, size_t index);
uint64_t mqtt_event_list_get_stream_id(const MqttEventListFFI * ptr, size_t index);
uint64_t mqtt_event_list_get_stream_error_code(const MqttEventListFFI * ptr, size_t index);
char * mqtt_event_list_get_stream_close_reason(const MqttEventListFFI * ptr, size_t index);
int32_t mqtt_event_list_get_stream_closed_by_peer(const MqttEventListFFI * ptr, size_t index);
MqttEventListFFI * mqtt_quic_engine_take_events_list(QuicMqttEngineFFI * ptr);
MqttEventListFFI * mqtt_tls_engine_take_events_list(TlsMqttEngineFFI * ptr);

/* Checked functions: 0 = success. Error strings are optional owned outputs.
 * Required outputs are cleared on failure. packet_id is optional (0 = no ID).
 * All nonnull pointers must be valid, correctly aligned, and live for the call;
 * outputs must not alias inputs. Never free a handle concurrently with a call.
 */
/* These functions require the Cargo json feature and FLOWSDK_JSON. */
#ifdef FLOWSDK_JSON
int mqtt_engine_new_v1(const uint8_t *json, size_t len, MqttEngineFFI **out, char **error);
int mqtt_engine_command_v1(const MqttEngineFFI *engine, const uint8_t *json, size_t len, uint16_t *packet_id, char **error);
int mqtt_tls_engine_new_v1(const uint8_t *json, size_t len, TlsMqttEngineFFI **out, char **error);
int mqtt_tls_engine_command_v1(const TlsMqttEngineFFI *engine, const uint8_t *json, size_t len, uint16_t *packet_id, char **error);
int mqtt_quic_engine_new_v1(const uint8_t *json, size_t len, QuicMqttEngineFFI **out, char **error);
int mqtt_quic_engine_command_v1(const QuicMqttEngineFFI *engine, const uint8_t *json, size_t len, uint16_t *packet_id, char **error);
#endif
bool mqtt_engine_has_pending_output(const MqttEngineFFI *engine);
/* Define FLOWSDK_DURABLE_SESSION when linking a library built with
 * Cargo feature durable-session. The checkpoint symbols are absent otherwise. */
#ifdef FLOWSDK_DURABLE_SESSION
int mqtt_engine_snapshot_session(const MqttEngineFFI *engine, uint8_t **out, size_t *len, char **error);
int mqtt_engine_restore_session_state(const MqttEngineFFI *engine, const uint8_t *data, size_t len, char **error);
int mqtt_tls_engine_snapshot_session(const TlsMqttEngineFFI *engine, uint8_t **out, size_t *len, char **error);
int mqtt_tls_engine_restore_session_state(const TlsMqttEngineFFI *engine, const uint8_t *data, size_t len, char **error);
int mqtt_quic_engine_snapshot_session(const QuicMqttEngineFFI *engine, uint8_t **out, size_t *len, char **error);
int mqtt_quic_engine_restore_session_state(const QuicMqttEngineFFI *engine, const uint8_t *data, size_t len, char **error);
int mqtt_session_inspect(const uint8_t *data, size_t len, char **json, char **error);
#endif
#ifdef FLOWSDK_JSON
int mqtt_event_list_get_json(const MqttEventListFFI *events, size_t index, char **json, char **error);
#endif
#ifdef __cplusplus
}
#endif
#endif
