/*
 * SPDX-License-Identifier: MPL-2.0
 *
 * MQTTClient.h — Paho C synchronous API, implemented by FlowSDK.
 *
 * This header is a drop-in compatible subset of the Eclipse Paho C
 * `MQTTClient.h`. Programs written against Paho's synchronous API can be
 * compiled and linked against the FlowSDK implementation (libflowsdk_paho).
 */

#ifndef FLOWSDK_PAHO_MQTTCLIENT_H
#define FLOWSDK_PAHO_MQTTCLIENT_H

#include "MQTTProperties.h"

#if defined(__cplusplus)
extern "C" {
#endif

/* ─── Return codes ──────────────────────────────────────────────────── */

#define MQTTCLIENT_SUCCESS 0
#define MQTTCLIENT_FAILURE -1
#define MQTTCLIENT_DISCONNECTED -3
#define MQTTCLIENT_MAX_MESSAGES_INFLIGHT -4
#define MQTTCLIENT_BAD_UTF8_STRING -5
#define MQTTCLIENT_NULL_PARAMETER -6
#define MQTTCLIENT_TOPICNAME_TRUNCATED -7
#define MQTTCLIENT_BAD_STRUCTURE -8
#define MQTTCLIENT_BAD_QOS -9
#define MQTTCLIENT_SSL_NOT_SUPPORTED -10
#define MQTTCLIENT_BAD_MQTT_VERSION -11
#define MQTTCLIENT_BAD_PROTOCOL -14
#define MQTTCLIENT_BAD_MQTT_OPTION -15
#define MQTTCLIENT_WRONG_MQTT_VERSION -16
#define MQTTCLIENT_0_LEN_WILL_TOPIC -17

/* ─── MQTT protocol versions ────────────────────────────────────────── */

#define MQTTVERSION_DEFAULT 0
#define MQTTVERSION_3_1 3
#define MQTTVERSION_3_1_1 4
#define MQTTVERSION_5 5

/* ─── Persistence types ─────────────────────────────────────────────── */

#define MQTTCLIENT_PERSISTENCE_DEFAULT 0
#define MQTTCLIENT_PERSISTENCE_NONE 1
#define MQTTCLIENT_PERSISTENCE_USER 2

/* ─── Handle and token types ────────────────────────────────────────── */

typedef void* MQTTClient;
typedef int MQTTClient_deliveryToken;
typedef int MQTTClient_token;

/* ─── MQTTClient_message ────────────────────────────────────────────── */

typedef struct MQTTClient_message
{
    char struct_id[4];      /**< Must be "MQTM" */
    int struct_version;     /**< Must be 0 or 1 */
    int payloadlen;         /**< The length of the payload in bytes */
    void* payload;          /**< A pointer to the payload bytes */
    int qos;                /**< The quality of service (QoS) 0, 1 or 2 */
    int retained;           /**< The retained flag */
    int dup;                /**< The dup (duplicate) flag */
    int msgid;              /**< The message identifier */
    MQTTProperties properties; /**< The MQTT v5 properties (struct_version >= 1) */
} MQTTClient_message;

#define MQTTClient_message_initializer \
    { {'M', 'Q', 'T', 'M'}, 1, 0, NULL, 0, 0, 0, 0, MQTTProperties_initializer }

/* ─── MQTTClient_willOptions ────────────────────────────────────────── */

typedef struct MQTTClient_willOptions
{
    char struct_id[4];        /**< Must be "MQTW" */
    int struct_version;       /**< Must be 0 or 1 */
    const char* topicName;    /**< The LWT topic to which the message is published */
    const char* message;      /**< The LWT payload (null-terminated string, version 0) */
    int retained;             /**< The retained flag for the LWT message */
    int qos;                  /**< The QoS for the LWT message */
    struct
    {
        int len;              /**< Binary payload length (version 1) */
        const void* data;     /**< Binary payload data (version 1) */
    } payload;
} MQTTClient_willOptions;

#define MQTTClient_willOptions_initializer { {'M', 'Q', 'T', 'W'}, 1, NULL, NULL, 0, 0, {0, NULL} }

/* ─── MQTTClient_SSLOptions ─────────────────────────────────────────── */

typedef struct MQTTClient_SSLOptions
{
    char struct_id[4];                  /**< Must be "MQTS" */
    int struct_version;                 /**< Struct version */
    const char* trustStore;             /**< CA certificate file (PEM) */
    const char* keyStore;               /**< Client certificate chain file (PEM) */
    const char* privateKey;             /**< Client private key file (PEM) */
    const char* privateKeyPassword;     /**< Password for the private key */
    const char* enabledCipherSuites;    /**< Colon-separated cipher list */
    int enableServerCertAuth;           /**< Verify the server certificate */
    int sslVersion;                     /**< SSL/TLS version */
    int verify;                         /**< Post-connect hostname check */
    const char* CApath;                 /**< CA certificate directory */
} MQTTClient_SSLOptions;

#define MQTTClient_SSLOptions_initializer { {'M', 'Q', 'T', 'S'}, 2, NULL, NULL, NULL, NULL, NULL, 1, 0, 0, NULL }

/* ─── MQTTClient_connectOptions ─────────────────────────────────────── */

typedef struct MQTTClient_nameValue
{
    const char* name;
    const char* value;
} MQTTClient_nameValue;

typedef struct MQTTClient_connectOptions
{
    char struct_id[4];                  /**< Must be "MQTC" */
    int struct_version;                 /**< Struct version (0-8) */
    int keepAliveInterval;              /**< Keep alive interval in seconds */
    int cleansession;                   /**< Clean session flag (MQTT 3.1.1) */
    int reliable;                       /**< Reliable publishing */
    MQTTClient_willOptions* will;       /**< Will (LWT) options, or NULL */
    const char* username;               /**< Username for authentication */
    const char* password;               /**< Password for authentication */
    int connectTimeout;                 /**< Connect timeout in seconds */
    int retryInterval;                  /**< Retry interval in seconds */
    MQTTClient_SSLOptions* ssl;         /**< SSL/TLS options, or NULL */
    int serverURIcount;                 /**< Number of server URIs */
    char* const* serverURIs;            /**< Array of server URIs */
    int MQTTVersion;                    /**< MQTT protocol version */
    struct
    {
        const char* serverURI;          /**< The connected server URI (version >= 3) */
        int MQTTVersion;                /**< The MQTT version used (version >= 3) */
        int sessionPresent;             /**< Session present flag (version >= 3) */
    } returned;
    struct
    {
        int len;                        /**< Binary password length (version >= 4) */
        const void* data;               /**< Binary password data (version >= 4) */
    } binarypwd;
    int maxInflightMessages;            /**< Max inflight messages (version >= 5) */
    int cleanstart;                     /**< Clean start flag (MQTT 5, version >= 6) */
    const MQTTClient_nameValue* httpHeaders; /**< HTTP headers (version >= 7) */
    const char* httpProxy;              /**< HTTP proxy (version >= 7) */
    const char* httpsProxy;             /**< HTTPS proxy (version >= 7) */
} MQTTClient_connectOptions;

#define MQTTClient_connectOptions_initializer \
    { {'M', 'Q', 'T', 'C'}, 8, 60, 1, 1, NULL, NULL, NULL, 30, 0, NULL, 0, NULL, \
      MQTTVERSION_DEFAULT, {NULL, 0, 0}, {0, NULL}, 0, 0, NULL, NULL, NULL }

/* ─── Callback function types ───────────────────────────────────────── */

typedef void MQTTClient_connectionLost(void* context, char* cause);
typedef int MQTTClient_messageArrived(void* context, char* topicName, int topicLen,
                                      MQTTClient_message* message);
typedef void MQTTClient_deliveryComplete(void* context, MQTTClient_deliveryToken dt);

/* ─── Lifecycle ─────────────────────────────────────────────────────── */

int MQTTClient_create(MQTTClient* handle, const char* serverURI, const char* clientId,
                      int persistence_type, void* persistence_context);

void MQTTClient_destroy(MQTTClient* handle);

/* ─── Connection ────────────────────────────────────────────────────── */

int MQTTClient_connect(MQTTClient handle, MQTTClient_connectOptions* options);

int MQTTClient_disconnect(MQTTClient handle, int timeout);

int MQTTClient_isConnected(MQTTClient handle);

/* ─── Publish ───────────────────────────────────────────────────────── */

int MQTTClient_publish(MQTTClient handle, const char* topicName, int payloadlen,
                       const void* payload, int qos, int retained,
                       MQTTClient_deliveryToken* dt);

int MQTTClient_publishMessage(MQTTClient handle, const char* topicName,
                              MQTTClient_message* msg, MQTTClient_deliveryToken* dt);

/* ─── Subscribe ─────────────────────────────────────────────────────── */

int MQTTClient_subscribe(MQTTClient handle, const char* topic, int qos);

int MQTTClient_subscribeMany(MQTTClient handle, int count, char* const* topic, int* qos);

int MQTTClient_unsubscribe(MQTTClient handle, const char* topic);

int MQTTClient_unsubscribeMany(MQTTClient handle, int count, char* const* topic);

/* ─── Receive / callbacks ───────────────────────────────────────────── */

int MQTTClient_setCallbacks(MQTTClient handle, void* context,
                            MQTTClient_connectionLost* cl,
                            MQTTClient_messageArrived* ma,
                            MQTTClient_deliveryComplete* dc);

int MQTTClient_receive(MQTTClient handle, char** topicName, int* topicLen,
                       MQTTClient_message** message, unsigned long timeout);

int MQTTClient_waitForCompletion(MQTTClient handle, MQTTClient_deliveryToken dt,
                                 unsigned long timeout);

int MQTTClient_getPendingDeliveryTokens(MQTTClient handle,
                                        MQTTClient_deliveryToken** tokens);

void MQTTClient_yield(void);

/* ─── Utilities ─────────────────────────────────────────────────────── */

const char* MQTTClient_strerror(int code);

void MQTTClient_free(void* ptr);

void MQTTClient_freeMessage(MQTTClient_message** msg);

#if defined(__cplusplus)
}
#endif

#endif /* FLOWSDK_PAHO_MQTTCLIENT_H */
