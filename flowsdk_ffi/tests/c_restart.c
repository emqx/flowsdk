/* SPDX-License-Identifier: MPL-2.0 */
#include "flowsdk.h"
#include <assert.h>
#include <stdio.h>
#include <string.h>

static void ok(int status, char **error) {
    if (status) fprintf(stderr, "%s\n", *error ? *error : "C API failed");
    assert(status == MQTT_OK);
    assert(*error == NULL);
}

int main(void) {
    const char *config = "{\"version\":1,\"connect\":{\"options\":{\"client_id\":\"c-restart\",\"clean_start\":false},"
        "\"properties\":[{\"SessionExpiryInterval\":{\"value\":3600}}]},\"runtime\":{\"peer\":\"tcp://broker:1883\"}}";
    const char *connect = "{\"command\":\"connect\"}";
    const char *publish = "{\"command\":\"publish\",\"topic\":\"test\",\"payload\":[0,255],\"options\":{\"qos\":1}}";
    MqttEngineFFI *engine = NULL;
    char *error = NULL, *info = NULL;
    uint8_t *bytes = NULL;
    size_t len = 0;
    uint16_t id = 0;
    ok(mqtt_engine_new_v1((const uint8_t *)config, strlen(config), &engine, &error), &error);
    ok(mqtt_engine_command_v1(engine, (const uint8_t *)connect, strlen(connect), NULL, &error), &error);
    bytes = mqtt_engine_take_outgoing(engine, &len);
    assert(len && bytes[0] == 0x10);
    mqtt_engine_free_bytes(bytes, len);
    const uint8_t connack[] = {0x20, 3, 0, 0, 0};
    mqtt_engine_handle_incoming(engine, connack, sizeof(connack));
    ok(mqtt_engine_command_v1(engine, (const uint8_t *)publish, strlen(publish), &id, &error), &error);
    assert(id > 0);
    bytes = mqtt_engine_take_outgoing(engine, &len);
    assert(len && bytes[0] == 0x32);
    mqtt_engine_free_bytes(bytes, len);
#ifdef FLOWSDK_DURABLE_SESSION
    ok(mqtt_engine_snapshot_session(engine, &bytes, &len, &error), &error);
    mqtt_engine_free(engine);
    /* A real application atomically commits these opaque bytes before restart. */
    FILE *file = tmpfile();
    assert(file && fwrite(bytes, 1, len, file) == len);
    assert(fflush(file) == 0);
    rewind(file);
    assert(fread(bytes, 1, len, file) == len);
    fclose(file);
    ok(mqtt_session_inspect(bytes, len, &info, &error), &error);
    assert(strstr(info, "c-restart"));
    mqtt_engine_free_string(info);
    ok(mqtt_engine_new_v1((const uint8_t *)config, strlen(config), &engine, &error), &error);
    ok(mqtt_engine_restore_session_state(engine, bytes, len, &error), &error);
    mqtt_engine_free_bytes(bytes, len);
#else
    mqtt_engine_handle_connection_lost(engine);
#endif
    ok(mqtt_engine_command_v1(engine, (const uint8_t *)connect, strlen(connect), NULL, &error), &error);
    bytes = mqtt_engine_take_outgoing(engine, &len);
    mqtt_engine_free_bytes(bytes, len);
    const uint8_t resumed[] = {0x20, 3, 1, 0, 0};
    mqtt_engine_handle_incoming(engine, resumed, sizeof(resumed));
    bytes = mqtt_engine_take_outgoing(engine, &len);
    assert(len && bytes[0] == 0x3a); /* DUP QoS 1 */
    mqtt_engine_free_bytes(bytes, len);
    const uint8_t ack[] = {0x40, 2, id >> 8, id & 255};
    mqtt_engine_handle_incoming(engine, ack, sizeof(ack));
    info = mqtt_engine_take_events(engine);
    assert(strstr(info, "Published"));
    mqtt_engine_free_string(info);
    mqtt_engine_free(engine);
#ifdef FLOWSDK_DURABLE_SESSION
    /* Failure outputs are deterministic and allocations have the same owners. */
    len = 999;
    assert(mqtt_engine_snapshot_session(NULL, &bytes, &len, &error) == MQTT_INVALID_ARGUMENT);
    assert(bytes == NULL && len == 0 && error != NULL);
    mqtt_engine_free_string(error);
#endif
    return 0;
}
