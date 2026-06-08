/*
 * SPDX-License-Identifier: MPL-2.0
 *
 * paho_async_smoke.c — Smoke test for the FlowSDK Paho-compatible async API.
 *
 * Build (after `cargo build -p flowsdk_paho`):
 *   cc -I../include paho_async_smoke.c \
 *      -L../../target/debug -lflowsdk_paho -lpthread -ldl -lm \
 *      -o paho_async_smoke
 *
 * Run:
 *   DYLD_LIBRARY_PATH=../../target/debug ./paho_async_smoke
 *
 * Exercises: create → setCallbacks → connect (onSuccess/onFailure) →
 *            subscribe (onSuccess) → send (onSuccess) →
 *            messageArrived loopback → disconnect (onSuccess) → destroy.
 */

#include "MQTTAsync.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define ADDRESS  "tcp://broker.emqx.io:1883"
#define CLIENTID "flowsdk_paho_async_smoke"
#define TOPIC    "flowsdk/paho/async/smoke"
#define PAYLOAD  "hello from flowsdk paho async"
#define QOS      1

/* ── State flags (set from callbacks, polled in main) ──────────────────── */

static volatile int g_connected      = 0;
static volatile int g_connect_failed = 0;
static volatile int g_subscribed     = 0;
static volatile int g_msg_received   = 0;
static volatile int g_disconnected   = 0;

/* ── Connection callbacks ────────────────────────────────────────────────── */

static void onConnect(void* ctx, MQTTAsync_successData* response)
{
    (void)ctx;
    printf("[conn] connected, server='%s' mqttver=%d sessionPresent=%d\n",
           response ? response->alt.connect.serverURI    : "?",
           response ? response->alt.connect.MQTTVersion  : 0,
           response ? response->alt.connect.sessionPresent : 0);
    g_connected = 1;
}

static void onConnectFailure(void* ctx, MQTTAsync_failureData* response)
{
    (void)ctx;
    printf("[conn] FAIL: code=%d msg=%s\n",
           response ? response->code : -1,
           (response && response->message) ? response->message : "(none)");
    g_connect_failed = 1;
}

static void onConnLost(void* ctx, char* cause)
{
    (void)ctx;
    printf("[conn] connection lost: %s\n", cause ? cause : "(unknown)");
}

/* ── Subscribe callbacks ─────────────────────────────────────────────────── */

static void onSubscribe(void* ctx, MQTTAsync_successData* response)
{
    (void)ctx; (void)response;
    printf("[sub]  subscribed to " TOPIC "\n");
    g_subscribed = 1;
}

static void onSubscribeFailure(void* ctx, MQTTAsync_failureData* response)
{
    (void)ctx;
    printf("[sub]  FAIL: code=%d\n", response ? response->code : -1);
}

/* ── Publish callbacks ───────────────────────────────────────────────────── */

static void onPublish(void* ctx, MQTTAsync_successData* response)
{
    (void)ctx;
    printf("[pub]  delivery confirmed, token=%d\n",
           response ? response->token : -1);
}

static void onPublishFailure(void* ctx, MQTTAsync_failureData* response)
{
    (void)ctx;
    printf("[pub]  FAIL: code=%d\n", response ? response->code : -1);
}

/* ── Message arrival ─────────────────────────────────────────────────────── */

static int onMessage(void* ctx, char* topicName, int topicLen,
                     MQTTAsync_message* msg)
{
    (void)ctx; (void)topicLen;
    printf("[recv] topic='%s' payload='%.*s'\n",
           topicName, msg->payloadlen, (char*)msg->payload);
    g_msg_received = 1;
    MQTTAsync_freeMessage(&msg);
    MQTTAsync_free(topicName);
    return 1;
}

/* ── Delivery-complete callback ──────────────────────────────────────────── */

static void onDeliveryComplete(void* ctx, MQTTAsync_token token)
{
    (void)ctx;
    printf("[pub]  delivery complete, token=%d\n", token);
}

/* ── Disconnect callback ─────────────────────────────────────────────────── */

static void onDisconnect(void* ctx, MQTTAsync_successData* response)
{
    (void)ctx; (void)response;
    printf("[disc] disconnected\n");
    g_disconnected = 1;
}

/* ── Helper: poll a flag with a millisecond timeout ─────────────────────── */

static int wait_flag(volatile int* flag, int timeout_ms)
{
    for (int i = 0; i < timeout_ms / 100 && !*flag; i++)
        usleep(100 * 1000);
    return *flag;
}

