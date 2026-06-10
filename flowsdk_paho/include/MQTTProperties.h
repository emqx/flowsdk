/*
 * SPDX-License-Identifier: MPL-2.0
 *
 * MQTTProperties.h — MQTT v5 property containers and API, implemented by FlowSDK.
 *
 * Drop-in compatible subset of Eclipse Paho's `MQTTProperties.h`. The struct
 * layouts (including the tagged-union `MQTTProperty.value`) match Paho exactly,
 * so programs that read or build v5 properties behave identically.
 */

#ifndef FLOWSDK_PAHO_MQTTPROPERTIES_H
#define FLOWSDK_PAHO_MQTTPROPERTIES_H

#if defined(__cplusplus)
extern "C" {
#endif

/* ─── Property identifier codes (MQTT 5.0 §2.2.2) ─────────────────────── */

enum MQTTPropertyCodes {
    MQTTPROPERTY_CODE_PAYLOAD_FORMAT_INDICATOR = 1,
    MQTTPROPERTY_CODE_MESSAGE_EXPIRY_INTERVAL = 2,
    MQTTPROPERTY_CODE_CONTENT_TYPE = 3,
    MQTTPROPERTY_CODE_RESPONSE_TOPIC = 8,
    MQTTPROPERTY_CODE_CORRELATION_DATA = 9,
    MQTTPROPERTY_CODE_SUBSCRIPTION_IDENTIFIER = 11,
    MQTTPROPERTY_CODE_SESSION_EXPIRY_INTERVAL = 17,
    MQTTPROPERTY_CODE_ASSIGNED_CLIENT_IDENTIFER = 18,
    MQTTPROPERTY_CODE_SERVER_KEEP_ALIVE = 19,
    MQTTPROPERTY_CODE_AUTHENTICATION_METHOD = 21,
    MQTTPROPERTY_CODE_AUTHENTICATION_DATA = 22,
    MQTTPROPERTY_CODE_REQUEST_PROBLEM_INFORMATION = 23,
    MQTTPROPERTY_CODE_WILL_DELAY_INTERVAL = 24,
    MQTTPROPERTY_CODE_REQUEST_RESPONSE_INFORMATION = 25,
    MQTTPROPERTY_CODE_RESPONSE_INFORMATION = 26,
    MQTTPROPERTY_CODE_SERVER_REFERENCE = 28,
    MQTTPROPERTY_CODE_REASON_STRING = 31,
    MQTTPROPERTY_CODE_RECEIVE_MAXIMUM = 33,
    MQTTPROPERTY_CODE_TOPIC_ALIAS_MAXIMUM = 34,
    MQTTPROPERTY_CODE_TOPIC_ALIAS = 35,
    MQTTPROPERTY_CODE_MAXIMUM_QOS = 36,
    MQTTPROPERTY_CODE_RETAIN_AVAILABLE = 37,
    MQTTPROPERTY_CODE_USER_PROPERTY = 38,
    MQTTPROPERTY_CODE_MAXIMUM_PACKET_SIZE = 39,
    MQTTPROPERTY_CODE_WILDCARD_SUBSCRIPTION_AVAILABLE = 40,
    MQTTPROPERTY_CODE_SUBSCRIPTION_IDENTIFIERS_AVAILABLE = 41,
    MQTTPROPERTY_CODE_SHARED_SUBSCRIPTION_AVAILABLE = 42
};

/** Returns a printable string description of an MQTT v5 property code. */
const char* MQTTPropertyName(int value);

/* ─── Property value types ────────────────────────────────────────────── */

enum MQTTPropertyTypes {
    MQTTPROPERTY_TYPE_BYTE,
    MQTTPROPERTY_TYPE_TWO_BYTE_INTEGER,
    MQTTPROPERTY_TYPE_FOUR_BYTE_INTEGER,
    MQTTPROPERTY_TYPE_VARIABLE_BYTE_INTEGER,
    MQTTPROPERTY_TYPE_BINARY_DATA,
    MQTTPROPERTY_TYPE_UTF_8_ENCODED_STRING,
    MQTTPROPERTY_TYPE_UTF_8_STRING_PAIR
};

/** Returns the value type (MQTTPROPERTY_TYPE_*) for a property code, or -1. */
int MQTTProperty_getType(int value);

/* ─── Property structs ────────────────────────────────────────────────── */

typedef struct {
    int len;     /**< Length of the data in bytes (not NUL-terminated) */
    char* data;  /**< Pointer to the data */
} MQTTLenString;

typedef struct {
    int identifier;  /**< The MQTT v5 property code (enum MQTTPropertyCodes) */
    union {
        unsigned char byte;       /**< single byte value */
        unsigned short integer2;  /**< two-byte integer value */
        unsigned int integer4;    /**< four-byte / variable-byte integer value */
        struct {
            MQTTLenString data;   /**< string value, or user-property key */
            MQTTLenString value;  /**< user-property value */
        };
    } value;
} MQTTProperty;

typedef struct MQTTProperties {
    int count;             /**< number of property entries in the array */
    int max_count;         /**< capacity of the array (entries) */
    int length;            /**< encoded byte length of all properties */
    MQTTProperty* array;   /**< array of properties */
} MQTTProperties;

#define MQTTProperties_initializer { 0, 0, 0, NULL }

/* ─── Property API ────────────────────────────────────────────────────── */

/** Returns the encoded byte length of all properties. */
int MQTTProperties_len(MQTTProperties* props);

/** Adds a property (deep-copying any string/binary data). Returns 0 on success. */
int MQTTProperties_add(MQTTProperties* props, const MQTTProperty* prop);

/** Frees the property array and any heap-backed data; resets counts. */
void MQTTProperties_free(MQTTProperties* props);

/** Returns 1 if a property with the given code is present, else 0. */
int MQTTProperties_hasProperty(MQTTProperties* props, int propid);

/** Returns the number of properties with the given code. */
int MQTTProperties_propertyCount(MQTTProperties* props, int propid);

/** Returns the numeric value of the first property with the given code,
 *  or -9999999 if not present / not numeric. */
int MQTTProperties_getNumericValue(MQTTProperties* props, int propid);

/** As getNumericValue but for the property at the given (per-code) index. */
int MQTTProperties_getNumericValueAt(MQTTProperties* props, int propid, int index);

/** Returns a pointer to the first property with the given code, or NULL. */
MQTTProperty* MQTTProperties_getProperty(MQTTProperties* props, int propid);

/** Returns a pointer to the property with the given code at `index`, or NULL. */
MQTTProperty* MQTTProperties_getPropertyAt(MQTTProperties* props, int propid, int index);

#if defined(__cplusplus)
}
#endif

#endif /* FLOWSDK_PAHO_MQTTPROPERTIES_H */
