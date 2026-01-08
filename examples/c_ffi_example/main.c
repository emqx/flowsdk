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

MqttEngineFFI *mqtt_engine_new(const char *client_id, uint8_t mqtt_version);
void mqtt_engine_free(MqttEngineFFI *ptr);
void mqtt_engine_connect(MqttEngineFFI *ptr);
void mqtt_engine_handle_incoming(MqttEngineFFI *ptr, const uint8_t *data,
                                 size_t len);
void mqtt_engine_handle_tick(MqttEngineFFI *ptr, uint64_t now_ms);
int64_t mqtt_engine_next_tick_ms(MqttEngineFFI *ptr);
uint8_t *mqtt_engine_take_outgoing(MqttEngineFFI *ptr, size_t *out_len);
void mqtt_engine_free_bytes(uint8_t *ptr, size_t len);
char *mqtt_engine_take_events(MqttEngineFFI *ptr);
void mqtt_engine_free_string(char *ptr);
int32_t mqtt_engine_publish(MqttEngineFFI *ptr, const char *topic,
                            const uint8_t *payload, size_t payload_len,
                            uint8_t qos);

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
  MqttEngineFFI *engine = mqtt_engine_new("c_tcp_client", 5);
  uint64_t start_time = get_time_ms();
  mqtt_engine_connect(engine);

  // 3. I/O Loop
  uint8_t read_buf[4096];
  int running = 1;
  uint32_t loop_without_activity = 0;
  int published = 0;
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

    // D. Process Events
    char *events = mqtt_engine_take_events(engine);
    if (events) {
      if (strcmp(events, "[]") != 0) {
        printf("Events: %s\n", events);

        if (strstr(events, "Connected") && !published) {
          printf("Connection acknowledged! Publishing QoS 1 message...\n");
          int32_t pid =
              mqtt_engine_publish(engine, "test/topic/ffi",
                                  (const uint8_t *)"hello qos1 from C", 17, 1);
          printf("Publish sent, PID: %d\n", pid);
          published = 1;
        }

        if (strstr(events, "Published") && published) {
          printf("Publish acknowledged by broker (PUBACK received)!\n");
          puback_received = 1;
        }
      }
      mqtt_engine_free_string(events);
      loop_without_activity = 0;
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
