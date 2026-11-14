use domain::Metric;

use crate::{DecodeError, Reading};

/// 16-bit Bluetooth service UUID that carries BTHome v2 service data.
pub const SERVICE_UUID: u16 = 0xFCD2;

const ENCRYPTED_FLAG: u8 = 0b1;
const SUPPORTED_VERSION: u8 = 2;

#[derive(Debug, Clone, PartialEq)]
pub struct Packet {
    pub packet_id: Option<u8>,
    pub readings: Vec<Reading>,
}

impl Packet {
    pub fn parse(data: &[u8]) -> Result<Self, DecodeError> {
        let (&info, mut rest) = data.split_first().ok_or(DecodeError::Truncated)?;
        if info & ENCRYPTED_FLAG != 0 {
            return Err(DecodeError::Encrypted);
        }
        let version = info >> 5;
        if version != SUPPORTED_VERSION {
            return Err(DecodeError::UnsupportedVersion(version));
        }
        let mut packet = Self {
            packet_id: None,
            readings: Vec::new(),
        };
        while let Some((&id, tail)) = rest.split_first() {
            let size = object_size(id)?;
            let (value, tail) = tail.split_at_checked(size).ok_or(DecodeError::Truncated)?;
            packet.apply(id, value);
            rest = tail;
        }
        Ok(packet)
    }

    /// Records one object. `value` already has the length `object_size` gave
    /// for `id`; skipped objects fall through the last arm.
    fn apply(&mut self, id: u8, value: &[u8]) {
        let mut push = |metric, value| self.readings.push(Reading { metric, value });
        match *value {
            [id_byte] if id == 0x00 => self.packet_id = Some(id_byte),
            [percent] if id == 0x01 => push(Metric::Battery, f64::from(percent)),
            [lo, hi] if id == 0x02 => {
                push(
                    Metric::Temperature,
                    f64::from(i16::from_le_bytes([lo, hi])) / 100.0,
                );
            }
            [lo, hi] if id == 0x03 => {
                push(
                    Metric::Humidity,
                    f64::from(u16::from_le_bytes([lo, hi])) / 100.0,
                );
            }
            _ => {}
        }
    }
}

fn object_size(id: u8) -> Result<usize, DecodeError> {
    match id {
        0x00 | 0x01 | 0x10 => Ok(1),
        0x02 | 0x03 | 0x0C => Ok(2),
        other => Err(DecodeError::UnsupportedObject(other)),
    }
}

#[cfg(test)]
mod tests {
    use domain::Metric;

    use crate::{DecodeError, Packet};

    fn value_of(packet: &Packet, metric: Metric) -> Option<f64> {
        packet
            .readings
            .iter()
            .find(|reading| reading.metric == metric)
            .map(|reading| reading.value)
    }

    fn assert_close(actual: Option<f64>, expected: f64) {
        let actual = actual.expect("metric present");
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    #[test]
    fn parses_a_real_packet() {
        let packet = Packet::parse(&[
            0x40, 0x00, 0x83, 0x01, 0x58, 0x02, 0x4f, 0x08, 0x03, 0x1d, 0x14,
        ])
        .expect("valid packet");
        assert_eq!(packet.packet_id, Some(0x83));
        assert_close(value_of(&packet, Metric::Battery), 88.0);
        assert_close(value_of(&packet, Metric::Temperature), 21.27);
        assert_close(value_of(&packet, Metric::Humidity), 51.49);
    }

    #[test]
    fn negative_temperature_is_signed() {
        let packet = Packet::parse(&[0x40, 0x02, 0xec, 0xff]).expect("valid packet");
        assert_close(value_of(&packet, Metric::Temperature), -0.20);
        assert_eq!(packet.packet_id, None);
    }

    #[test]
    fn encrypted_payload_is_rejected() {
        assert_eq!(
            Packet::parse(&[0x41, 0x00, 0x01]),
            Err(DecodeError::Encrypted)
        );
    }

    #[test]
    fn other_versions_are_rejected() {
        assert_eq!(
            Packet::parse(&[0x20, 0x00, 0x01]),
            Err(DecodeError::UnsupportedVersion(1))
        );
    }

    #[test]
    fn truncated_payloads_are_rejected() {
        assert_eq!(Packet::parse(&[]), Err(DecodeError::Truncated));
        assert_eq!(
            Packet::parse(&[0x40, 0x02, 0x4f]),
            Err(DecodeError::Truncated)
        );
        assert_eq!(Packet::parse(&[0x40, 0x01]), Err(DecodeError::Truncated));
    }

    #[test]
    fn unknown_objects_are_rejected() {
        assert_eq!(
            Packet::parse(&[0x40, 0x05, 0x00]),
            Err(DecodeError::UnsupportedObject(0x05))
        );
    }

    #[test]
    fn known_but_unused_objects_are_skipped() {
        let packet =
            Packet::parse(&[0x40, 0x0c, 0x01, 0x02, 0x10, 0x01, 0x01, 0x58]).expect("valid packet");
        assert_eq!(packet.readings.len(), 1);
        assert_close(value_of(&packet, Metric::Battery), 88.0);
    }
}
