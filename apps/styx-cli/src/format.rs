use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::CliError;

const HEX_LEN: usize = 40;
const SPARKLINE_LEVELS: [char; 8] = ['_', '.', ':', '-', '=', '+', '*', '#'];

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InfoHashHex([u8; 20]);

impl InfoHashHex {
    #[must_use]
    pub const fn new(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn repeat(byte: u8) -> Self {
        Self([byte; 20])
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 20] {
        &self.0
    }
}

impl fmt::Display for InfoHashHex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for InfoHashHex {
    type Err = CliError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != HEX_LEN {
            return Err(CliError::InvalidInfoHashLength);
        }

        let mut bytes = [0_u8; 20];
        for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
            let high = decode_hex(chunk[0], index * 2)?;
            let low = decode_hex(chunk[1], index * 2 + 1)?;
            bytes[index] = (high << 4) | low;
        }
        Ok(Self(bytes))
    }
}

impl Serialize for InfoHashHex {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for InfoHashHex {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[must_use]
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[must_use]
pub fn format_rate(bytes_per_second: u64) -> String {
    format!("{}/s", format_bytes(bytes_per_second))
}

#[must_use]
pub fn format_percent(progress: f32) -> String {
    let progress = if progress.is_finite() { progress } else { 0.0 };
    format!("{:.1}%", progress.clamp(0.0, 1.0) * 100.0)
}

#[must_use]
pub fn sparkline(samples: &[u64], width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if samples.is_empty() {
        return " ".repeat(width);
    }

    let max = samples.iter().copied().max().unwrap_or(0);
    if max == 0 {
        return SPARKLINE_LEVELS[0].to_string().repeat(width);
    }

    (0..width)
        .map(|column| {
            let index = column * samples.len() / width;
            let sample = samples[index];
            let level = sample as f64 / max as f64;
            let bucket = (level * (SPARKLINE_LEVELS.len() - 1) as f64).round() as usize;
            SPARKLINE_LEVELS[bucket]
        })
        .collect()
}

fn decode_hex(byte: u8, index: usize) -> Result<u8, CliError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(CliError::InvalidInfoHashHex {
            index,
            byte: byte as char,
        }),
    }
}
