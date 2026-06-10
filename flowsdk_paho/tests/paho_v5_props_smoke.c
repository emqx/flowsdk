/*
 * SPDX-License-Identifier: MPL-2.0
 *
 * paho_v5_props_smoke.c — Smoke test for MQTT v5 property support (Phase 4).
 *
 * Build (after `cargo build -p flowsdk_paho`):
 *   cc -I../include paho_v5_props_smoke.c \
 *      -L../../target/debug -lflowsdk_paho -lpthread -ldl -lm \
 *      -o paho_v5_props_smoke
 *
 * Run:
 *   DYLD_LIBRARY_PATH=../../target/debug ./paho_v5_props_smoke
 *
 * Exercises (all over MQTT v5):
 *   - connect with onSuccess5  (reads reasonCode + server CONNACK properties)
 *   - publish with a user property attached via MQTTAsync_responseOptions.properties
 *   - messageArrived reads message->properties back and verifies the round-trip
 *   - the MQTTProperties_* helper API (add / getProperty / free)
 */

#include "MQTTAsync.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define ADDRESS  "tcp://broker.emqx.io:1883"
#define CLIENTID "flowsdk_paho_v5props_smoke"
#define TOPIC    "flowsdk/paho/v5props/smoke"
#define PAYLOAD  "v5 payload with properties"
#define QOS      1

#define UP_KEY   "sensor_id"
#define UP_VALUE "room-42"

static volatile int g_connected     = 0;
static volatile int g_connect_fail  = 0;
static volatile int g_subscribed    = 0;
static volatile int g_msg_received  = 0;
static volatile int g_prop_matched  = 0;

/* ── Connect (v5) ──────────────────────────────────────────────────────── */

static void onConnect5(void* ctx, MQTTAsync_successData5* r)
{
    (void)ctx;
    printf("[conn] connected v5: reasonCode=%d, %d CONNACK properties\n",
           r ? r->reasonCode : -1, r ? r->properties.count : 0);
    g_connected = 1;
}

static void onConnectFailure5(void* ctx, MQTTAsync_failureData5* r)
{
    (void)ctx;
    printf("[conn] FAIL v5: reasonCode=%d code=%d msg=%s\n",
           r ? r->reasonCode : -1, r ? r->code : -1,
           (r && r->message) ? r->message : "(none)");
    g_connect_fail = 1;
}

static void onConnLost(void* ctx, char* cause)
{
    (void)ctx;
    printf("[conn] lost: %s\n", cause ? cause : "(unknown)");
}

/* ── Subscribe ─────────────────────────────────────────────────────────── */

static void onSubscribe5(void* ctx, MQTTAsync_successData5* r)
{
    (void)ctx;
    printf("[sub]  subscribed: reasonCode=%d\n", r ? r->reasonCode : -1);
    g_subscribed = 1;
}

/* ── Message arrival — inspect v5 properties ───────────────────────────── */

static int onMessage(void* ctx, char* topicName, int topicLen, MQTTAsync_message* msg)
{
    (void)ctx; (void)topicLen;
    printf("[recv] topic='%s' payload='%.*s' (%d properties)\n",
           topicName, msg->payloadlen, (char*)msg->payload, msg->properties.count);

    /* Look for our user property in the delivered message. */
    MQTTProperty* p = MQTTProperties_getProperty(&msg->properties,
                                                 MQTTPROPERTY_CODE_USER_PROPERTY);
    if (p != NULL) {
        int klen = p->value.data.len;
        int vlen = p->value.value.len;
        printf("[recv] user property: '%.*s' = '%.*s'\n",
               klen, p->value.data.data, vlen, p->value.value.data);
        if (klen == (int)strlen(UP_KEY) &&
            memcmp(p->value.data.data, UP_KEY, klen) == 0 &&
            vlen == (int)strlen(UP_VALUE) &&
            memcmp(p->value.value.data, UP_VALUE, vlen) == 0) {
            g_prop_matched = 1;
        }
    } else {
        printf("[recv] WARN: no user property on message\n");
    }

    g_msg_received = 1;
    MQTTAsync_freeMessage(&msg);
    MQTTAsync_free(topicName);
    return 1;
}

static void onDeliveryComplete(void* ctx, MQTTAsync_token token)
{
    (void)ctx;
    printf("[pub]  delivery complete, token=%d\n", token);
}

/* ── Helper ────────────────────────────────────────────────────────────── */

static int wait_flag(volatile int* flag, int timeout_ms)
{
    for (int i = 0; i < timeout_ms / 100 && !*flag; i++)
        usleep(100 * 1000);
    return *flag;
}

