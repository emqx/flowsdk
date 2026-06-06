/*
 * SPDX-License-Identifier: MPL-2.0
 *
 * paho_smoke.c — Smoke test for the FlowSDK Paho-compatible synchronous API.
 *
 * Build (after `cargo build -p flowsdk_paho`):
 *   cc -I../include paho_smoke.c \
 *      -L../../target/debug -lflowsdk_paho -lpthread -ldl -lm \
 *      -o paho_smoke
 *
 * Run:
 *   ./paho_smoke
 *
 * Exercises: create → connect → subscribe → publish → receive → disconnect → destroy.
 */

#include "MQTTClient.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define ADDRESS "tcp://broker.emqx.io:1883"
#define CLIENTID "flowsdk_paho_smoke"
#define TOPIC "flowsdk/paho/smoke"
#define PAYLOAD "hello from flowsdk paho"
#define QOS 1

static volatile int message_received = 0;

int onMessage(void* context, char* topicName, int topicLen, MQTTClient_message* message)
{
    (void)context;
    (void)topicLen;
    printf("[recv] topic='%s' payload='%.*s'\n",
           topicName, message->payloadlen, (char*)message->payload);
    message_received = 1;
    MQTTClient_freeMessage(&message);
    MQTTClient_free(topicName);
    return 1;
}

void onConnLost(void* context, char* cause)
{
    (void)context;
    printf("[conn] connection lost: %s\n", cause ? cause : "(unknown)");
}

void onDelivery(void* context, MQTTClient_deliveryToken dt)
{
    (void)context;
    printf("[send] delivery complete, token=%d\n", dt);
}

int main(void)
{
    MQTTClient client;
    MQTTClient_connectOptions conn_opts = MQTTClient_connectOptions_initializer;
    int rc;

    printf("strerror(0) = %s\n", MQTTClient_strerror(MQTTCLIENT_SUCCESS));
    printf("strerror(-1) = %s\n", MQTTClient_strerror(MQTTCLIENT_FAILURE));

    rc = MQTTClient_create(&client, ADDRESS, CLIENTID,
                           MQTTCLIENT_PERSISTENCE_NONE, NULL);
    if (rc != MQTTCLIENT_SUCCESS) {
        printf("FAIL: create rc=%d\n", rc);
        return 1;
    }
    printf("OK: created\n");

    MQTTClient_setCallbacks(client, NULL, onConnLost, onMessage, onDelivery);

    conn_opts.keepAliveInterval = 20;
    conn_opts.cleansession = 1;
    conn_opts.MQTTVersion = MQTTVERSION_5;

    rc = MQTTClient_connect(client, &conn_opts);
    if (rc != MQTTCLIENT_SUCCESS) {
        printf("FAIL: connect rc=%d (%s)\n", rc, MQTTClient_strerror(rc));
        MQTTClient_destroy(&client);
        return 1;
    }
    printf("OK: connected (isConnected=%d)\n", MQTTClient_isConnected(client));

    rc = MQTTClient_subscribe(client, TOPIC, QOS);
    if (rc != MQTTCLIENT_SUCCESS) {
        printf("FAIL: subscribe rc=%d\n", rc);
        MQTTClient_destroy(&client);
        return 1;
    }
    printf("OK: subscribed to %s\n", TOPIC);

    MQTTClient_deliveryToken token;
    rc = MQTTClient_publish(client, TOPIC, (int)strlen(PAYLOAD), PAYLOAD,
                            QOS, 0, &token);
    if (rc != MQTTCLIENT_SUCCESS) {
        printf("FAIL: publish rc=%d\n", rc);
        MQTTClient_destroy(&client);
        return 1;
    }
    printf("OK: published\n");

    /* Wait up to 5 seconds for the loopback message */
    for (int i = 0; i < 50 && !message_received; i++) {
        usleep(100 * 1000);
    }

    if (!message_received) {
        printf("WARN: no message received within timeout\n");
    } else {
        printf("OK: message round-tripped\n");
    }

    rc = MQTTClient_disconnect(client, 1000);
    printf("OK: disconnect rc=%d\n", rc);

    MQTTClient_destroy(&client);
    printf("OK: destroyed\n");

    return message_received ? 0 : 2;
}
