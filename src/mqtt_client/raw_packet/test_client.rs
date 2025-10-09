//! Raw TCP client for protocol testing
//!
//! ⚠️ **DANGER**: This module provides direct TCP access for sending malformed packets.
//! Only use for protocol compliance testing. **DO NOT use in production.**

use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};

/// Raw TCP client for sending and receiving malformed MQTT packets
///
/// This client bypasses all MQTT protocol handling and provides direct TCP access.
/// Use it to test server behavior with malformed packets, reserved bits, protocol violations, etc.
///
/// # Example
///
/// ```no_run
/// # use flowsdk::mqtt_client::raw_packet::test_client::RawTestClient;
/// # use flowsdk::mqtt_client::raw_packet::RawPacketBuilder;
/// # use flowsdk::mqtt_serde::control_packet::MqttPacket;
/// # use flowsdk::mqtt_serde::mqttv5::connectv5::MqttConnect;
/// # async fn example() -> std::io::Result<()> {
/// let mut client = RawTestClient::connect("localhost:1883").await?;
///
/// // Send malformed CONNECT with reserved bit set
/// let connect = MqttConnect::default();
/// let mut builder = RawPacketBuilder::from_packet(
///     MqttPacket::Connect5(connect)
/// )?;
/// builder.set_fixed_header_flags(0x01); // MQTT-2.1.3-1 violation
///
/// client.send_raw(builder.build()).await?;
///
/// // Wait for CONNACK or disconnection
/// match client.receive_raw(8192, 5000).await {
///     Ok(response) => println!("Received: {:?}", response),
///     Err(e) => println!("Broker disconnected: {}", e),
/// }
/// # Ok(())
/// # }
/// ```
pub struct RawTestClient {
    stream: TcpStream,
}

