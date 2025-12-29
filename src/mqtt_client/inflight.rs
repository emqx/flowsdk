use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use super::error::MqttClientError;
use crate::mqtt_serde::control_packet::MqttPacket;

/// Entry in the inflight queue
#[derive(Debug, Clone)]
pub struct InflightEntry {
    /// The packet identifier
    pub packet_id: u16,
    /// The original packet (for retransmission)
    pub packet: MqttPacket,
    /// Timestamp when the packet was first sent
    pub sent_at: Instant,
    /// Number of times the packet has been sent
    pub retry_count: u32,
    /// QoS level (1 or 2)
    pub qos: u8,
}

/// A queue for managing inflight QoS 1 and QoS 2 messages.
///
/// Strictly follows MQTT 3.1.1 and 5.0 specifications:
/// - O(1) lookups by PacketId.
/// - O(1) timeout checks using a deadline queue.
/// - Enforces `Receive Maximum` flow control.
/// - Handles retransmission rules (MQTT 3.1.1 resends, MQTT 5.0 waits for reconnect).
pub struct InflightQueue {
    /// Inflight entries keyed by packet identifier
    entries: HashMap<u16, InflightEntry>,
    /// Queue for efficient timeout checking (PacketId, SentAt)
    deadline_queue: VecDeque<(u16, Instant)>,
    /// Maximum number of inflight messages allowed (MQTT v5 Receive Maximum)
    receive_maximum: u16,
    /// MQTT version to determine retransmission rules
    mqtt_version: u8,
    /// Default retransmission timeout
    retransmission_timeout: Duration,
    /// Current number of PUBLISH packets in the queue (for flow control)
    publish_count: usize,
}

