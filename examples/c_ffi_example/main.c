// SPDX-License-Identifier: MPL-2.0

#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <netdb.h>
#include <netinet/in.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>

// FFI declarations
typedef struct MqttEngineFFI MqttEngineFFI;

typedef struct {
  const char *client_id;
  uint8_t mqtt_version;
  uint8_t clean_start;
  uint16_t keep_alive;
  const char *username;
  const char *password;
  uint64_t reconnect_base_delay_ms;
  uint64_t reconnect_max_delay_ms;
  uint32_t max_reconnect_attempts;
} MqttOptionsC;

typedef struct MqttEventListFFI MqttEventListFFI;

MqttEngineFFI *mqtt_engine_new(const char *client_id, uint8_t mqtt_version);
MqttEngineFFI *mqtt_engine_new_with_opts(const MqttOptionsC *opts);
void mqtt_engine_free(MqttEngineFFI *ptr);
void mqtt_engine_connect(MqttEngineFFI *ptr);
void mqtt_engine_handle_incoming(MqttEngineFFI *ptr, const uint8_t *data,
                                 size_t len);
void mqtt_engine_handle_tick(MqttEngineFFI *ptr, uint64_t now_ms);
int64_t mqtt_engine_next_tick_ms(MqttEngineFFI *ptr);
uint8_t *mqtt_engine_take_outgoing(MqttEngineFFI *ptr, size_t *out_len);
void mqtt_engine_free_bytes(uint8_t *ptr, size_t len);
void mqtt_engine_free_string(char *ptr);
int32_t mqtt_engine_publish(MqttEngineFFI *ptr, const char *topic,
                            const uint8_t *payload, size_t payload_len,
                            uint8_t qos);
int32_t mqtt_engine_subscribe(MqttEngineFFI *ptr, const char *topic_filter,
                              uint8_t qos);
int32_t mqtt_engine_unsubscribe(MqttEngineFFI *ptr, const char *topic_filter);
void mqtt_engine_disconnect(MqttEngineFFI *ptr);
int mqtt_engine_is_connected(MqttEngineFFI *ptr);
uint8_t mqtt_engine_get_version(MqttEngineFFI *ptr);
void mqtt_engine_auth(MqttEngineFFI *ptr, uint8_t reason_code);
void mqtt_engine_handle_connection_lost(MqttEngineFFI *ptr);

// Native Event API
MqttEventListFFI *mqtt_engine_take_events_list(MqttEngineFFI *ptr);
void mqtt_event_list_free(MqttEventListFFI *ptr);
size_t mqtt_event_list_len(const MqttEventListFFI *ptr);
uint8_t mqtt_event_list_get_tag(const MqttEventListFFI *ptr, size_t index);
uint8_t mqtt_event_list_get_connected_rc(const MqttEventListFFI *ptr,
                                         size_t index);
char *mqtt_event_list_get_message_topic(const MqttEventListFFI *ptr,
                                        size_t index);
uint8_t *mqtt_event_list_get_message_payload(const MqttEventListFFI *ptr,
                                             size_t index, size_t *out_len);

// Helper to get monotonic time in milliseconds
uint64_t get_time_ms() {
  struct timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  return (uint64_t)ts.tv_sec * 1000 + (uint64_t)ts.tv_nsec / 1000000;
}

