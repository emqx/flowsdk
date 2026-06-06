// SPDX-License-Identifier: MPL-2.0
//! Parse Paho-style server URIs into host:port pairs and transport type.
//!
//! Paho uses URI schemes like `tcp://host:port`, `ssl://host:port`, `ws://`, `wss://`.
//! FlowSDK uses `host:port` for TCP and `mqtts://host:port` for TLS.

/// Transport type determined from URI scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportType {
    /// Plain TCP connection
    Tcp,
    /// TLS-encrypted connection
    Tls,
}

/// Parsed server URI with transport type and host:port address.
#[derive(Debug, Clone)]
pub struct ParsedUri {
    /// The transport type (TCP or TLS)
    pub transport: TransportType,
    /// The host:port address (e.g., "broker.emqx.io:1883")
    pub addr: String,
    /// The hostname only (for TLS SNI)
    pub hostname: String,
    /// The port number
    pub port: u16,
}

/// Parse a Paho-style server URI.
///
/// Supported schemes:
/// - `tcp://host:port` → TCP
/// - `ssl://host:port` → TLS
/// - `mqtt://host:port` → TCP
/// - `mqtts://host:port` → TLS
/// - `host:port` (no scheme) → TCP
///
/// Returns `None` if the URI cannot be parsed.
pub fn parse_server_uri(uri: &str) -> Option<ParsedUri> {
    let (transport, addr_part) = if let Some(rest) = uri.strip_prefix("tcp://") {
        (TransportType::Tcp, rest)
    } else if let Some(rest) = uri.strip_prefix("ssl://") {
        (TransportType::Tls, rest)
    } else if let Some(rest) = uri.strip_prefix("mqtt://") {
        (TransportType::Tcp, rest)
    } else if let Some(rest) = uri.strip_prefix("mqtts://") {
        (TransportType::Tls, rest)
    } else {
        // No scheme — treat as TCP
        (TransportType::Tcp, uri)
    };

    // Parse host:port
    let (hostname, port) = parse_host_port(addr_part)?;
    let addr = format!("{}:{}", hostname, port);

    Some(ParsedUri {
        transport,
        addr,
        hostname: hostname.to_string(),
        port,
    })
}

/// Parse `host:port` string. Returns (hostname, port).
fn parse_host_port(addr: &str) -> Option<(&str, u16)> {
    // Handle IPv6: [::1]:1883
    if addr.starts_with('[') {
        let bracket_end = addr.find(']')?;
        let host = &addr[..bracket_end + 1];
        let rest = &addr[bracket_end + 1..];
        if let Some(port_str) = rest.strip_prefix(':') {
            let port = port_str.parse().ok()?;
            return Some((host, port));
        }
        // No port → default 1883
        return Some((host, 1883));
    }

    // Regular host:port
    if let Some(colon_pos) = addr.rfind(':') {
        let host = &addr[..colon_pos];
        let port_str = &addr[colon_pos + 1..];
        if let Ok(port) = port_str.parse::<u16>() {
            return Some((host, port));
        }
    }

    // No port → default 1883
    Some((addr, 1883))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tcp_scheme() {
        let parsed = parse_server_uri("tcp://broker.emqx.io:1883").unwrap();
        assert_eq!(parsed.transport, TransportType::Tcp);
        assert_eq!(parsed.addr, "broker.emqx.io:1883");
        assert_eq!(parsed.hostname, "broker.emqx.io");
        assert_eq!(parsed.port, 1883);
    }

    #[test]
    fn test_ssl_scheme() {
        let parsed = parse_server_uri("ssl://broker.emqx.io:8883").unwrap();
        assert_eq!(parsed.transport, TransportType::Tls);
        assert_eq!(parsed.addr, "broker.emqx.io:8883");
        assert_eq!(parsed.port, 8883);
    }

    #[test]
    fn test_no_scheme() {
        let parsed = parse_server_uri("broker.emqx.io:1883").unwrap();
        assert_eq!(parsed.transport, TransportType::Tcp);
        assert_eq!(parsed.addr, "broker.emqx.io:1883");
    }

    #[test]
    fn test_no_port() {
        let parsed = parse_server_uri("tcp://broker.emqx.io").unwrap();
        assert_eq!(parsed.port, 1883);
    }

    #[test]
    fn test_mqtt_scheme() {
        let parsed = parse_server_uri("mqtt://broker.emqx.io:1883").unwrap();
        assert_eq!(parsed.transport, TransportType::Tcp);
    }

    #[test]
    fn test_mqtts_scheme() {
        let parsed = parse_server_uri("mqtts://broker.emqx.io:8883").unwrap();
        assert_eq!(parsed.transport, TransportType::Tls);
    }
}
