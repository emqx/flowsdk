// SPDX-License-Identifier: MPL-2.0
use super::*;
#[cfg(feature = "quic")]
use flowsdk::mqtt_serde::control_packet::MqttPacket;
use flowsdk::mqtt_serde::mqttv5::common::properties::Property;
use flowsdk::mqtt_serde::parser::leveled::ParseLevel;

impl From<MqttParseLevelFFI> for ParseLevel {
    fn from(level: MqttParseLevelFFI) -> Self {
        match level {
            MqttParseLevelFFI::Full => Self::Full,
            MqttParseLevelFFI::HeadersParsed => Self::HeadersParsed,
            MqttParseLevelFFI::TypeOnly => Self::TypeOnly,
        }
    }
}

fn acknowledgement_properties(
    engine: &MqttEngine,
    kind: MqttAcknowledgementFFI,
    packet_id: u16,
    reason_code: u8,
    properties: Vec<MqttPropertyFFI>,
) -> Result<Vec<Property>, MqttErrorFFI> {
    if engine.options().auto_ack {
        return Err(properties::invalid(
            "Manual acknowledgements require auto_ack=False",
        ));
    }
    if packet_id == 0 {
        return Err(properties::invalid("Packet ID must be nonzero"));
    }
    let properties = properties::validate(properties, engine.mqtt_version(), |p| {
        matches!(
            p,
            MqttPropertyFFI::ReasonString { .. } | MqttPropertyFFI::UserProperty { .. }
        )
    })?;
    let valid = match kind {
        MqttAcknowledgementFFI::PubComp => matches!(reason_code, 0 | 0x92),
        _ => matches!(
            reason_code,
            0 | 0x10 | 0x80 | 0x83 | 0x87 | 0x90 | 0x91 | 0x97 | 0x99
        ),
    };
    if !valid || (engine.mqtt_version() != 5 && reason_code != 0) {
        return Err(properties::invalid(
            "Invalid acknowledgement reason code for this packet/version",
        ));
    }
    Ok(properties)
}

fn acknowledge_received(
    engine: &mut MqttEngine,
    kind: MqttAcknowledgementFFI,
    packet_id: u16,
    reason_code: u8,
    properties: Vec<MqttPropertyFFI>,
) -> Result<(), MqttErrorFFI> {
    let properties = acknowledgement_properties(engine, kind, packet_id, reason_code, properties)?;
    match kind {
        MqttAcknowledgementFFI::PubAck => engine.puback(packet_id, reason_code, properties),
        MqttAcknowledgementFFI::PubRec => engine.pubrec(packet_id, reason_code, properties),
        MqttAcknowledgementFFI::PubComp => engine.pubcomp(packet_id, reason_code, properties),
    }
    .map_err(Into::into)
}

#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
impl MqttEngineFFI {
    pub fn set_parse_level(&self, level: MqttParseLevelFFI) -> Result<(), MqttErrorFFI> {
        self.engine
            .lock()
            .unwrap()
            .try_set_parse_level(level.into())
            .map_err(Into::into)
    }

    pub fn acknowledge(
        &self,
        kind: MqttAcknowledgementFFI,
        packet_id: u16,
        reason_code: u8,
        properties: Vec<MqttPropertyFFI>,
        stream_id: Option<u64>,
    ) -> Result<(), MqttErrorFFI> {
        if stream_id.is_some() {
            return Err(properties::invalid("Stream IDs require QUIC"));
        }
        let mut engine = self.engine.lock().unwrap();
        acknowledge_received(&mut engine, kind, packet_id, reason_code, properties)
    }
}

#[cfg(feature = "tls")]
#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
impl TlsMqttEngineFFI {
    pub fn set_parse_level(&self, level: MqttParseLevelFFI) -> Result<(), MqttErrorFFI> {
        self.engine
            .lock()
            .unwrap()
            .engine_mut()
            .try_set_parse_level(level.into())
            .map_err(Into::into)
    }

    pub fn acknowledge(
        &self,
        kind: MqttAcknowledgementFFI,
        packet_id: u16,
        reason_code: u8,
        properties: Vec<MqttPropertyFFI>,
        stream_id: Option<u64>,
    ) -> Result<(), MqttErrorFFI> {
        if stream_id.is_some() {
            return Err(properties::invalid("Stream IDs require QUIC"));
        }
        let mut engine = self.engine.lock().unwrap();
        acknowledge_received(
            engine.engine_mut(),
            kind,
            packet_id,
            reason_code,
            properties,
        )
    }
}

#[cfg(feature = "quic")]
#[cfg_attr(feature = "uniffi-bindings", uniffi::export)]
impl QuicMqttEngineFFI {
    pub fn set_parse_level(&self, level: MqttParseLevelFFI) -> Result<(), MqttErrorFFI> {
        let mut engine = self.engine.lock().unwrap();
        engine.engine_mut().try_set_parse_level(level.into())?;
        engine.set_parse_level(level.into());
        Ok(())
    }

    pub fn acknowledge(
        &self,
        kind: MqttAcknowledgementFFI,
        packet_id: u16,
        reason_code: u8,
        properties: Vec<MqttPropertyFFI>,
        stream_id: Option<u64>,
    ) -> Result<(), MqttErrorFFI> {
        use flowsdk::mqtt_serde::{mqttv3, mqttv5};

        let mut engine = self.engine.lock().unwrap();
        let properties =
            acknowledgement_properties(engine.engine(), kind, packet_id, reason_code, properties)?;
        let packet = match (engine.engine().mqtt_version(), kind) {
            (5, MqttAcknowledgementFFI::PubAck) => MqttPacket::PubAck5(
                mqttv5::pubackv5::MqttPubAck::new(packet_id, reason_code, properties),
            ),
            (5, MqttAcknowledgementFFI::PubRec) => MqttPacket::PubRec5(
                mqttv5::pubrecv5::MqttPubRec::new(packet_id, reason_code, properties),
            ),
            (5, MqttAcknowledgementFFI::PubComp) => MqttPacket::PubComp5(
                mqttv5::pubcompv5::MqttPubComp::new(packet_id, reason_code, properties),
            ),
            (_, MqttAcknowledgementFFI::PubAck) => {
                MqttPacket::PubAck3(mqttv3::puback::MqttPubAck::new(packet_id))
            }
            (_, MqttAcknowledgementFFI::PubRec) => {
                MqttPacket::PubRec3(mqttv3::pubrec::MqttPubRec::new(packet_id))
            }
            (_, MqttAcknowledgementFFI::PubComp) => {
                MqttPacket::PubComp3(mqttv3::pubcomp::MqttPubComp::new(packet_id))
            }
        };
        let stream = stream_id
            .or_else(|| engine.control_stream_id())
            .ok_or_else(|| properties::invalid("QUIC has no control stream"))?;
        engine.acknowledge_on(stream, packet).map_err(Into::into)
    }
}
