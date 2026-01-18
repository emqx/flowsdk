// SPDX-License-Identifier: MPL-2.0

#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <netdb.h>
#include <netinet/in.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>

// FFI declarations
typedef struct TlsMqttEngineFFI TlsMqttEngineFFI;
typedef struct MqttEventListFFI MqttEventListFFI;

typedef struct {
  const char *ca_cert_file;
  const char *client_cert_file;
  const char *client_key_file;
  uint8_t insecure_skip_verify;
} MqttTlsOptionsC;

TlsMqttEngineFFI *mqtt_tls_engine_new(const char *client_id,
                                      uint8_t mqtt_version,
                                      const char *server_name,
                                      const MqttTlsOptionsC *opts);
void mqtt_tls_engine_free(TlsMqttEngineFFI *ptr);
void mqtt_tls_engine_connect(TlsMqttEngineFFI *ptr);
void mqtt_tls_engine_handle_socket_data(TlsMqttEngineFFI *ptr,
                                        const uint8_t *data, size_t len);
uint8_t *mqtt_tls_engine_take_socket_data(TlsMqttEngineFFI *ptr,
                                          size_t *out_len);
void mqtt_tls_engine_handle_tick(TlsMqttEngineFFI *ptr, uint64_t now_ms);
int32_t mqtt_tls_engine_publish(TlsMqttEngineFFI *ptr, const char *topic,
                                const uint8_t *payload, size_t payload_len,
                                uint8_t qos);
int32_t mqtt_tls_engine_subscribe(TlsMqttEngineFFI *ptr,
                                  const char *topic_filter, uint8_t qos);
int32_t mqtt_tls_engine_unsubscribe(TlsMqttEngineFFI *ptr,
                                    const char *topic_filter);
void mqtt_tls_engine_disconnect(TlsMqttEngineFFI *ptr);
int32_t mqtt_tls_engine_is_connected(TlsMqttEngineFFI *ptr);

void mqtt_engine_free_bytes(uint8_t *ptr, size_t len);
void mqtt_engine_free_string(char *ptr);

// Native Event API
MqttEventListFFI *mqtt_tls_engine_take_events_list(TlsMqttEngineFFI *ptr);
void mqtt_event_list_free(MqttEventListFFI *ptr);
size_t mqtt_event_list_len(const MqttEventListFFI *ptr);
uint8_t mqtt_event_list_get_tag(const MqttEventListFFI *ptr, size_t index);
uint8_t mqtt_event_list_get_connected_rc(const MqttEventListFFI *ptr,
                                         size_t index);
char *mqtt_event_list_get_message_topic(const MqttEventListFFI *ptr,
                                        size_t index);