int main(int argc, char **argv) {
  const char *broker_host = "broker.emqx.io";
  const char *broker_port_str = "1883";

  if (argc > 1)
    broker_host = argv[1];
  if (argc > 2)
    broker_port_str = argv[2];

  printf("Connecting to MQTT broker at %s:%s...\n", broker_host,
         broker_port_str);

  // 1. Resolve Hostname and Connect
  struct addrinfo hints, *res, *p;
  memset(&hints, 0, sizeof(hints));
  hints.ai_family = AF_INET; // IPv4
  hints.ai_socktype = SOCK_STREAM;

  int status = getaddrinfo(broker_host, broker_port_str, &hints, &res);
  if (status != 0) {
    fprintf(stderr, "getaddrinfo: %s\n", gai_strerror(status));
    return 1;
  }

  int sock = -1;
  for (p = res; p != NULL; p = p->ai_next) {
    sock = socket(p->ai_family, p->ai_socktype, p->ai_protocol);
    if (sock < 0)
      continue;

    if (connect(sock, p->ai_addr, p->ai_addrlen) == 0)
      break;

    close(sock);
    sock = -1;
  }

  freeaddrinfo(res);

  if (sock < 0) {
    fprintf(stderr, "Could not connect to %s:%s\n", broker_host,
            broker_port_str);
    return 1;
  }

  // Set socket to non-blocking
  fcntl(sock, F_SETFL, O_NONBLOCK);
  printf("TCP Connection established.\n");

  // 2. Initialize Engine
  char client_id[32];
  snprintf(client_id, sizeof(client_id), "c_ffi_tcp_%u",
           (unsigned int)(get_time_ms() % 100000));

  MqttOptionsC opts = {0};
  opts.client_id = client_id;
  opts.mqtt_version = 5;
  opts.clean_start = 1;
  opts.keep_alive = 60;

  MqttEngineFFI *engine = mqtt_engine_new_with_opts(&opts);
  printf("Engine initialized (Version: %u, ClientID: %s).\n",
         mqtt_engine_get_version(engine), client_id);

  uint64_t start_time = get_time_ms();
  mqtt_engine_connect(engine);

  // 3. I/O Loop
  uint8_t read_buf[4096];
  int running = 1;
  uint32_t loop_without_activity = 0;
  int puback_received = 0;

  printf("Starting I/O loop...\n");
  while (running) {
    uint64_t now_ms = get_time_ms() - start_time;

    // A. Handle Ticks
    mqtt_engine_handle_tick(engine, now_ms);

    // B. Handle Outgoing
    size_t out_len = 0;
    uint8_t *out_bytes = mqtt_engine_take_outgoing(engine, &out_len);
    if (out_bytes) {
      ssize_t sent = send(sock, out_bytes, out_len, 0);
      if (sent < 0) {
        if (errno != EAGAIN && errno != EWOULDBLOCK) {
          perror("send");
          running = 0;
        }
      } else {
        printf("Sent %zd bytes to broker\n", sent);
        loop_without_activity = 0;
      }
      mqtt_engine_free_bytes(out_bytes, out_len);
    }

    // C. Handle Incoming
    ssize_t recvd = recv(sock, read_buf, sizeof(read_buf), 0);
    if (recvd > 0) {
      printf("Received %zd bytes from broker\n", recvd);
      mqtt_engine_handle_incoming(engine, read_buf, recvd);
      loop_without_activity = 0;
    } else if (recvd < 0) {
      if (errno != EAGAIN && errno != EWOULDBLOCK) {
        perror("recv");
        running = 0;
      }
    } else {
      printf("Broker closed connection\n");
      running = 0;
    }

    // D. Process Events (Native Structs)
    MqttEventListFFI *events = mqtt_engine_take_events_list(engine);
    if (events) {
      size_t len = mqtt_event_list_len(events);
      for (size_t i = 0; i < len; i++) {
        uint8_t tag = mqtt_event_list_get_tag(events, i);
        loop_without_activity = 0;

        if (tag == 1) { // Connected
          uint8_t rc = mqtt_event_list_get_connected_rc(events, i);
          printf("Connection acknowledged (RC: %u)! Subscribing...\n", rc);
          mqtt_engine_subscribe(engine, "test/topic/ffi", 1);
        } else if (tag == 5) { // Subscribed
          printf("Subscription acknowledged! Publishing QoS 1 message...\n");
          int32_t pid = mqtt_engine_publish(
              engine, "test/topic/ffi", (const uint8_t *)"hello from C native",
              19, 1);
          printf("Publish sent, PID: %d\n", pid);
        } else if (tag == 4) { // Published
          printf("Publish acknowledged by broker (PUBACK received)!\n");
          printf("Unsubscribing from test/topic/ffi...\n");
          mqtt_engine_unsubscribe(engine, "test/topic/ffi");
          printf("Disconnecting...\n");
          mqtt_engine_disconnect(engine);
        } else if (tag == 2) { // Disconnected
          printf("Gracefully disconnected from broker.\n");
          puback_received = 1; // Reuse this flag to exit loop
        } else if (tag == 3) { // MessageReceived
          char *topic = mqtt_event_list_get_message_topic(events, i);
          size_t p_len = 0;
          uint8_t *payload =
              mqtt_event_list_get_message_payload(events, i, &p_len);
          printf("Message received on topic %s: %.*s\n", topic, (int)p_len,
                 (char *)payload);
          mqtt_engine_free_string(topic);
          mqtt_engine_free_bytes(payload, p_len);
        } else if (tag == 8) { // Error
          printf("Engine Error!\n");
        }
      }
      mqtt_event_list_free(events);
    }

    // E. Sleep until next tick or small interval
    int64_t next_tick_ms = mqtt_engine_next_tick_ms(engine);
    uint32_t sleep_ms = 10;
    if (next_tick_ms >= 0) {
      uint64_t current_relative = get_time_ms() - start_time;
      if (next_tick_ms > (int64_t)current_relative) {
        uint64_t diff = next_tick_ms - current_relative;
        if (diff < sleep_ms)
          sleep_ms = (uint32_t)diff;
      } else {
        sleep_ms = 1;
      }
    }
    usleep(sleep_ms * 1000);

    // Limit loop for example safety
    if (puback_received) {
      printf("Success! QoS 1 message published and acknowledged. Exiting.\n");
      running = 0;
    }

    loop_without_activity++;
    if (loop_without_activity > 5000) { // 50 seconds
      printf("Timeout waiting for more events, exiting...\n");
      running = 0;
    }
  }

  // 4. Cleanup
  close(sock);
  mqtt_engine_free(engine);
  printf("Done.\n");

  return 0;
}
