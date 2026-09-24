//! Parsers for device-info responses: firmware, battery, product/serial, and
//! capabilities. These wire formats are stable across ring generations.

use serde::{Deserialize, Serialize};

use crate::protocol::Packet;

/// Firmware / version metadata (response tag `0x09`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub api_version: String,
    pub firmware_version: String,
    pub bootloader_version: String,
    pub bt_stack_version: String,
    /// BLE MAC, colon-separated.
    pub mac: String,
}

impl DeviceInfo {
    pub fn parse(packet: &Packet) -> Option<DeviceInfo> {
        if packet.tag != 0x09 || packet.payload.len() < 18 {
            return None;
        }
        let p = &packet.payload;
        let v3 = |s: &[u8]| {
            s.iter()
                .map(|b| b.to_string())
                .collect::<Vec<_>>()
                .join(".")
        };
        Some(DeviceInfo {
            api_version: v3(&p[0..3]),
            firmware_version: v3(&p[3..6]),
            bootloader_version: v3(&p[6..9]),
            bt_stack_version: v3(&p[9..12]),
            mac: p[12..18]
                .iter()
                .rev()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(":"),
        })
    }
}

/// Battery state (response tag `0x0d`). Requires app-auth on rings with a key set.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Battery {
    pub percent: u8,
    pub charging_progress: u8,
    pub charging_recommended: u8,
}

impl Battery {
    pub fn parse(packet: &Packet) -> Option<Battery> {
        if packet.tag != 0x0d || packet.payload.len() < 3 {
            return None;
        }
        Some(Battery {
            percent: packet.payload[0],
            charging_progress: packet.payload[1],
            charging_recommended: packet.payload[2],
        })
    }
}

/// A product-info response (tag `0x19`): a status byte then ASCII/bytes.
pub fn parse_product_ascii(packet: &Packet) -> Option<String> {
    if packet.tag != 0x19 || packet.payload.is_empty() {
        return None;
    }
    if packet.payload[0] != 0 {
        return None;
    }
    let text: String = String::from_utf8_lossy(&packet.payload[1..])
        .trim_end_matches('\0')
        .trim()
        .to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// A single `(feature, value)` pair from a capabilities page.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Capability {
    pub feature: u8,
    pub value: u8,
}

/// Parse a capabilities page response (`0x2f` ext `0x02`).
pub fn parse_capabilities(packet: &Packet) -> Vec<Capability> {
    if packet.ext_tag() != Some(0x02) || packet.payload.len() < 2 {
        return Vec::new();
    }
    // payload: [0]=ext 0x02, [1]=page count, then (feature, value) pairs.
    packet.payload[2..]
        .chunks_exact(2)
        .map(|c| Capability {
            feature: c[0],
            value: c[1],
        })
        .collect()
}

/// Ring generation, derived from the hardware id the ring reports (`0x19`
/// product slot `HARDWARE`, e.g. `BLB_03`, `ORE_06`, `COR_05`).
///
/// The generation gates protocol behaviour: `SetFeatureMode` needs generation > 2,
/// the Ring 5 app-stream setup only applies to `Gen5`, and the SpO2 calibration
/// coefficients are chosen per hardware family (see `docs/spo2-calibration.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RingGeneration {
    /// Ring 3 (Horizon / Heritage), hardware ids `BLB_*` (codename `gen2x`).
    Gen3,
    /// Ring 4, hardware ids `ORE_*` / `JAD_*` (codename `oreo`).
    Gen4,
    /// Ring 5, hardware ids `COR_*` or any `*_05` suffix (codename `cooper`).
    Gen5,
    /// Anything we have not seen yet.
    Unknown,
}

impl RingGeneration {
    /// Classify a hardware id string. Unknown prefixes fall back to the `_05`
    /// suffix rule the Ring 5 detection used before this helper existed.
    pub fn from_hardware_id(hw: &str) -> Self {
        let hw = hw.trim();
        let upper = hw.to_ascii_uppercase();
        if upper.starts_with("BLB_") {
            RingGeneration::Gen3
        } else if upper.starts_with("ORE_") || upper.starts_with("JAD_") {
            RingGeneration::Gen4
        } else if upper.starts_with("COR_") || upper.rsplit('_').next() == Some("05") {
            RingGeneration::Gen5
        } else {
            RingGeneration::Unknown
        }
    }

    /// Generation number (3/4/5), or `None` when unknown.
    pub fn number(self) -> Option<u8> {
        match self {
            RingGeneration::Gen3 => Some(3),
            RingGeneration::Gen4 => Some(4),
            RingGeneration::Gen5 => Some(5),
            RingGeneration::Unknown => None,
        }
    }

    /// Firmware codename Oura uses for this family (`gen2x` / `oreo` / `cooper`).
    pub fn codename(self) -> Option<&'static str> {
        match self {
            RingGeneration::Gen3 => Some("gen2x"),
            RingGeneration::Gen4 => Some("oreo"),
            RingGeneration::Gen5 => Some("cooper"),
            RingGeneration::Unknown => None,
        }
    }

    /// `SetFeatureMode` is accepted from generation 3 on. An unknown hardware id
    /// is treated as capable so the caller attempts the call and lets the ring
    /// reject it if needed.
    pub fn supports_feature_mode(self) -> bool {
        match self.number() {
            Some(n) => n > 2,
            None => true,
        }
    }

    /// Ring 5 uses the extended app-stream setup and `ExtGetEvent` drain.
    pub fn is_ring5(self) -> bool {
        self == RingGeneration::Gen5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ring3_firmware() {
        let frame = hex::decode("091202000003040301000105000cffeeddccbbaa").unwrap();
        let p = Packet::parse(&frame).unwrap();
        let info = DeviceInfo::parse(&p).unwrap();
        assert_eq!(info.api_version, "2.0.0");
        assert_eq!(info.firmware_version, "3.4.3");
        assert_eq!(info.mac, "aa:bb:cc:dd:ee:ff");
    }

    #[test]
    fn parses_battery() {
        let p = Packet::parse(&hex::decode("0d0659000001f00f").unwrap()).unwrap();
        let b = Battery::parse(&p).unwrap();
        assert_eq!(b.percent, 0x59); // 89%
    }

    #[test]
    fn parses_serial() {
        let p = Packet::parse(&hex::decode("191100585858585858585858585858").unwrap()).unwrap();
        // status 0x00 then ASCII "XXXXXXXXXXXX" (real serial scrubbed; see local/device-identifiers.md)
        assert_eq!(parse_product_ascii(&p).as_deref(), Some("XXXXXXXXXXXX"));
    }

    #[test]
    fn classifies_ring_generations() {
        assert_eq!(RingGeneration::from_hardware_id("BLB_03"), RingGeneration::Gen3);
        assert_eq!(RingGeneration::from_hardware_id("ORE_06"), RingGeneration::Gen4);
        assert_eq!(RingGeneration::from_hardware_id("JAD_02"), RingGeneration::Gen4);
        assert_eq!(RingGeneration::from_hardware_id("COR_01"), RingGeneration::Gen5);
        assert_eq!(RingGeneration::from_hardware_id("XYZ_05"), RingGeneration::Gen5);
        assert_eq!(RingGeneration::from_hardware_id(""), RingGeneration::Unknown);
        assert!(RingGeneration::Gen3.supports_feature_mode());
        assert!(RingGeneration::Unknown.supports_feature_mode());
        assert_eq!(RingGeneration::Gen5.codename(), Some("cooper"));
    }
}
