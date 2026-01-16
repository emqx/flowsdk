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
  uint8_t insecure_skip_verify;
  const char *alpn;
  const char *ca_cert_file;
  const char *client_cert_file;
  const char *client_key_file;
} MqttTlsOptionsFFI;

typedef struct {
  char *remote_addr;
  uint8_t *data;
  size_t len;
} MqttDatagramFFI;

QuicMqttEngineFFI *mqtt_quic_engine_new(const char *client_id,
                                        uint8_t mqtt_version);
void mqtt_quic_engine_free(QuicMqttEngineFFI *ptr);
int32_t mqtt_quic_engine_connect(QuicMqttEngineFFI *ptr,
                                 const char *server_addr,
                                 const char *server_name,
                                 const MqttTlsOptionsFFI *opts);
void mqtt_quic_engine_handle_datagram(QuicMqttEngineFFI *ptr,
                                      const uint8_t *data, size_t len,
                                      const char *remote_addr);
void mqtt_quic_engine_handle_tick(QuicMqttEngineFFI *ptr, uint64_t now_ms);
MqttDatagramFFI *
mqtt_quic_engine_take_outgoing_datagrams(QuicMqttEngineFFI *ptr,
                                         size_t *out_count);
void mqtt_quic_engine_free_datagrams(MqttDatagramFFI *datagrams, size_t count);
char *mqtt_quic_engine_take_events(QuicMqttEngineFFI *ptr);
void mqtt_engine_free_string(char *ptr);
int32_t mqtt_quic_engine_publish(QuicMqttEngineFFI *ptr, const char *topic,
                                 const uint8_t *payload, size_t payload_len,
                                 uint8_t qos);
int32_t mqtt_quic_engine_subscribe(QuicMqttEngineFFI *ptr,
                                   const char *topic_filter, uint8_t qos);
int32_t mqtt_quic_engine_unsubscribe(QuicMqttEngineFFI *ptr,
                                     const char *topic_filter);
void mqtt_quic_engine_disconnect(QuicMqttEngineFFI *ptr);
int mqtt_quic_engine_is_connected(QuicMqttEngineFFI *ptr);

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
  QuicMqttEngineFFI *engine = mqtt_quic_engine_new("c_quic_client", 5);
  if (!engine) {
    fprintf(stderr, "Failed to create QUIC engine\n");
    return 1;
  }

  MqttTlsOptionsFFI q_opts = {0};
  q_opts.insecure_skip_verify =
      1; // For testing coverage, set to 0 should work as well.
  q_opts.alpn = "mqtt";

  if (mqtt_quic_engine_connect(engine, server_addr_str, broker_host, &q_opts) !=
      0) {
    fprintf(stderr, "Failed to initiate QUIC connection\n");
    return 1;
  }

  uint64_t start_time = get_time_ms();
  int running = 1;
  int subscribed = 0;
  int published = 0;
  int disconnected = 0;
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
    MqttDatagramFFI *datagrams =
        mqtt_quic_engine_take_outgoing_datagrams(engine, &dg_count);
    if (datagrams) {
      for (size_t i = 0; i < dg_count; i++) {
        struct addrinfo hints, *res;
        memset(&hints, 0, sizeof(hints));
        hints.ai_family = AF_INET;
        hints.ai_socktype = SOCK_DGRAM;

        char host[256], port[16];
        char *colon = strrchr(datagrams[i].remote_addr, ':');
        if (colon) {
          size_t host_len = colon - datagrams[i].remote_addr;
          strncpy(host, datagrams[i].remote_addr, host_len);
          host[host_len] = '\0';
          strcpy(port, colon + 1);

          if (getaddrinfo(host, port, &hints, &res) == 0) {
            sendto(sock, datagrams[i].data, datagrams[i].len, 0, res->ai_addr,
                   res->ai_addrlen);
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

    // D. Process Events
    char *events = mqtt_quic_engine_take_events(engine);
    if (events) {
      if (strcmp(events, "[]") != 0) {
        printf("Events: %s\n", events);

        if (strstr(events, "Connected") && !subscribed) {
          printf("QUIC Connection established! Subscribing...\n");
          mqtt_quic_engine_subscribe(engine, "test/topic/quic", 1);
          subscribed = 1;
        }

        if (strstr(events, "Subscribed") && !published) {
          printf("Subscribed! Publishing...\n");
          mqtt_quic_engine_publish(engine, "test/topic/quic",
                                   (const uint8_t *)"hello from C over QUIC",
                                   22, 1);
          published = 1;
        }

        if (strstr(events, "Published") && !disconnected) {
          printf("Published! Disconnecting...\n");
          mqtt_quic_engine_disconnect(engine);
          disconnected = 1;
        }

        if (disconnected && strstr(events, "Disconnected")) {
          printf("Disconnected gracefully.\n");
          running = 0;
        }
      }
      mqtt_engine_free_string(events);
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
