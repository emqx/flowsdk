// SPDX-License-Identifier: MPL-2.0

use super::error::MqttClientError;
use crate::mqtt_serde::control_packet::MqttPacket;
use crate::{
    collections::Map,
    time::{DefaultTime, TimePoint},
};
use alloc::{collections::VecDeque, string::ToString, vec::Vec};
use core::time::Duration;

#[cfg(feature = "std")]
mod host;

/// An outstanding MQTT exchange. Desktop callers retain `Instant` timestamps.
#[derive(Debug, Clone)]
pub struct InflightEntry<T: TimePoint = DefaultTime> {
    pub packet_id: u16,
    pub packet: MqttPacket,
    pub sent_at: T,
    pub retry_count: u32,
    pub qos: u8,
    /// The originating logical stream, or None for an ordinary byte transport.
    pub stream: Option<u64>,
}

/// Inflight exchanges with receive-maximum accounting and ordered retransmission.
///
/// Lookups use a hash map on hosts and an O(log n) tree without `std`.
/// Replay order is admission order, independent of map order or equal timestamps.
/// MQTT 5 exchanges are replayed on reconnect, not on a retransmission timer.
pub struct InflightQueue<T: TimePoint = DefaultTime> {
    entries: Map<u16, InflightEntry<T>>,
    admission_order: VecDeque<u16>,
    deadline_queue: VecDeque<(u16, T)>,
    receive_maximum: u16,
    mqtt_version: u8,
    retransmission_timeout: Duration,
    publish_quota_used: usize,
    last_update: Option<T>,
}

impl<T: TimePoint> InflightQueue<T> {
    pub fn new(receive_maximum: u16, mqtt_version: u8, retransmission_timeout: Duration) -> Self {
        Self {
            entries: Map::new(),
            admission_order: VecDeque::new(),
            deadline_queue: VecDeque::new(),
            receive_maximum: if receive_maximum == 0 {
                u16::MAX
            } else {
                receive_maximum
            },
            mqtt_version,
            retransmission_timeout,
            publish_quota_used: 0,
            last_update: None,
        }
    }

    fn check_time(&self, now: T) -> Result<(), MqttClientError> {
        if self.last_update.is_some_and(|last| now < last)
            || (self.mqtt_version != 5 && now.checked_add(self.retransmission_timeout).is_none())
        {
            return Err(MqttClientError::InvalidConfiguration {
                field: "monotonic_time".into(),
                reason: "inflight timestamp moved backwards or its deadline overflowed".into(),
            });
        }
        Ok(())
    }

    pub fn can_push_publish(&self) -> bool {
        self.publish_quota_used < self.receive_maximum as usize
    }

    pub fn update_receive_maximum(&mut self, receive_maximum: u16) {
        if receive_maximum > 0 {
            self.receive_maximum = receive_maximum;
        }
    }

    pub fn push_at(
        &mut self,
        packet_id: u16,
        packet: MqttPacket,
        qos: u8,
        now: T,
    ) -> Result<(), MqttClientError> {
        self.push_with_stream_at(packet_id, packet, qos, None, now)
    }

    pub fn push_with_stream_at(
        &mut self,
        packet_id: u16,
        packet: MqttPacket,
        qos: u8,
        stream: Option<u64>,
        now: T,
    ) -> Result<(), MqttClientError> {
        self.check_time(now)?;
        if packet_id == 0 || self.entries.contains_key(&packet_id) {
            return Err(MqttClientError::InvalidPacketId { packet_id });
        }
        let publish = matches!(packet, MqttPacket::Publish5(_) | MqttPacket::Publish3(_));
        if publish && !self.can_push_publish() {
            return Err(MqttClientError::BufferFull {
                buffer_type: "inflight_publish".to_string(),
                capacity: self.receive_maximum as usize,
            });
        }
        if publish {
            self.publish_quota_used += 1;
        }
        self.entries.insert(
            packet_id,
            InflightEntry {
                packet_id,
                packet,
                sent_at: now,
                retry_count: 0,
                qos,
                stream,
            },
        );
        self.admission_order.push_back(packet_id);
        self.last_update = Some(now);
        self.refresh_deadline(packet_id, now);
        Ok(())
    }

    fn refresh_deadline(&mut self, packet_id: u16, now: T) {
        if self.mqtt_version != 5 {
            // Keep one live deadline per exchange, including PUBREL/resume.
            self.deadline_queue.retain(|(id, _)| *id != packet_id);
            let deadline = now
                .checked_add(self.retransmission_timeout)
                .expect("validated inflight deadline");
            self.deadline_queue.push_back((packet_id, deadline));
        }
    }

    pub fn get(&self, packet_id: u16) -> Option<&InflightEntry<T>> {
        self.entries.get(&packet_id)
    }
    pub fn contains(&self, packet_id: u16) -> bool {
        self.entries.contains_key(&packet_id)
    }

