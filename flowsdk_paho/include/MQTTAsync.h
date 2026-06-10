/*
 * SPDX-License-Identifier: MPL-2.0
 *
 * MQTTAsync.h — Paho C asynchronous API, implemented by FlowSDK.
 *
 * This header is a drop-in compatible subset of the Eclipse Paho C
 * `MQTTAsync.h`. Programs written against Paho's asynchronous API can be
 * compiled and linked against the FlowSDK implementation (libflowsdk_paho).
 *
 * NOTE: MQTT v5 property handling (MQTTProperties contents) is a layout-only
 * placeholder in this release; individual property get/set is not yet wired.
 */

#ifndef FLOWSDK_PAHO_MQTTASYNC_H
#define FLOWSDK_PAHO_MQTTASYNC_H

#include "MQTTClient.h"

#if defined(__cplusplus)
extern "C" {
#endif

/* ─── Return codes (aliases of the MQTTCLIENT_* values) ──────────────── */

#define MQTTASYNC_SUCCESS 0
#define MQTTASYNC_FAILURE -1
#define MQTTASYNC_DISCONNECTED -3
#define MQTTASYNC_MAX_MESSAGES_INFLIGHT -4
#define MQTTASYNC_BAD_UTF8_STRING -5
#define MQTTASYNC_NULL_PARAMETER -6
#define MQTTASYNC_TOPICNAME_TRUNCATED -7
#define MQTTASYNC_BAD_STRUCTURE -8
#define MQTTASYNC_BAD_QOS -9
#define MQTTASYNC_SSL_NOT_SUPPORTED -10
#define MQTTASYNC_BAD_MQTT_VERSION -11
#define MQTTASYNC_BAD_PROTOCOL -14
#define MQTTASYNC_BAD_MQTT_OPTION -15
#define MQTTASYNC_WRONG_MQTT_VERSION -16
#define MQTTASYNC_0_LEN_WILL_TOPIC -17

/* ─── Handle / token / message aliases ───────────────────────────────── */

typedef void* MQTTAsync;
typedef int MQTTAsync_token;
typedef MQTTClient_message MQTTAsync_message;
typedef MQTTClient_willOptions MQTTAsync_willOptions;
typedef MQTTClient_SSLOptions MQTTAsync_SSLOptions;
typedef MQTTClient_nameValue MQTTAsync_nameValue;

#define MQTTAsync_message_initializer MQTTClient_message_initializer
#define MQTTAsync_willOptions_initializer MQTTClient_willOptions_initializer
#define MQTTAsync_SSLOptions_initializer MQTTClient_SSLOptions_initializer

/* ─── MQTT v5 property containers ─────────────────────────────────────── */

/* MQTTProperty / MQTTProperties and their API come from MQTTProperties.h,
 * pulled in transitively via MQTTClient.h above. */

typedef struct MQTTSubscribe_options
{
    char struct_id[4];          /**< Must be "MQSO" */
    int struct_version;
    unsigned char noLocal;
    unsigned char retainAsPublished;
    unsigned char retainHandling;
} MQTTSubscribe_options;

#define MQTTSubscribe_options_initializer { {'M', 'Q', 'S', 'O'}, 0, 0, 0, 0 }

/* ─── Success / failure data ─────────────────────────────────────────── */

typedef struct
{
    int token;
    union
    {
        struct
        {
            MQTTAsync_message message;
            char* destinationName;
        } pub;
        struct
        {
            char* serverURI;
            int MQTTVersion;
            int sessionPresent;
        } connect;
        int qos;
        int* qosList;
    } alt;
} MQTTAsync_successData;

typedef struct
{
    int token;
    int code;
    const char* message;
} MQTTAsync_failureData;

typedef struct
{
    int token;
    int reasonCode;
    MQTTProperties properties;
    union
    {
        struct
        {
            MQTTAsync_message message;
            char* destinationName;
        } pub;
        struct
        {
            char* serverURI;
            int MQTTVersion;
            int sessionPresent;
        } connect;
        int qos;
        int* qosList;
    } alt;
} MQTTAsync_successData5;

typedef struct
{
    int token;
    int reasonCode;
    MQTTProperties properties;
    int code;
    const char* message;
    int packet_type;
} MQTTAsync_failureData5;

/* ─── Callback function types ────────────────────────────────────────── */

typedef void MQTTAsync_onSuccess(void* context, MQTTAsync_successData* response);
typedef void MQTTAsync_onFailure(void* context, MQTTAsync_failureData* response);
typedef void MQTTAsync_onSuccess5(void* context, MQTTAsync_successData5* response);
typedef void MQTTAsync_onFailure5(void* context, MQTTAsync_failureData5* response);

typedef void MQTTAsync_connectionLost(void* context, char* cause);
typedef int MQTTAsync_messageArrived(void* context, char* topicName, int topicLen,
                                     MQTTAsync_message* message);
typedef void MQTTAsync_deliveryComplete(void* context, MQTTAsync_token token);
typedef void MQTTAsync_connected(void* context, char* cause);
typedef void MQTTAsync_disconnected(void* context, MQTTProperties* properties,
                                    int reasonCode);

/* ─── MQTTAsync_responseOptions ──────────────────────────────────────── */

typedef struct MQTTAsync_responseOptions
{
    char struct_id[4];                          /**< Must be "MQTR" */
    int struct_version;                         /**< 0 or 1 */
    MQTTAsync_onSuccess* onSuccess;
    MQTTAsync_onFailure* onFailure;
    void* context;
    MQTTAsync_token token;                       /**< output: assigned token */
    MQTTAsync_onSuccess5* onSuccess5;
    MQTTAsync_onFailure5* onFailure5;
    MQTTProperties properties;
    MQTTSubscribe_options subscribeOptions;
    int subscribeOptionsCount;
    MQTTSubscribe_options* subscribeOptionsList;
} MQTTAsync_responseOptions;

#define MQTTAsync_responseOptions_initializer \
    { {'M', 'Q', 'T', 'R'}, 1, NULL, NULL, NULL, 0, NULL, NULL, \
      MQTTProperties_initializer, MQTTSubscribe_options_initializer, 0, NULL }

/* ─── MQTTAsync_createOptions ────────────────────────────────────────── */

typedef struct MQTTAsync_createOptions
{
    char struct_id[4];                          /**< Must be "MQCO" */
    int struct_version;
    int sendWhileDisconnected;
    int maxBufferedMessages;
    int MQTTVersion;
    int allowDisconnectedSendAtAnyTime;
    int deleteOldestMessages;
    int restoreMessages;
    int persistQoS0;
} MQTTAsync_createOptions;

#define MQTTAsync_createOptions_initializer \
    { {'M', 'Q', 'C', 'O'}, 2, 0, 100, MQTTVERSION_DEFAULT, 0, 0, 1, 1 }

/* ─── MQTTAsync_connectOptions ───────────────────────────────────────── */

typedef struct MQTTAsync_connectOptions
{
    char struct_id[4];                          /**< Must be "MQTC" */
    int struct_version;
    int keepAliveInterval;
    int cleansession;
    int maxInflight;
    MQTTAsync_willOptions* will;
    const char* username;
    const char* password;
    int connectTimeout;
    int retryInterval;
    MQTTAsync_SSLOptions* ssl;
    MQTTAsync_onSuccess* onSuccess;
    MQTTAsync_onFailure* onFailure;
    void* context;
    int serverURIcount;
    char* const* serverURIs;
    int MQTTVersion;
    struct
    {
        const char* serverURI;
        int MQTTVersion;
        int sessionPresent;
    } returned;
    struct
    {
        int len;
        const void* data;
    } binarypwd;
    int cleanstart;
    MQTTProperties* connectProperties;
    MQTTProperties* willProperties;
    MQTTAsync_onSuccess5* onSuccess5;
    MQTTAsync_onFailure5* onFailure5;
    int automaticReconnect;
    int minRetryInterval;
    int maxRetryInterval;
    MQTTAsync_createOptions* createOptions;
    const MQTTAsync_nameValue* httpHeaders;
    const char* httpProxy;
    const char* httpsProxy;
} MQTTAsync_connectOptions;

#define MQTTAsync_connectOptions_initializer \
    { {'M', 'Q', 'T', 'C'}, 8, 60, 1, 65535, NULL, NULL, NULL, 30, 0, NULL, \
      NULL, NULL, NULL, 0, NULL, MQTTVERSION_DEFAULT, {NULL, 0, 0}, {0, NULL}, \
      0, NULL, NULL, NULL, NULL, 0, 1, 60, NULL, NULL, NULL, NULL }

/* ─── MQTTAsync_disconnectOptions ────────────────────────────────────── */

typedef struct MQTTAsync_disconnectOptions
{
    char struct_id[4];                          /**< Must be "MQTD" */
    int struct_version;
    int timeout;
    MQTTAsync_onSuccess* onSuccess;
    MQTTAsync_onFailure* onFailure;
    void* context;
    MQTTProperties properties;
    int reasonCode;
    MQTTAsync_onSuccess5* onSuccess5;
    MQTTAsync_onFailure5* onFailure5;
} MQTTAsync_disconnectOptions;

#define MQTTAsync_disconnectOptions_initializer \
    { {'M', 'Q', 'T', 'D'}, 0, 0, NULL, NULL, NULL, \
      MQTTProperties_initializer, 0, NULL, NULL }

/* ─── Lifecycle ──────────────────────────────────────────────────────── */

int MQTTAsync_create(MQTTAsync* handle, const char* serverURI, const char* clientId,
                     int persistence_type, void* persistence_context);

int MQTTAsync_createWithOptions(MQTTAsync* handle, const char* serverURI,
                                const char* clientId, int persistence_type,
                                void* persistence_context,
                                MQTTAsync_createOptions* options);

void MQTTAsync_destroy(MQTTAsync* handle);

/* ─── Callbacks ──────────────────────────────────────────────────────── */

int MQTTAsync_setCallbacks(MQTTAsync handle, void* context,
                           MQTTAsync_connectionLost* cl,
                           MQTTAsync_messageArrived* ma,
                           MQTTAsync_deliveryComplete* dc);

int MQTTAsync_setConnected(MQTTAsync handle, void* context, MQTTAsync_connected* co);

int MQTTAsync_setDisconnected(MQTTAsync handle, void* context,
                              MQTTAsync_disconnected* co);

/* ─── Connection ─────────────────────────────────────────────────────── */

int MQTTAsync_connect(MQTTAsync handle, const MQTTAsync_connectOptions* options);

int MQTTAsync_disconnect(MQTTAsync handle, const MQTTAsync_disconnectOptions* options);

int MQTTAsync_isConnected(MQTTAsync handle);

/* ─── Publish ────────────────────────────────────────────────────────── */

int MQTTAsync_send(MQTTAsync handle, const char* destinationName, int payloadlen,
                   const void* payload, int qos, int retained,
                   MQTTAsync_responseOptions* response);

int MQTTAsync_sendMessage(MQTTAsync handle, const char* destinationName,
                          const MQTTAsync_message* message,
                          MQTTAsync_responseOptions* response);

/* ─── Subscribe ──────────────────────────────────────────────────────── */

int MQTTAsync_subscribe(MQTTAsync handle, const char* topic, int qos,
                        MQTTAsync_responseOptions* response);

int MQTTAsync_subscribeMany(MQTTAsync handle, int count, char* const* topic,
                            int* qos, MQTTAsync_responseOptions* response);

int MQTTAsync_unsubscribe(MQTTAsync handle, const char* topic,
                          MQTTAsync_responseOptions* response);

int MQTTAsync_unsubscribeMany(MQTTAsync handle, int count, char* const* topic,
                              MQTTAsync_responseOptions* response);

/* ─── Utilities ──────────────────────────────────────────────────────── */

int MQTTAsync_waitForCompletion(MQTTAsync handle, MQTTAsync_token token,
                                unsigned long timeout);

int MQTTAsync_getPendingTokens(MQTTAsync handle, MQTTAsync_token** tokens);

void MQTTAsync_free(void* ptr);

void MQTTAsync_freeMessage(MQTTAsync_message** message);

const char* MQTTAsync_strerror(int code);

#if defined(__cplusplus)
}
#endif

#endif /* FLOWSDK_PAHO_MQTTASYNC_H */
