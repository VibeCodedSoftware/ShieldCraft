//! Small shared helpers: deterministic random numbers, hashing, time formatting.

use std::time::{SystemTime, UNIX_EPOCH};

/// SplitMix64 step. Used both as a PRNG and as a stateless hash for counter-based noise,
/// which lets any animation frame be generated independently and reproducibly.
#[inline]
pub fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Hash a list of keys into one 64-bit value.
#[inline]
pub fn hash_keys(keys: &[u64]) -> u64 {
    keys.iter()
        .fold(0x243F_6A88_85A3_08D3u64, |acc, k| splitmix64(acc ^ k.wrapping_mul(0x9E37_79B9_7F4A_7C15)))
}

/// Uniform float in [0, 1) from a 64-bit hash.
#[inline]
pub fn unit_f32(h: u64) -> f32 {
    ((h >> 40) as f32) * (1.0 / (1u64 << 24) as f32)
}

/// Uniform float in [-1, 1).
#[inline]
pub fn signed_f32(h: u64) -> f32 {
    unit_f32(h) * 2.0 - 1.0
}

/// Sequential PRNG for places that need a stream of numbers.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(splitmix64(seed ^ 0xA076_1D64_78BD_642F))
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        splitmix64(self.0)
    }
    pub fn f32(&mut self) -> f32 {
        unit_f32(self.next_u64())
    }
    pub fn signed(&mut self) -> f32 {
        signed_f32(self.next_u64())
    }
    pub fn bool(&mut self) -> bool {
        self.next_u64() & 1 == 0
    }
}

/// A seed derived from the wall clock, for "new random seed" buttons.
pub fn fresh_seed() -> u64 {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    splitmix64(nanos as u64 ^ (std::process::id() as u64) << 32) % 1_000_000_000
}

pub fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Format a UNIX timestamp as `YYYY-MM-DDTHH:MM:SSZ` (UTC) without extra dependencies.
pub fn format_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, (rem % 3600) / 60, rem % 60)
}

/// Compact local-agnostic stamp for filenames: `YYYYMMDD-HHMMSS`.
pub fn file_stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rem / 3600, (rem % 3600) / 60, rem % 60)
}

// Howard Hinnant's days-to-civil algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{n} B") } else { format!("{v:.1} {}", UNITS[i]) }
}

pub fn human_duration(secs: f64) -> String {
    if secs < 60.0 {
        format!("{secs:.1} s")
    } else if secs < 3600.0 {
        format!("{}m {:02}s", (secs / 60.0) as u64, (secs % 60.0) as u64)
    } else {
        format!("{}h {:02}m", (secs / 3600.0) as u64, ((secs % 3600.0) / 60.0) as u64)
    }
}

pub fn sha256_file(path: &std::path::Path) -> std::io::Result<String> {
    use sha2::Digest;
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_formatting() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc(1_792_540_800), "2026-10-21T00:00:00Z");
    }

    #[test]
    fn hash_is_deterministic_and_spread() {
        assert_eq!(hash_keys(&[1, 2, 3]), hash_keys(&[1, 2, 3]));
        assert_ne!(hash_keys(&[1, 2, 3]), hash_keys(&[1, 2, 4]));
        let mean: f32 = (0..10_000u64).map(|i| unit_f32(hash_keys(&[i]))).sum::<f32>() / 10_000.0;
        assert!((mean - 0.5).abs() < 0.02);
    }
}