    pub fn transition_pubrel_at(
        &mut self,
        packet_id: u16,
        packet: MqttPacket,
        now: T,
    ) -> Result<(), MqttClientError> {
        self.check_time(now)?;
        if let Some(entry) = self.entries.get_mut(&packet_id) {
            entry.packet = packet;
            entry.sent_at = now;
            self.refresh_deadline(packet_id, now);
        }
        self.last_update = Some(now);
        Ok(())
    }

    pub fn resume_on_stream_at(
        &mut self,
        packet_id: u16,
        stream: u64,
        now: T,
    ) -> Result<(), MqttClientError> {
        self.check_time(now)?;
        if let Some(entry) = self.entries.get_mut(&packet_id) {
            entry.stream = Some(stream);
            entry.sent_at = now;
            self.refresh_deadline(packet_id, now);
        }
        self.last_update = Some(now);
        Ok(())
    }

    pub fn acknowledge(&mut self, packet_id: u16) -> Option<InflightEntry<T>> {
        let entry = self.entries.remove(&packet_id)?;
        self.admission_order.retain(|id| *id != packet_id);
        self.deadline_queue.retain(|(id, _)| *id != packet_id);
        if matches!(
            entry.packet,
            MqttPacket::Publish5(_)
                | MqttPacket::Publish3(_)
                | MqttPacket::PubRel5(_)
                | MqttPacket::PubRel3(_)
        ) {
            self.publish_quota_used = self.publish_quota_used.saturating_sub(1);
        }
        Some(entry)
    }

    pub fn get_expired_at(&mut self, now: T) -> Result<Vec<MqttPacket>, MqttClientError> {
        Ok(self
            .get_expired_with_stream_at(now)?
            .into_iter()
            .map(|(packet, _)| packet)
            .collect())
    }

    pub fn get_expired_with_stream_at(
        &mut self,
        now: T,
    ) -> Result<Vec<(MqttPacket, Option<u64>)>, MqttClientError> {
        self.check_time(now)?;
        self.last_update = Some(now);
        let mut expired = Vec::new();
        // Limit to the original entries: even a zero timeout retries once per tick.
        for _ in 0..self.deadline_queue.len() {
            let Some(&(id, deadline)) = self.deadline_queue.front() else {
                break;
            };
            if now < deadline {
                break;
            }
            self.deadline_queue.pop_front();
            if let Some(entry) = self.entries.get_mut(&id) {
                entry.retry_count = entry.retry_count.saturating_add(1);
                entry.sent_at = now;
                expired.push((entry.packet.clone(), entry.stream));
                self.refresh_deadline(id, now);
            }
        }
        Ok(expired)
    }

    pub fn get_all_for_reconnect(&self) -> Vec<MqttPacket> {
        self.admission_order
            .iter()
            .filter_map(|id| self.entries.get(id))
            .map(|entry| entry.packet.clone())
            .collect()
    }

    pub fn snapshot_for_reconnect(&self) -> Vec<(u16, MqttPacket)> {
        self.admission_order
            .iter()
            .filter_map(|id| self.entries.get(id))
            .map(|entry| (entry.packet_id, entry.packet.clone()))
            .collect()
    }

    pub fn next_expiration(&self) -> Option<T> {
        self.deadline_queue.front().map(|&(_, deadline)| deadline)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.admission_order.clear();
        self.deadline_queue.clear();
        self.publish_quota_used = 0;
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::mqtt_serde::mqttv5::publish::MqttPublish;
    use std::time::Instant;

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

    #[test]
    fn test_inflight_v5_deadline_queue_cleanup() {
        let mut q = InflightQueue::new(100, 5, Duration::from_secs(5));

        // For MQTT v5, deadline_queue should never be populated
        // Push and acknowledge many messages to simulate sustained load
        for i in 1..=50 {
            q.push(i, create_packet(i), 1).unwrap();
        }

        // Verify deadline_queue remains empty (never populated for v5)
        assert_eq!(q.deadline_queue.len(), 0);
        assert_eq!(q.len(), 50);

        // Acknowledge all messages
        for i in 1..=50 {
            q.acknowledge(i);
        }

        // After acknowledging all, entries should be empty
        assert_eq!(q.len(), 0);
        // deadline_queue should still be empty
        assert_eq!(q.deadline_queue.len(), 0);

        // Push and acknowledge more messages to verify it stays empty
        for i in 51..=100 {
            q.push(i, create_packet(i), 1).unwrap();
        }
        assert_eq!(q.deadline_queue.len(), 0);
        assert_eq!(q.len(), 50);

        for i in 51..=100 {
            q.acknowledge(i);
        }
        assert_eq!(q.len(), 0);
        assert_eq!(q.deadline_queue.len(), 0);

        // Verify get_expired() still works (returns empty for v5)
        for i in 101..=110 {
            q.push(i, create_packet(i), 1).unwrap();
        }

        let expired = q.get_expired(Instant::now());
        assert!(expired.is_empty()); // MQTT v5 doesn't retransmit
        assert_eq!(q.deadline_queue.len(), 0); // Still empty
        assert_eq!(q.len(), 10); // Unacknowledged entries remain
    }
}
