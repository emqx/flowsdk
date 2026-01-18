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

// FFI declarations for QUIC
typedef struct QuicMqttEngineFFI QuicMqttEngineFFI;

typedef struct {
  const char *ca_cert_file;
  const char *client_cert_file;
  const char *client_key_file;
  uint8_t insecure_skip_verify;
} MqttTlsOptionsC;

typedef struct {
  char *addr;
  uint8_t *data;
  size_t data_len;
} MqttDatagramC;

typedef struct MqttEventListFFI MqttEventListFFI;

QuicMqttEngineFFI *mqtt_quic_engine_new(const char *client_id,
                                        uint8_t mqtt_version);
void mqtt_quic_engine_free(QuicMqttEngineFFI *ptr);
int32_t mqtt_quic_engine_connect(QuicMqttEngineFFI *ptr,
                                 const char *server_addr,
                                 const char *server_name,
                                 const MqttTlsOptionsC *opts);
void mqtt_quic_engine_handle_datagram(QuicMqttEngineFFI *ptr,
                                      const uint8_t *data, size_t len,
                                      const char *remote_addr);
void mqtt_quic_engine_handle_tick(QuicMqttEngineFFI *ptr, uint64_t now_ms);
MqttDatagramC *mqtt_quic_engine_take_outgoing_datagrams(QuicMqttEngineFFI *ptr,
                                                        size_t *out_count);
void mqtt_quic_engine_free_datagrams(MqttDatagramC *datagrams, size_t count);
void mqtt_engine_free_string(char *ptr);
void mqtt_engine_free_bytes(uint8_t *ptr, size_t len);
int32_t mqtt_quic_engine_publish(QuicMqttEngineFFI *ptr, const char *topic,
                                 const uint8_t *payload, size_t payload_len,
                                 uint8_t qos);
int32_t mqtt_quic_engine_subscribe(QuicMqttEngineFFI *ptr,
                                   const char *topic_filter, uint8_t qos);
int32_t mqtt_quic_engine_unsubscribe(QuicMqttEngineFFI *ptr,
                                     const char *topic_filter);
void mqtt_quic_engine_disconnect(QuicMqttEngineFFI *ptr);
int mqtt_quic_engine_is_connected(QuicMqttEngineFFI *ptr);