impl RawTestClient {
    /// Connect to an MQTT broker using raw TCP
    ///
    /// # Arguments
    ///
    /// * `addr` - Broker address (e.g., "localhost:1883")
    ///
    /// # Errors
    ///
    /// Returns error if connection fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::test_client::RawTestClient;
    /// # async fn example() -> std::io::Result<()> {
    /// let mut client = RawTestClient::connect("localhost:1883").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn connect(addr: &str) -> io::Result<Self> {
        let stream = TcpStream::connect(addr).await?;
        Ok(Self { stream })
    }

    /// Send raw bytes to the broker
    ///
    /// # Arguments
    ///
    /// * `bytes` - Raw packet bytes to send
    ///
    /// # Errors
    ///
    /// Returns error if send fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::test_client::RawTestClient;
    /// # async fn example() -> std::io::Result<()> {
    /// # let mut client = RawTestClient::connect("localhost:1883").await?;
    /// // Send PINGREQ
    /// client.send_raw(vec![0xC0, 0x00]).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_raw(&mut self, bytes: Vec<u8>) -> io::Result<()> {
        self.stream.write_all(&bytes).await?;
        self.stream.flush().await?;
        Ok(())
    }

    /// Receive raw bytes from the broker with timeout
    ///
    /// # Arguments
    ///
    /// * `max_size` - Maximum number of bytes to read
    /// * `timeout_ms` - Timeout in milliseconds
    ///
    /// # Returns
    ///
    /// Raw bytes received from broker
    ///
    /// # Errors
    ///
    /// Returns error if read fails, times out, or connection closes.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::test_client::RawTestClient;
    /// # async fn example() -> std::io::Result<()> {
    /// # let mut client = RawTestClient::connect("localhost:1883").await?;
    /// # client.send_raw(vec![0xC0, 0x00]).await?;
    /// let response = client.receive_raw(8192, 5000).await?;
    /// println!("Received {} bytes", response.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn receive_raw(&mut self, max_size: usize, timeout_ms: u64) -> io::Result<Vec<u8>> {
        let read_future = async {
            let mut buffer = vec![0u8; max_size];
            let n = self.stream.read(&mut buffer).await?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    "Connection closed by broker",
                ));
            }
            buffer.truncate(n);
            Ok(buffer)
        };

        match timeout(Duration::from_millis(timeout_ms), read_future).await {
            Ok(result) => result,
            Err(_) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("Read timed out after {} ms", timeout_ms),
            )),
        }
    }

    /// Send raw packet and expect immediate disconnection
    ///
    /// This is useful for testing that the broker correctly disconnects
    /// when receiving malformed packets.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Raw packet bytes to send
    /// * `timeout_ms` - Timeout in milliseconds to wait for disconnection
    ///
    /// # Returns
    ///
    /// `Ok(())` if broker disconnects, error otherwise
    ///
    /// # Errors
    ///
    /// Returns error if broker doesn't disconnect or if send fails.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::test_client::RawTestClient;
    /// # async fn example() -> std::io::Result<()> {
    /// # let mut client = RawTestClient::connect("localhost:1883").await?;
    /// // Send reserved packet type (MQTT-2.1.2-1 violation)
    /// client.send_expect_disconnect(vec![0xF0, 0x00], 5000).await?;
    /// println!("Broker correctly disconnected");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_expect_disconnect(
        &mut self,
        bytes: Vec<u8>,
        timeout_ms: u64,
    ) -> io::Result<()> {
        // Send the malformed packet
        self.send_raw(bytes).await?;

        // Wait for disconnection
        let result = timeout(Duration::from_millis(timeout_ms), async {
            loop {
                // Try to read from connection
                match self.receive_raw(1, 100).await {
                    Err(e) if e.kind() == io::ErrorKind::ConnectionReset => {
                        return Ok::<(), io::Error>(());
                    }
                    Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                        // Keep waiting
                        continue;
                    }
                    Err(e) => return Err(e),
                    Ok(_) => {
                        // Connection still alive, keep waiting
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        })
        .await;

        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("Broker did not disconnect within {} ms", timeout_ms),
            )),
        }
    }

    /// Send raw packet and read response
    ///
    /// Convenience method that combines send and receive.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Raw packet bytes to send
    /// * `max_response_size` - Maximum response size
    /// * `timeout_ms` - Timeout in milliseconds
    ///
    /// # Returns
    ///
    /// Raw response bytes from broker
    ///
    /// # Errors
    ///
    /// Returns error if send/receive fails or times out.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::test_client::RawTestClient;
    /// # async fn example() -> std::io::Result<()> {
    /// # let mut client = RawTestClient::connect("localhost:1883").await?;
    /// // Send PINGREQ and get PINGRESP
    /// let response = client.send_and_receive(
    ///     vec![0xC0, 0x00],
    ///     8192,
    ///     5000
    /// ).await?;
    /// assert_eq!(response, vec![0xD0, 0x00]); // PINGRESP
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_and_receive(
        &mut self,
        bytes: Vec<u8>,
        max_response_size: usize,
        timeout_ms: u64,
    ) -> io::Result<Vec<u8>> {
        self.send_raw(bytes).await?;
        self.receive_raw(max_response_size, timeout_ms).await
    }

    /// Check if connection is still alive
    ///
    /// # Returns
    ///
    /// `true` if connection is alive, `false` otherwise
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::test_client::RawTestClient;
    /// # async fn example() -> std::io::Result<()> {
    /// # let mut client = RawTestClient::connect("localhost:1883").await?;
    /// if client.is_connected().await {
    ///     println!("Still connected");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn is_connected(&mut self) -> bool {
        // Try to peek at the stream
        let mut buf = [0u8; 1];
        matches!(
            timeout(Duration::from_millis(10), self.stream.peek(&mut buf)).await,
            Ok(Ok(_))
        )
    }

    /// Close the connection
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use flowsdk::mqtt_client::raw_packet::test_client::RawTestClient;
    /// # async fn example() -> std::io::Result<()> {
    /// # let mut client = RawTestClient::connect("localhost:1883").await?;
    /// client.close().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn close(mut self) -> io::Result<()> {
        self.stream.shutdown().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires running MQTT broker
    async fn test_raw_client_connect() {
        let result = RawTestClient::connect("localhost:1883").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[ignore] // Requires running MQTT broker
    async fn test_raw_client_send_receive() {
        let mut client = RawTestClient::connect("localhost:1883").await.unwrap();

        // Send PINGREQ
        client.send_raw(vec![0xC0, 0x00]).await.unwrap();

        // Receive PINGRESP
        let response = client.receive_raw(8192, 5000).await.unwrap();
        assert_eq!(response, vec![0xD0, 0x00]);
    }
}
