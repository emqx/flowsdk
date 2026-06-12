/*
 * SPDX-License-Identifier: MPL-2.0
 *
 * paho_qos_smoke.c — Cross-QoS round-trip test (Phase 6 verification).
 *
 * Build (after `cargo build -p flowsdk_paho`):
 *   cc -I../include paho_qos_smoke.c \
 *      -L../../target/debug -lflowsdk_paho -lpthread -ldl -lm \
 *      -o paho_qos_smoke
 *
 * Run:
 *   DYLD_LIBRARY_PATH=../../target/debug ./paho_qos_smoke
 *
 * Subscribes at QoS 2, then publishes one message at each of QoS 0, 1 and 2
 * and verifies all three are received. The QoS 2 publish exercises the full
 * PUBREC/PUBREL/PUBCOMP handshake that the other smoke tests do not cover.
 */

#include "MQTTClient.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define ADDRESS  "tcp://broker.emqx.io:1883"
#define CLIENTID "flowsdk_paho_qos_smoke"
#define TOPIC    "flowsdk/paho/qos/smoke"

/* One received-flag per QoS level; the payload encodes the QoS it was sent at. */
static volatile int received[3] = {0, 0, 0};

static int onMessage(void* ctx, char* topicName, int topicLen, MQTTClient_message* m)
{
    (void)ctx; (void)topicLen;
    int q = -1;
    if (m->payloadlen > 0) {
        char c = ((char*)m->payload)[m->payloadlen - 1];
        if (c >= '0' && c <= '2') q = c - '0';
    }
    printf("[recv] '%.*s' (delivered qos=%d, tagged qos=%d)\n",
           m->payloadlen, (char*)m->payload, m->qos, q);
    if (q >= 0 && q <= 2) received[q] = 1;
    MQTTClient_freeMessage(&m);
    MQTTClient_free(topicName);
    return 1;
}

static void onConnLost(void* ctx, char* cause)
{
    (void)ctx;
    printf("[conn] lost: %s\n", cause ? cause : "(unknown)");
}

static int publish_at(MQTTClient client, int qos)
{
    char payload[64];
    snprintf(payload, sizeof(payload), "qos-test-%d", qos); /* last char is the QoS */
    MQTTClient_deliveryToken token;
    int rc = MQTTClient_publish(client, TOPIC, (int)strlen(payload), payload,
                                qos, 0, &token);
    if (rc != MQTTCLIENT_SUCCESS) {
        printf("FAIL: publish qos=%d rc=%d\n", qos, rc);
        return rc;
    }
    /* For QoS > 0 the token should be non-zero; QoS 0 has no token. */
    printf("OK: published qos=%d (token=%d)\n", qos, token);
    if (qos > 0) {
        rc = MQTTClient_waitForCompletion(client, token, 5000);
        printf("   waitForCompletion qos=%d rc=%d\n", qos, rc);
    }
    return MQTTCLIENT_SUCCESS;
}

int main(void)
{
    MQTTClient client;
    MQTTClient_connectOptions conn_opts = MQTTClient_connectOptions_initializer;
    int rc;

    rc = MQTTClient_create(&client, ADDRESS, CLIENTID,
                           MQTTCLIENT_PERSISTENCE_NONE, NULL);
    if (rc != MQTTCLIENT_SUCCESS) { printf("FAIL: create rc=%d\n", rc); return 1; }

    MQTTClient_setCallbacks(client, NULL, onConnLost, onMessage, NULL);

    conn_opts.keepAliveInterval = 20;
    conn_opts.cleansession      = 1;
    conn_opts.MQTTVersion       = MQTTVERSION_5;

    rc = MQTTClient_connect(client, &conn_opts);
    if (rc != MQTTCLIENT_SUCCESS) {
        printf("FAIL: connect rc=%d (%s)\n", rc, MQTTClient_strerror(rc));
        MQTTClient_destroy(&client);
        return 1;
    }
    printf("OK: connected\n");

    rc = MQTTClient_subscribe(client, TOPIC, 2); /* subscribe at QoS 2 */
    if (rc != MQTTCLIENT_SUCCESS) {
        printf("FAIL: subscribe rc=%d\n", rc);
        MQTTClient_destroy(&client);
        return 1;
    }
    printf("OK: subscribed at QoS 2\n");

    for (int q = 0; q <= 2; q++) {
        if (publish_at(client, q) != MQTTCLIENT_SUCCESS) {
            MQTTClient_destroy(&client);
            return 1;
        }
    }

    /* Wait for all three messages to come back. */
    for (int i = 0; i < 80; i++) {
        if (received[0] && received[1] && received[2]) break;
        usleep(100 * 1000);
    }

    int ok = received[0] && received[1] && received[2];
    printf("received: qos0=%d qos1=%d qos2=%d\n", received[0], received[1], received[2]);

    MQTTClient_disconnect(client, 1000);
    MQTTClient_destroy(&client);

    if (ok) { printf("OK: all QoS levels round-tripped\n"); return 0; }
    printf("FAIL: not all QoS levels round-tripped\n");
    return 2;
}