/* ── main ──────────────────────────────────────────────────────────────── */

int main(void)
{
    MQTTAsync client;
    int rc;

    /* Quick exercise of the property helper API + type classification. */
    printf("getType(USER_PROPERTY)  = %d (expect %d)\n",
           MQTTProperty_getType(MQTTPROPERTY_CODE_USER_PROPERTY),
           MQTTPROPERTY_TYPE_UTF_8_STRING_PAIR);
    printf("PropertyName(TOPIC_ALIAS) = %s\n",
           MQTTPropertyName(MQTTPROPERTY_CODE_TOPIC_ALIAS));

    rc = MQTTAsync_create(&client, ADDRESS, CLIENTID,
                          MQTTCLIENT_PERSISTENCE_NONE, NULL);
    if (rc != MQTTASYNC_SUCCESS) { printf("FAIL: create rc=%d\n", rc); return 1; }
    printf("OK: created\n");

    MQTTAsync_setCallbacks(client, NULL, onConnLost, onMessage, onDeliveryComplete);

    MQTTAsync_connectOptions conn_opts = MQTTAsync_connectOptions_initializer;
    conn_opts.keepAliveInterval = 20;
    conn_opts.cleanstart        = 1;
    conn_opts.MQTTVersion       = MQTTVERSION_5;
    conn_opts.onSuccess5        = onConnect5;
    conn_opts.onFailure5        = onConnectFailure5;

    rc = MQTTAsync_connect(client, &conn_opts);
    if (rc != MQTTASYNC_SUCCESS) {
        printf("FAIL: connect rc=%d\n", rc);
        MQTTAsync_destroy(&client);
        return 1;
    }
    if (!wait_flag(&g_connected, 10000) || g_connect_fail) {
        printf("FAIL: connect timeout/error\n");
        MQTTAsync_destroy(&client);
        return 1;
    }

    MQTTAsync_responseOptions sub_opts = MQTTAsync_responseOptions_initializer;
    sub_opts.onSuccess5 = onSubscribe5;
    rc = MQTTAsync_subscribe(client, TOPIC, QOS, &sub_opts);
    if (rc != MQTTASYNC_SUCCESS) {
        printf("FAIL: subscribe rc=%d\n", rc);
        MQTTAsync_destroy(&client);
        return 1;
    }
    if (!wait_flag(&g_subscribed, 10000)) {
        printf("FAIL: subscribe timeout\n");
        MQTTAsync_destroy(&client);
        return 1;
    }

    /* Publish with a user property attached. */
    MQTTAsync_responseOptions pub_opts = MQTTAsync_responseOptions_initializer;
    MQTTProperty up;
    memset(&up, 0, sizeof(up));
    up.identifier       = MQTTPROPERTY_CODE_USER_PROPERTY;
    up.value.data.data  = (char*)UP_KEY;
    up.value.data.len   = (int)strlen(UP_KEY);
    up.value.value.data = (char*)UP_VALUE;
    up.value.value.len  = (int)strlen(UP_VALUE);
    if (MQTTProperties_add(&pub_opts.properties, &up) != 0) {
        printf("FAIL: MQTTProperties_add\n");
        MQTTAsync_destroy(&client);
        return 1;
    }
    printf("OK: publish carries %d property (len=%d)\n",
           pub_opts.properties.count, MQTTProperties_len(&pub_opts.properties));

    rc = MQTTAsync_send(client, TOPIC, (int)strlen(PAYLOAD), PAYLOAD, QOS, 0, &pub_opts);
    MQTTProperties_free(&pub_opts.properties); /* library deep-copied it */
    if (rc != MQTTASYNC_SUCCESS) {
        printf("FAIL: send rc=%d\n", rc);
        MQTTAsync_destroy(&client);
        return 1;
    }
    printf("OK: send dispatched (token=%d)\n", pub_opts.token);

    if (!wait_flag(&g_msg_received, 10000)) {
        printf("WARN: no message received within timeout\n");
    }

    MQTTAsync_disconnectOptions disc = MQTTAsync_disconnectOptions_initializer;
    disc.timeout = 1000;
    MQTTAsync_disconnect(client, &disc);
    usleep(300 * 1000);
    MQTTAsync_destroy(&client);
    printf("OK: destroyed\n");

    if (g_prop_matched) {
        printf("OK: user property round-tripped (%s=%s)\n", UP_KEY, UP_VALUE);
        return 0;
    }
    printf("FAIL: user property did not round-trip\n");
    return 2;
}
