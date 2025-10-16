# Contributing

## Areas for Contribution

1. **Core MQTT Protocol**: Changes go in `src/mqtt_serde/`
2. **MQTT Client**: Enhancements to `src/mqtt_client/`
3. **gRPC Conversions**: Shared logic in `src/grpc_conversions.rs` and `mqtt_grpc_duality/src/lib.rs`
4. **Proxy Applications**: New features in `mqtt_grpc_duality/src/bin/`
5. **Examples**: Simple demos in `src/bin/`
6. **Protocol Tests**: Add compliance tests in `tests/protocol_compliance_tests.rs`
7. **Documentation**: Update docs in `docs/` directory

## Development Workflow

1. Fork the repository
2. Create a feature branch
3. Make changes with tests
4. Ensure all tests pass: `cargo test --workspace`
5. Run formatter: `cargo fmt --all`
6. Run clippy: `cargo clippy --workspace -- -D warnings`
7. Submit pull request

## Testing Guidelines

- Add unit tests for new functionality
- Add integration tests for client operations
- Use `#[ignore]` for tests requiring live broker
- Document protocol violations in raw packet tests
- Keep test coverage above 80%