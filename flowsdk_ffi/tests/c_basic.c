/* SPDX-License-Identifier: MPL-2.0 */
#include "flowsdk.h"
#include <assert.h>

/* The typed C API must work without JSON, persistence or UniFFI. */
int main(void) {
    MqttEngineFFI *engine = mqtt_engine_new("c-basic", 5);
    assert(engine);
    assert(!mqtt_engine_has_pending_output(engine));
    mqtt_engine_connect(engine);
    assert(mqtt_engine_has_pending_output(engine));
    size_t len = 0;
    uint8_t *bytes = mqtt_engine_take_outgoing(engine, &len);
    assert(len && bytes[0] == 0x10);
    mqtt_engine_free_bytes(bytes, len);
    assert(!mqtt_engine_has_pending_output(engine));

    const uint8_t connack[] = {0x20, 3, 0, 0, 0};
    mqtt_engine_handle_incoming(engine, connack, sizeof(connack));
    assert(mqtt_engine_is_connected(engine));
    MqttEventListFFI *events = mqtt_engine_take_events_list(engine);
    assert(mqtt_event_list_len(events) == 1);
    assert(mqtt_event_list_get_tag(events, 0) == 1); /* Connected */
    mqtt_event_list_free(events);

    const uint8_t payload[] = {0, 255};
    int32_t id = mqtt_engine_publish(engine, "test", payload, sizeof(payload), 1);
    assert(id > 0);
    bytes = mqtt_engine_take_outgoing(engine, &len);
    assert(len && bytes[0] == 0x32);
    mqtt_engine_free_bytes(bytes, len);
    const uint8_t ack[] = {0x40, 2, id >> 8, id & 255};
    mqtt_engine_handle_incoming(engine, ack, sizeof(ack));
    events = mqtt_engine_take_events_list(engine);
    assert(mqtt_event_list_len(events) == 1);
    assert(mqtt_event_list_get_tag(events, 0) == 4); /* Published */
    assert(mqtt_event_list_get_published_pid(events, 0) == id);
    mqtt_event_list_free(events);
    mqtt_engine_free(engine);
    return 0;
}