impl InflightQueue {
    pub fn new(receive_maximum: u16, mqtt_version: u8, retransmission_timeout: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            deadline_queue: VecDeque::new(),
            receive_maximum: if receive_maximum == 0 {
                65535
            } else {
                receive_maximum
            },
            mqtt_version,
            retransmission_timeout,
            publish_count: 0,
        }
    }

    /// Check if another PUBLISH message can be sent
    pub fn can_push_publish(&self) -> bool {
        self.publish_count < self.receive_maximum as usize
    }

    /// Update the receive maximum limit (e.g., from CONNACK)
    pub fn update_receive_maximum(&mut self, receive_maximum: u16) {
        if receive_maximum > 0 {
            self.receive_maximum = receive_maximum;
        }
    }

    /// Push a message into the inflight queue
    pub fn push(
        &mut self,
        packet_id: u16,
        packet: MqttPacket,
        qos: u8,
    ) -> Result<(), MqttClientError> {
        // Validation for PUBLISH should happen in MqttEngine before calling push,
        // but we keep a check here as a safety measure.
        if matches!(packet, MqttPacket::Publish5(_) | MqttPacket::Publish3(_))
            && !self.can_push_publish()
        {
            return Err(MqttClientError::BufferFull {
                buffer_type: "inflight_publish".to_string(),
                capacity: self.receive_maximum as usize,
            });
        }

        let now = Instant::now();
        if matches!(packet, MqttPacket::Publish5(_) | MqttPacket::Publish3(_)) {
            self.publish_count += 1;
        }

        let entry = InflightEntry {
            packet_id,
            packet,
            sent_at: now,
            retry_count: 0,
            qos,
        };

        self.entries.insert(packet_id, entry);
        self.deadline_queue.push_back((packet_id, now));
        Ok(())
    }

    /// Acknowledge a message (PUBACK, PUBREL, etc.)
    pub fn acknowledge(&mut self, packet_id: u16) -> Option<InflightEntry> {
        if let Some(entry) = self.entries.remove(&packet_id) {
            if matches!(
                entry.packet,
                MqttPacket::Publish5(_) | MqttPacket::Publish3(_)
            ) {
                self.publish_count -= 1;
            }
            Some(entry)
        } else {
            None
        }
        // Note: deadline_queue cleanup happens lazily during get_expired
    }

    /// Get all expired messages that need retransmission (MQTT 3.1.1 only)
    pub fn get_expired(&mut self, now: Instant) -> Vec<MqttPacket> {
        let mut expired = Vec::new();

        // MQTT v5.0: MUST NOT retransmit while connection is active
        if self.mqtt_version == 5 {
            return expired;
        }

        while let Some(&(pid, sent_at)) = self.deadline_queue.front() {
            if now.duration_since(sent_at) < self.retransmission_timeout {
                break;
            }

            self.deadline_queue.pop_front();

            if let Some(entry) = self.entries.get_mut(&pid) {
                // If the entry timestamp matches, it's truly expired and not yet acknowledged
                if entry.sent_at == sent_at {
                    entry.retry_count += 1;
                    entry.sent_at = now;
                    expired.push(entry.packet.clone());
                    // Re-queue for next timeout
                    self.deadline_queue.push_back((pid, now));
                }
            }
        }

        expired
    }

    /// Get all messages to be re-sent upon reconnection (MQTT 3.1.1 and 5.0 with CleanStart=0)
    pub fn get_all_for_reconnect(&self) -> Vec<MqttPacket> {
        let mut all = self.entries.values().collect::<Vec<_>>();
        // Sort by retry_count or original sent time if needed, but here we just send
        all.sort_by_key(|e| e.sent_at);
        all.into_iter().map(|e| e.packet.clone()).collect()
    }

    /// Get the earliest expiration time for any inflight message
    pub fn next_expiration(&self) -> Option<Instant> {
        // MQTT v5.0: MUST NOT retransmit while connection is active
        if self.mqtt_version == 5 {
            return None;
        }

        self.deadline_queue
            .front()
            .map(|&(_, sent_at)| sent_at + self.retransmission_timeout)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.deadline_queue.clear();
        self.publish_count = 0;
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mqtt_serde::mqttv5::publish::MqttPublish;

    fn create_packet(pid: u16) -> MqttPacket {
        MqttPacket::Publish5(MqttPublish {
            dup: false,
            qos: 1,
            retain: false,
            topic_name: "test".to_string(),
            packet_id: Some(pid),
            payload: vec![],
            properties: vec![],
        })
    }

    #[test]
    fn test_inflight_push_ack() {
        let mut q = InflightQueue::new(10, 5, Duration::from_secs(5));
        assert!(q.can_push_publish());
        q.push(1, create_packet(1), 1).unwrap();
        assert_eq!(q.len(), 1);
        q.acknowledge(1).unwrap();
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn test_inflight_receive_maximum() {
        let mut q = InflightQueue::new(2, 5, Duration::from_secs(5));
        q.push(1, create_packet(1), 1).unwrap();
        q.push(2, create_packet(2), 1).unwrap();
        assert!(!q.can_push_publish());
        assert!(q.push(3, create_packet(3), 1).is_err());

        // Update limit
        q.update_receive_maximum(5);
        assert!(q.can_push_publish());
        q.push(3, create_packet(3), 1).unwrap();
        assert_eq!(q.len(), 3);
    }

    #[test]
    fn test_inflight_v3_retransmission() {
        let mut q = InflightQueue::new(10, 3, Duration::from_secs(1));
        let start = Instant::now();
        q.push(1, create_packet(1), 1).unwrap();

        let expired = q.get_expired(start + Duration::from_secs(2));
        assert_eq!(expired.len(), 1);
        assert_eq!(q.len(), 1);

        // Should be queued again
        let expired2 = q.get_expired(start + Duration::from_secs(4));
        assert_eq!(expired2.len(), 1);
    }

    #[test]
    fn test_inflight_v5_no_retransmission() {
        let mut q = InflightQueue::new(10, 5, Duration::from_secs(1));
        let start = Instant::now();
        q.push(1, create_packet(1), 1).unwrap();

        let expired = q.get_expired(start + Duration::from_secs(2));
        assert!(expired.is_empty());
    }
}
