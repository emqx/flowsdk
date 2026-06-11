/*
 * SPDX-License-Identifier: MPL-2.0
 *
 * paho_tls_smoke.c — Smoke test for TLS/SSL support (Phase 5), sync API.
 *
 * Build (after `cargo build -p flowsdk_paho`):
 *   cc -I../include paho_tls_smoke.c \
 *      -L../../target/debug -lflowsdk_paho -lpthread -ldl -lm \
 *      -o paho_tls_smoke
 *
 * Run:
 *   DYLD_LIBRARY_PATH=../../target/debug ./paho_tls_smoke
 *
 * Connects to the public TLS endpoint ssl://broker.emqx.io:8883 with server
 * certificate verification enabled (validated against the system trust store),
 * then exercises subscribe → publish → receive → disconnect.
 */

#include "MQTTClient.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define ADDRESS  "ssl://broker.emqx.io:8883"
#define CLIENTID "flowsdk_paho_tls_smoke"
#define TOPIC    "flowsdk/paho/tls/smoke"
#define PAYLOAD  "hello over TLS"
#define QOS      1

static volatile int message_received = 0;

static int onMessage(void* ctx, char* topicName, int topicLen, MQTTClient_message* m)
{
    (void)ctx; (void)topicLen;
    printf("[recv] topic='%s' payload='%.*s'\n",
           topicName, m->payloadlen, (char*)m->payload);
    message_received = 1;
    MQTTClient_freeMessage(&m);
    MQTTClient_free(topicName);
    return 1;
}

static void onConnLost(void* ctx, char* cause)
{
    (void)ctx;
    printf("[conn] lost: %s\n", cause ? cause : "(unknown)");
}

int main(void)
{
    MQTTClient client;
    MQTTClient_connectOptions conn_opts = MQTTClient_connectOptions_initializer;
    MQTTClient_SSLOptions ssl_opts = MQTTClient_SSLOptions_initializer;
    int rc;

    rc = MQTTClient_create(&client, ADDRESS, CLIENTID,
                           MQTTCLIENT_PERSISTENCE_NONE, NULL);
    if (rc != MQTTCLIENT_SUCCESS) { printf("FAIL: create rc=%d\n", rc); return 1; }
    printf("OK: created\n");

    MQTTClient_setCallbacks(client, NULL, onConnLost, onMessage, NULL);

    /* Full server-certificate + hostname verification against system roots. */
    ssl_opts.enableServerCertAuth = 1;
    ssl_opts.verify               = 1;
    ssl_opts.sslVersion           = MQTT_SSL_VERSION_TLS_1_2;

    conn_opts.keepAliveInterval = 20;
    conn_opts.cleansession      = 1;
    conn_opts.MQTTVersion       = MQTTVERSION_5;
    conn_opts.ssl               = &ssl_opts;

    rc = MQTTClient_connect(client, &conn_opts);
    if (rc != MQTTCLIENT_SUCCESS) {
        printf("FAIL: TLS connect rc=%d (%s)\n", rc, MQTTClient_strerror(rc));
        MQTTClient_destroy(&client);
        return 1;
    }
    printf("OK: TLS connected (isConnected=%d)\n", MQTTClient_isConnected(client));

    rc = MQTTClient_subscribe(client, TOPIC, QOS);
    if (rc != MQTTCLIENT_SUCCESS) {
        printf("FAIL: subscribe rc=%d\n", rc);
        MQTTClient_destroy(&client);
        return 1;
    }
    printf("OK: subscribed\n");

    MQTTClient_deliveryToken token;
    rc = MQTTClient_publish(client, TOPIC, (int)strlen(PAYLOAD), PAYLOAD, QOS, 0, &token);
    if (rc != MQTTCLIENT_SUCCESS) {
        printf("FAIL: publish rc=%d\n", rc);
        MQTTClient_destroy(&client);
        return 1;
    }
    printf("OK: published (token=%d)\n", token);

    for (int i = 0; i < 50 && !message_received; i++)
        usleep(100 * 1000);

    if (message_received)
        printf("OK: message round-tripped over TLS\n");
    else
        printf("WARN: no message received within timeout\n");

    rc = MQTTClient_disconnect(client, 1000);
    printf("OK: disconnect rc=%d\n", rc);
    MQTTClient_destroy(&client);
    printf("OK: destroyed\n");

    return message_received ? 0 : 2;
}