/* ── main ────────────────────────────────────────────────────────────────── */

int main(void)
{
    MQTTAsync client;
    int rc;

    printf("strerror(0)  = %s\n", MQTTAsync_strerror(MQTTASYNC_SUCCESS));
    printf("strerror(-1) = %s\n", MQTTAsync_strerror(MQTTASYNC_FAILURE));

    /* 1. Create */
    rc = MQTTAsync_create(&client, ADDRESS, CLIENTID,
                          MQTTCLIENT_PERSISTENCE_NONE, NULL);
    if (rc != MQTTASYNC_SUCCESS) {
        printf("FAIL: create rc=%d\n", rc);
        return 1;
    }
    printf("OK: created\n");

    /* 2. Set persistent callbacks (conn-lost, message-arrived, delivery-complete) */
    MQTTAsync_setCallbacks(client, NULL, onConnLost, onMessage, onDeliveryComplete);

    /* 3. Connect — non-blocking; wait for onConnect/onConnectFailure */
    MQTTAsync_connectOptions conn_opts = MQTTAsync_connectOptions_initializer;
    conn_opts.keepAliveInterval = 20;
    conn_opts.cleansession      = 1;
    conn_opts.MQTTVersion       = MQTTVERSION_5;
    conn_opts.onSuccess         = onConnect;
    conn_opts.onFailure         = onConnectFailure;

    rc = MQTTAsync_connect(client, &conn_opts);
    if (rc != MQTTASYNC_SUCCESS) {
        printf("FAIL: connect rc=%d\n", rc);
        MQTTAsync_destroy(&client);
        return 1;
    }
    if (!wait_flag(&g_connected, 10000) || g_connect_failed) {
        printf("FAIL: connect timeout or error\n");
        MQTTAsync_destroy(&client);
        return 1;
    }
    printf("OK: connected (isConnected=%d)\n", MQTTAsync_isConnected(client));

    /* 4. Subscribe — non-blocking; wait for onSubscribe */
    MQTTAsync_responseOptions sub_opts = MQTTAsync_responseOptions_initializer;
    sub_opts.onSuccess = onSubscribe;
    sub_opts.onFailure = onSubscribeFailure;

    rc = MQTTAsync_subscribe(client, TOPIC, QOS, &sub_opts);
    if (rc != MQTTASYNC_SUCCESS) {
        printf("FAIL: subscribe rc=%d\n", rc);
        MQTTAsync_destroy(&client);
        return 1;
    }
    printf("OK: subscribe dispatched (token=%d)\n", sub_opts.token);
    if (!wait_flag(&g_subscribed, 10000)) {
        printf("FAIL: subscribe timeout\n");
        MQTTAsync_destroy(&client);
        return 1;
    }
    printf("OK: subscribed\n");

    /* 5. Publish — non-blocking; token is set synchronously in pub_opts.token */
    MQTTAsync_responseOptions pub_opts = MQTTAsync_responseOptions_initializer;
    pub_opts.onSuccess = onPublish;
    pub_opts.onFailure = onPublishFailure;

    rc = MQTTAsync_send(client, TOPIC,
                        (int)strlen(PAYLOAD), PAYLOAD,
                        QOS, 0, &pub_opts);
    if (rc != MQTTASYNC_SUCCESS) {
        printf("FAIL: send rc=%d\n", rc);
        MQTTAsync_destroy(&client);
        return 1;
    }
    printf("OK: send dispatched (token=%d)\n", pub_opts.token);

    /* 6. Wait for loopback message in onMessage */
    if (!wait_flag(&g_msg_received, 10000)) {
        printf("WARN: no message received within timeout\n");
    } else {
        printf("OK: message round-tripped\n");
    }

    /* 7. Disconnect — non-blocking; wait for onDisconnect */
    MQTTAsync_disconnectOptions disc_opts = MQTTAsync_disconnectOptions_initializer;
    disc_opts.onSuccess = onDisconnect;
    disc_opts.timeout   = 1000;

    rc = MQTTAsync_disconnect(client, &disc_opts);
    if (rc != MQTTASYNC_SUCCESS) {
        printf("FAIL: disconnect rc=%d\n", rc);
        MQTTAsync_destroy(&client);
        return 1;
    }
    wait_flag(&g_disconnected, 5000);
    printf("OK: disconnect complete\n");

    /* 8. Destroy */
    MQTTAsync_destroy(&client);
    printf("OK: destroyed\n");

    return g_msg_received ? 0 : 2;
}