// Native Event API
MqttEventListFFI *mqtt_quic_engine_take_events_list(QuicMqttEngineFFI *ptr);
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
  /*
   * NOTE: This is an example to demonstrate the FFI usage.
   * To keep it simple and readable, some production-grade robustness checks are
   * omitted:
   * - Unchecked return values for some system calls (sendto, fcntl, recvfrom).
   * - Minimal error handling and resource cleanup on failure paths.
   * - Simplified string handling (potential buffer overflows if misused).
   */
  const char *broker_host = "broker.emqx.io";
  const char *broker_port = "14567";

  if (argc > 1)
    broker_host = argv[1];
  if (argc > 2)
    broker_port = argv[2];

  printf("Resolving %s:%s...\n", broker_host, broker_port);

  // 1. Resolve Hostname
  struct addrinfo hints, *res;
  memset(&hints, 0, sizeof(hints));
  hints.ai_family = AF_INET; // IPv4
  hints.ai_socktype = SOCK_DGRAM;

  int status = getaddrinfo(broker_host, broker_port, &hints, &res);
  if (status != 0) {
    fprintf(stderr, "getaddrinfo: %s\n", gai_strerror(status));
    return 1;
  }

  char server_addr_str[256];
  struct sockaddr_in *addr_in = (struct sockaddr_in *)res->ai_addr;
  char ip_str[INET_ADDRSTRLEN];
  inet_ntop(AF_INET, &(addr_in->sin_addr), ip_str, INET_ADDRSTRLEN);
  snprintf(server_addr_str, sizeof(server_addr_str), "%s:%u", ip_str,
           ntohs(addr_in->sin_port));
  freeaddrinfo(res);

  printf("Connecting to MQTT-over-QUIC broker at %s (resolved from %s)...\n",
         server_addr_str, broker_host);

  // 2. Create UDP socket
  int sock = socket(AF_INET, SOCK_DGRAM, 0);
  if (sock < 0) {
    perror("socket");
    return 1;
  }

  // Set to non-blocking
  fcntl(sock, F_SETFL, O_NONBLOCK);

  // 2. Initialize QUIC Engine
  char client_id[32];
  snprintf(client_id, sizeof(client_id), "c_ffi_quic_%u",
           (unsigned int)(get_time_ms() % 100000));

  QuicMqttEngineFFI *engine = mqtt_quic_engine_new(client_id, 5);
  if (!engine) {
    fprintf(stderr, "Failed to create QUIC engine\n");
    return 1;
  }
  printf("Engine initialized (ClientID: %s).\n", client_id);

  MqttTlsOptionsC q_opts = {0};
  q_opts.insecure_skip_verify = 1;

  if (mqtt_quic_engine_connect(engine, server_addr_str, broker_host, &q_opts) !=
      0) {
    fprintf(stderr, "Failed to initiate QUIC connection\n");
    return 1;
  }

  uint64_t start_time = get_time_ms();
  int running = 1;
  uint32_t loop_without_activity = 0;

  uint8_t read_buf[2048];
  struct sockaddr_in remote_addr;
  socklen_t addr_len = sizeof(remote_addr);

  printf("Starting QUIC I/O loop...\n");
  while (running) {
    uint64_t now_ms = get_time_ms() - start_time;

    // A. Handle Ticks
    mqtt_quic_engine_handle_tick(engine, now_ms);

    // B. Handle Outgoing Datagrams
    size_t dg_count = 0;
    MqttDatagramC *datagrams =
        mqtt_quic_engine_take_outgoing_datagrams(engine, &dg_count);
    if (datagrams) {
      for (size_t i = 0; i < dg_count; i++) {
        struct addrinfo hints, *res;
        memset(&hints, 0, sizeof(hints));
        hints.ai_family = AF_INET;
        hints.ai_socktype = SOCK_DGRAM;

        char host[256], port[16];
        char *colon = strrchr(datagrams[i].addr, ':');
        if (colon) {
          size_t host_len = colon - datagrams[i].addr;
          strncpy(host, datagrams[i].addr, host_len);
          host[host_len] = '\0';
          strcpy(port, colon + 1);

          if (getaddrinfo(host, port, &hints, &res) == 0) {
            sendto(sock, datagrams[i].data, datagrams[i].data_len, 0,
                   res->ai_addr, res->ai_addrlen);
            freeaddrinfo(res);
          }
        }
      }
      mqtt_quic_engine_free_datagrams(datagrams, dg_count);
      loop_without_activity = 0;
    }

    // C. Handle Incoming Datagrams
    ssize_t recvd = recvfrom(sock, read_buf, sizeof(read_buf), 0,
                             (struct sockaddr *)&remote_addr, &addr_len);
    if (recvd > 0) {
      char remote_ip[INET_ADDRSTRLEN];
      inet_ntop(AF_INET, &remote_addr.sin_addr, remote_ip, sizeof(remote_ip));
      char remote_str[INET_ADDRSTRLEN + 16];
      snprintf(remote_str, sizeof(remote_str), "%s:%u", remote_ip,
               ntohs(remote_addr.sin_port));

      mqtt_quic_engine_handle_datagram(engine, read_buf, recvd, remote_str);
      loop_without_activity = 0;
    }

    // D. Process Events (Native Structs)
    MqttEventListFFI *events = mqtt_quic_engine_take_events_list(engine);
    if (events) {
      size_t len = mqtt_event_list_len(events);
      for (size_t i = 0; i < len; i++) {
        uint8_t tag = mqtt_event_list_get_tag(events, i);
        loop_without_activity = 0;

        if (tag == 1) { // Connected
          printf("QUIC Connection established! Subscribing...\n");
          mqtt_quic_engine_subscribe(engine, "test/topic/quic", 1);
        } else if (tag == 5) { // Subscribed
          printf("Subscribed! Publishing...\n");
          mqtt_quic_engine_publish(
              engine, "test/topic/quic",
              (const uint8_t *)"hello from C over QUIC native", 29, 1);
        } else if (tag == 4) { // Published
          printf("Published! Disconnecting...\n");
          mqtt_quic_engine_disconnect(engine);
        } else if (tag == 2) { // Disconnected
          printf("Disconnected gracefully.\n");
          running = 0;
        } else if (tag == 3) { // MessageReceived
          char *topic = mqtt_event_list_get_message_topic(events, i);
          size_t p_len = 0;
          uint8_t *payload =
              mqtt_event_list_get_message_payload(events, i, &p_len);
          printf("Message received on topic %s: %.*s\n", topic, (int)p_len,
                 (char *)payload);
          mqtt_engine_free_string(topic);
          mqtt_engine_free_bytes(payload, p_len);
        }
      }
      mqtt_event_list_free(events);
    }

    usleep(10000); // 10ms
    loop_without_activity++;
    if (loop_without_activity > 5000) {
      printf("Timeout, exiting...\n");
      running = 0;
    }
  }

  // 4. Cleanup
  mqtt_quic_engine_free(engine);
  close(sock);
  printf("Done.\n");

  return 0;
}