uint8_t *mqtt_event_list_get_message_payload(const MqttEventListFFI *ptr,
                                             size_t index, size_t *out_len);

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
   * - Unchecked return values for some system calls (send, recv).
   * - Minimal error handling and resource cleanup on failure paths.
   */
  const char *broker_host = "broker.emqx.io";
  const char *broker_port = "8883";

  if (argc > 1)
    broker_host = argv[1];
  if (argc > 2)
    broker_port = argv[2];

  printf("Resolving %s:%s...\n", broker_host, broker_port);

  struct addrinfo hints, *res;
  memset(&hints, 0, sizeof(hints));
  hints.ai_family = AF_INET;
  hints.ai_socktype = SOCK_STREAM;

  int status = getaddrinfo(broker_host, broker_port, &hints, &res);
  if (status != 0) {
    fprintf(stderr, "getaddrinfo: %s\n", gai_strerror(status));
    return 1;
  }

  int sock = socket(res->ai_family, res->ai_socktype, res->ai_protocol);
  if (sock < 0) {
    perror("socket");
    return 1;
  }

  if (connect(sock, res->ai_addr, res->ai_addrlen) < 0) {
    perror("connect");
    return 1;
  }
  printf("TCP connected to %s:%s\n", broker_host, broker_port);
  freeaddrinfo(res);

  fcntl(sock, F_SETFL, O_NONBLOCK);

  // Initialize TLS Engine
  MqttTlsOptionsC q_opts = {0};
  q_opts.insecure_skip_verify = 1;

  char client_id[32];
  snprintf(client_id, sizeof(client_id), "c_ffi_tls_%u",
           (unsigned int)(get_time_ms() % 100000));

  TlsMqttEngineFFI *engine =
      mqtt_tls_engine_new(client_id, 5, broker_host, &q_opts);
  if (!engine) {
    fprintf(stderr, "Failed to create TLS engine\n");
    return 1;
  }
  printf("Engine initialized (ClientID: %s).\n", client_id);

  mqtt_tls_engine_connect(engine);

  uint64_t start_time = get_time_ms();
  int running = 1;
  uint32_t loop_without_activity = 0;

  uint8_t read_buf[4096];

  printf("Starting TLS I/O loop...\n");
  while (running) {
    uint64_t now_ms = get_time_ms() - start_time;

    // 1. Tick
    mqtt_tls_engine_handle_tick(engine, now_ms);

    // 2. Take outgoing TLS data to socket
    size_t out_len = 0;
    uint8_t *out_data = mqtt_tls_engine_take_socket_data(engine, &out_len);
    if (out_data) {
      send(sock, out_data, out_len, 0);
      mqtt_engine_free_bytes(out_data, out_len);
      loop_without_activity = 0;
    }

    // 3. Read incoming TLS data from socket
    ssize_t recvd = recv(sock, read_buf, sizeof(read_buf), 0);
    if (recvd > 0) {
      mqtt_tls_engine_handle_socket_data(engine, read_buf, recvd);
      loop_without_activity = 0;
    } else if (recvd == 0) {
      printf("Connection closed by peer\n");
      break;
    }

    // 4. Events (Native C API)
    MqttEventListFFI *events = mqtt_tls_engine_take_events_list(engine);
    if (events) {
      size_t len = mqtt_event_list_len(events);
      for (size_t i = 0; i < len; i++) {
        uint8_t tag = mqtt_event_list_get_tag(events, i);
        loop_without_activity = 0;

        if (tag == 1) { // Connected
          printf("TLS Connected! Subscribing...\n");
          mqtt_tls_engine_subscribe(engine, "test/topic/tls", 1);
        } else if (tag == 5) { // Subscribed
          printf("Subscribed! Publishing...\n");
          const char *payload = "hello from C over TLS native";
          mqtt_tls_engine_publish(engine, "test/topic/tls",
                                  (const uint8_t *)payload, strlen(payload), 1);
        } else if (tag == 3) { // MessageReceived
          char *topic = mqtt_event_list_get_message_topic(events, i);
          size_t p_len = 0;
          uint8_t *payload =
              mqtt_event_list_get_message_payload(events, i, &p_len);
          printf("Message received on topic %s: %.*s\n", topic, (int)p_len,
                 (char *)payload);
          mqtt_engine_free_string(topic);
          mqtt_engine_free_bytes(payload, p_len);

          printf("Unsubscribing from test/topic/tls...\n");
          mqtt_tls_engine_unsubscribe(engine, "test/topic/tls");
        } else if (tag == 6) { // Unsubscribed
          printf("Unsubscribed! Disconnecting...\n");
          mqtt_tls_engine_disconnect(engine);
        } else if (tag == 2) { // Disconnected
          printf("Disconnected gracefully.\n");
          running = 0;
        } else if (tag == 4) { // Published
          printf("Publish acknowledged.\n");
        } else if (tag == 8) { // Error
          printf("TLS Engine Error!\n");
        } else {
          printf("Unhandled event tag: %u\n", tag);
        }
      }
      mqtt_event_list_free(events);
    }

    usleep(10000);
    loop_without_activity++;
    if (loop_without_activity > 2000) {
      printf("No activity for a while, exiting...\n");
      break;
    }
  } // end of while(running)

  mqtt_tls_engine_free(engine);
  close(sock);
  printf("Done.\n");

  return 0;
}
