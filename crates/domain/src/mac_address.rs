use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::DomainError;

/// A 48-bit Bluetooth device address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct MacAddress([u8; 6]);

impl MacAddress {
    pub const fn new(octets: [u8; 6]) -> Self {
        Self(octets)
    }
}

impl FromStr for MacAddress {
    type Err = DomainError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || DomainError::InvalidMacAddress(text.to_owned());
        let mut octets = [0u8; 6];
        let mut parts = text.split(':');
        for octet in &mut octets {
            let part = parts.next().ok_or_else(invalid)?;
            if part.len() != 2 || !part.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(invalid());
            }
            *octet = u8::from_str_radix(part, 16).map_err(|_| invalid())?;
        }
        match parts.next() {
            None => Ok(Self(octets)),
            Some(_) => Err(invalid()),
        }
    }
}

impl fmt::Display for MacAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c, d, e, g] = self.0;
        write!(f, "{a:02X}:{b:02X}:{c:02X}:{d:02X}:{e:02X}:{g:02X}")
    }
}

impl TryFrom<String> for MacAddress {
    type Error = DomainError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<MacAddress> for String {
    fn from(address: MacAddress) -> Self {
        address.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::MacAddress;

    #[test]
    fn round_trips_through_text() {
        let address: MacAddress = "A4:C1:38:1C:02:76".parse().expect("valid address");
        assert_eq!(
            address,
            MacAddress::new([0xA4, 0xC1, 0x38, 0x1C, 0x02, 0x76])
        );
        assert_eq!(address.to_string(), "A4:C1:38:1C:02:76");
        assert_eq!(MacAddress::try_from(String::from(address)), Ok(address));
    }

    #[test]
    fn lowercase_is_normalised() {
        let address: MacAddress = "a4:c1:38:1c:02:76".parse().expect("valid address");
        assert_eq!(address.to_string(), "A4:C1:38:1C:02:76");
    }

    #[test]
    fn invalid_addresses_are_rejected() {
        for text in [
            "A4:C1:38:1C:02",
            "A4:C1:38:1C:02:76:00",
            "A4:C1:38:1C:02:ZZ",
            "A4-C1-38-1C-02-76",
            "A4:C1:38:1C:02:7",
            "A4:C1:38:1C:+2:76",
            "A4:C1:38:1C:-1:76",
            "",
        ] {
            assert!(text.parse::<MacAddress>().is_err(), "{text} accepted");
        }
    }
}
