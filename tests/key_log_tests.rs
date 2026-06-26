// SPDX-License-Identifier: MPL-2.0

//! Tests for TLS key logging (SSLKEYLOGFILE) support

/// Verify that QuicConfig builder wires enable_key_log into the built config.
#[cfg(feature = "quic")]
#[test]
fn quic_config_enable_key_log() {
    use flowsdk::mqtt_client::transport::quic::QuicConfig;

    let cfg = QuicConfig::builder().enable_key_log(true).build();
    assert!(cfg.enable_key_log);

    let cfg_default = QuicConfig::builder().build();
    assert!(!cfg_default.enable_key_log);
}

/// Verify that QuicConfig builder wires local_bind_addr into the built config.
#[cfg(feature = "quic")]
#[test]
fn quic_config_local_bind_addr() {
    use flowsdk::mqtt_client::transport::quic::QuicConfig;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let cfg = QuicConfig::builder().local_bind_addr(bind_addr).build();
    assert_eq!(cfg.local_bind_addr, Some(bind_addr));

    let cfg_from_ip = QuicConfig::builder()
        .local_bind_ip(IpAddr::V4(Ipv4Addr::LOCALHOST))
        .build();
    assert_eq!(cfg_from_ip.local_bind_addr, Some(bind_addr));

    let cfg_default = QuicConfig::builder().build();
    assert_eq!(cfg_default.local_bind_addr, None);
}

/// Verify that RustlsTlsConfig builder wires enable_key_log into the built config.
#[cfg(feature = "rustls-tls")]
#[test]
fn rustls_tls_config_enable_key_log() {
    use flowsdk::mqtt_client::transport::RustlsTlsConfig;

    let cfg = RustlsTlsConfig::builder().enable_key_log(true).build();
    assert!(cfg.enable_key_log);

    let cfg_default = RustlsTlsConfig::builder().build();
    assert!(!cfg_default.enable_key_log);
}

/// Verify that RustlsTlsConfig with key_log enabled produces a valid rustls ClientConfig.
/// We can't directly inspect the key_log field, but we confirm to_client_config() succeeds.
#[cfg(feature = "rustls-tls")]
#[test]
fn rustls_tls_config_key_log_builds_client_config() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

    use flowsdk::mqtt_client::transport::RustlsTlsConfig;

    let cfg = RustlsTlsConfig::builder()
        .use_system_roots(true)
        .enable_key_log(true)
        .build();

    let client_cfg = cfg.to_client_config();
    assert!(
        client_cfg.is_ok(),
        "to_client_config() should succeed with key_log enabled"
    );
}

/// Verify TokioAsyncClientConfig builder exposes quic_enable_key_log.
#[cfg(feature = "quic")]
#[test]
fn tokio_async_config_quic_key_log() {
    use flowsdk::mqtt_client::TokioAsyncClientConfig;

    let config = TokioAsyncClientConfig::builder()
        .quic_enable_key_log(true)
        .build();
    assert!(config.quic_enable_key_log);

    let config_default = TokioAsyncClientConfig::builder().build();
    assert!(!config_default.quic_enable_key_log);
}

/// Verify TokioAsyncClientConfig builder exposes quic_local_bind_addr.
#[cfg(feature = "quic")]
#[test]
fn tokio_async_config_quic_local_bind_addr() {
    use flowsdk::mqtt_client::TokioAsyncClientConfig;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let config = TokioAsyncClientConfig::builder()
        .quic_local_bind_addr(bind_addr)
        .build();
    assert_eq!(config.quic_local_bind_addr, Some(bind_addr));

    let config_default = TokioAsyncClientConfig::builder().build();
    assert_eq!(config_default.quic_local_bind_addr, None);
}

/// Verify TokioAsyncClientConfig builder exposes tls_enable_key_log.
#[cfg(feature = "rustls-tls")]
#[test]
fn tokio_async_config_tls_key_log() {
    use flowsdk::mqtt_client::TokioAsyncClientConfig;

    let config = TokioAsyncClientConfig::builder()
        .tls_enable_key_log(true)
        .build();
    assert!(config.tls_enable_key_log);

    let config_default = TokioAsyncClientConfig::builder().build();
    assert!(!config_default.tls_enable_key_log);
}
