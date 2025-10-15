# Development

## Building and Testing
```bash
# Build everything
cargo build --workspace

# Build with protocol testing features (DANGEROUS - test only!)
cargo build --workspace --features protocol-testing

# Build individual workspaces
cargo build                            # Main library only
cd mqtt_grpc_duality && cargo build    # Proxy workspace only

# Run tests
cargo test --workspace                 # All tests
cargo test --lib                       # Library tests only
cargo test --features protocol-testing # With raw packet API tests
cd mqtt_grpc_duality && cargo test     # Proxy tests only

# Run specific test suites
cargo test tokio_async_client          # Async client tests
cargo test subscribe_command           # Subscribe builder tests  
cargo test mqtt_v5                     # MQTT v5 feature tests
cargo test --features protocol-testing raw_packet  # Raw packet tests

# Clean build artifacts
cargo clean --workspace                # Everything
cargo clean                            # Main library
cd mqtt_grpc_duality && cargo clean    # gRPC Proxy workspace
```