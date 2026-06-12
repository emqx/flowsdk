<!-- SPDX-License-Identifier: MPL-2.0 -->
# flowsdk_paho — Eclipse Paho C drop-in over FlowSDK

`flowsdk_paho` implements the Eclipse Paho C MQTT client ABI on top of
FlowSDK's sans-I/O `MqttEngine`. Existing C/C++ programs written against the
Paho **synchronous** (`MQTTClient_*`) or **asynchronous** (`MQTTAsync_*`) API
can compile and link against this library unchanged.

```c
#include "MQTTClient.h"      /* same header names as Paho */
/* ... your existing Paho program ... */
```
```sh
cc -Idist/include app.c -Ldist/lib -lpaho-mqtt3c -o app   /* sync API  */
cc -Idist/include app.c -Ldist/lib -lpaho-mqtt3a -o app   /* async API */
```

## Architecture

- **Sans-I/O core.** The protocol state machine is FlowSDK's `MqttEngine`; this
  crate adds only the I/O and the C ABI. No `tokio` runtime — one client owns a
  dedicated `std::thread` I/O worker that drives a blocking `TcpStream` (or TLS
  stream) the same way the real Paho C library uses a pthread + blocking socket.
- **One handle, both APIs.** `MQTTClient` and `MQTTAsync` are the same opaque
  `PahoClientInner`. The sync API blocks the caller on a response channel until
  the matching ACK arrives; the async API returns immediately and the I/O
  thread fires the registered `onSuccess`/`onFailure`(`5`) callbacks.
- **Memory ownership** matches Paho: strings and messages handed to callbacks
  are `malloc`'d and freed with `MQTTClient_free` / `MQTTClient_freeMessage`
  (or the `MQTTAsync_*` equivalents).

## Implemented surface

| Area | Status |
|------|--------|
| Sync API `MQTTClient_*` | create/connect/publish/subscribe/receive/setCallbacks/waitForCompletion/yield/disconnect/destroy + helpers |
| Async API `MQTTAsync_*` | create/connect/send/subscribe/setCallbacks/setConnected/setDisconnected/waitForCompletion/disconnect/destroy + helpers |
| MQTT versions | 3.1, 3.1.1, 5.0 |
| QoS | 0, 1, 2 (full PUBREC/PUBREL/PUBCOMP) |
| MQTT v5 properties | `MQTTProperties` container + API; properties flow on connect/publish/subscribe and into v5 callbacks and arrived messages |
| TLS | `MQTTClient_SSLOptions` → `native-tls` (trust store, client identity, cert/hostname verification toggles, min protocol version) |

The exact exported ABI is pinned in
[`scripts/expected_symbols.txt`](scripts/expected_symbols.txt) (50 symbols) and
checked by [`scripts/check_symbols.sh`](scripts/check_symbols.sh).

## Build & package

```sh
# Build the library (cdylib + staticlib).
cargo build -p flowsdk_paho            # debug
cargo build -p flowsdk_paho --release  # optimized

# Stage a Paho-named drop-in distribution into flowsdk_paho/dist/.
sh scripts/package.sh --release
#   dist/include/   MQTTClient.h MQTTAsync.h MQTTProperties.h
#   dist/lib/       libpaho-mqtt3c.{dylib,so,a}  libpaho-mqtt3a.{dylib,so,a}
```

Both `libpaho-mqtt3c` and `libpaho-mqtt3a` are copies of the single FlowSDK
library, which implements both APIs together.

Cargo features: `tls` (default) pulls in `native-tls`; build with
`--no-default-features` for a TCP-only library.

## Tests

Rust unit tests (no network) cover URI parsing, the v5 property conversions and
C API, and the TLS connector construction:

```sh
cargo test -p flowsdk_paho
```

C smoke tests under [`tests/`](tests/) exercise the full client against a live
broker (`broker.emqx.io`). Build the library first, then:

```sh
make -C tests check_symbols   # audit the exported Paho symbol surface
make -C tests run             # sync: connect/sub/pub/recv round-trip
make -C tests run_async       # async callback path
make -C tests run_qos         # QoS 0/1/2 round-trip (incl. QoS2 handshake)
make -C tests run_v5props     # v5 user-property round-trip + onSuccess5
make -C tests run_tls         # ssl://broker.emqx.io:8883 with cert verification
```

## Known limitations

- Persistence: only `MQTTCLIENT_PERSISTENCE_NONE`; other types degrade
  gracefully (no on-disk message store).
- TLS: `enabledCipherSuites` and `CApath` are not expressible through
  `native-tls` and are ignored; encrypted (password-protected) private keys are
  not supported.
- MQTT v5 property serialization helpers (`MQTTProperties_read`/`_write`/
  `_copy`) are not exported — wire encoding is handled by the engine.
- WebSocket transports (`ws://`, `wss://`) and automatic reconnect are not
  implemented.
