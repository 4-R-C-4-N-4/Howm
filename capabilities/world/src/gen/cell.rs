use serde::{Deserialize, Serialize};

use super::hash::{ha, hb};

/// IP subnet domain classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Domain {
    Public = 0,
    Private = 1,
    Loopback = 2,
    Multicast = 3,
    Reserved = 4,
    Documentation = 5,
}

impl Domain {
    pub fn id(self) -> u32 {
        self as u32
    }
}

/// A cell in the world grid. Represents one /24 IPv4 subnet district.
///
/// All fields are derived deterministically from the three octets of the
/// /24 base address. This struct is the sole input to all downstream
/// generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cell {
    /// The three octets of the /24 base address.
    pub octets: [u8; 3],

    /// Packed cell key: (octet1 << 16) | (octet2 << 8) | octet3.
    pub key: u32,

    /// Grid X coordinate (octet3). Range 0–255.
    pub gx: u32,

    /// Grid Y coordinate (octet1 << 8 | octet2). Range 0–65535.
    pub gy: u32,

    /// ha(cell_key) — master seed for all generation.
    pub seed_hash: u32,

    /// Count of set bits in cell_key (0–24).
    pub popcount: u32,

    /// Normalised popcount: popcount / 24. Range [0.0, 1.0].
    pub popcount_ratio: f64,

    /// Sum of octets, normalised to [0.0, 1.0]. Age axis.
    pub age: f64,

    /// Inverted age (1.0 - age). Ancient = high value.
    pub inverted_age: f64,

    /// District domain classification.
    pub domain: Domain,

    /// Visual colour identity, 0–360°.
    pub hue: f64,

    /// For material selection.
    pub material_seed: u32,

    /// For creature selection.
    pub creature_seed: u32,
}

impl Cell {
    /// Create a cell from three octets (the /24 base address without the host byte).
    pub fn from_octets(o1: u8, o2: u8, o3: u8) -> Self {
        let key = (o1 as u32) << 16 | (o2 as u32) << 8 | o3 as u32;
        let gx = o3 as u32;
        let gy = (o1 as u32) << 8 | o2 as u32;
        let seed_hash = ha(key);
        let popcount = key.count_ones();
        let popcount_ratio = popcount as f64 / 24.0;
        let age = (o1 as f64 + o2 as f64 + o3 as f64) / 765.0;
        let domain = classify_domain(o1, o2, o3);
        let hue = (seed_hash & 0xFFF) as f64 / 4096.0 * 360.0;
        let material_seed = ha(key ^ 0x3f1a2b4c);
        let creature_seed = hb(key ^ 0x7c2e9f31);

        Self {
            octets: [o1, o2, o3],
            key,
            gx,
            gy,
            seed_hash,
            popcount,
            popcount_ratio,
            age,
            inverted_age: 1.0 - age,
            domain,
            hue,
            material_seed,
            creature_seed,
        }
    }

    /// Create a cell from a cell key (24-bit packed value).
    pub fn from_key(key: u32) -> Self {
        let o1 = ((key >> 16) & 0xFF) as u8;
        let o2 = ((key >> 8) & 0xFF) as u8;
        let o3 = (key & 0xFF) as u8;
        Self::from_octets(o1, o2, o3)
    }

    /// Create a cell from an IPv4 address string like "93.184.216.0".
    /// The last octet is ignored (it's the host byte in a /24).
    pub fn from_ip_str(ip: &str) -> Option<Self> {
        let parts: Vec<&str> = ip.split('.').collect();
        if parts.len() != 4 {
            return None;
        }
        let o1: u8 = parts[0].parse().ok()?;
        let o2: u8 = parts[1].parse().ok()?;
        let o3: u8 = parts[2].parse().ok()?;
        Some(Self::from_octets(o1, o2, o3))
    }

    /// Human-readable IP prefix.
    pub fn ip_prefix(&self) -> String {
        format!("{}.{}.{}.0/24", self.octets[0], self.octets[1], self.octets[2])
    }

    /// The aesthetic bucket — coarse district identity for form_id derivation.
    /// 3 bits popcount, 3 bits age, 3 bits domain.
    pub fn aesthetic_bucket(&self) -> u32 {
        let p = (self.popcount_ratio * 8.0).floor() as u32;
        let a = (self.age * 4.0).floor() as u32;
        let d = self.domain.id();
        (p & 0x7) | ((a & 0x7) << 3) | ((d & 0x7) << 5)
    }

    /// Get the cell key for a neighbor at grid offset (dx, dy).
    /// Handles wrapping: gx wraps at 0–255, gy wraps at 0–65535.
    pub fn neighbor_key(&self, dx: i32, dy: i32) -> u32 {
        let ngx = (self.gx as i32 + dx).rem_euclid(256) as u32;
        let ngy = (self.gy as i32 + dy).rem_euclid(65536) as u32;
        let no1 = (ngy >> 8) as u32;
        let no2 = ngy & 0xFF;
        (no1 << 16) | (no2 << 8) | ngx
    }
}

/// Classify an IP address into a domain.
fn classify_domain(o1: u8, o2: u8, _o3: u8) -> Domain {
    match o1 {
        10 => Domain::Private,
        127 => Domain::Loopback,
        172 if (16..=31).contains(&o2) => Domain::Private,
        192 if o2 == 168 => Domain::Private,
        192 if o2 == 0 && _o3 == 2 => Domain::Documentation,
        198 if o2 == 51 && _o3 == 100 => Domain::Documentation,
        203 if o2 == 0 && _o3 == 113 => Domain::Documentation,
        224..=239 => Domain::Multicast,
        240..=255 => Domain::Reserved,
        0 => Domain::Reserved,
        _ => Domain::Public,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_93_184_216() {
        let c = Cell::from_octets(93, 184, 216);
        assert_eq!(c.key, 0x5db8d8);
        assert_eq!(c.gx, 216);
        assert_eq!(c.gy, (93 << 8) | 184);
        assert_eq!(c.seed_hash, ha(0x5db8d8));
        assert_eq!(c.popcount, 0x5db8d8_u32.count_ones());
        assert_eq!(c.domain, Domain::Public);
    }

    #[test]
    fn cell_1_0_0() {
        let c = Cell::from_octets(1, 0, 0);
        assert_eq!(c.key, 0x010000);
        assert_eq!(c.gx, 0);
        assert_eq!(c.gy, 256);
        assert_eq!(c.popcount, 1);
        assert!((c.popcount_ratio - 1.0 / 24.0).abs() < 1e-10);
        assert!((c.age - 1.0 / 765.0).abs() < 1e-10);
        assert_eq!(c.domain, Domain::Public);
    }

    #[test]
    fn cell_from_ip_str() {
        let c = Cell::from_ip_str("93.184.216.0").unwrap();
        assert_eq!(c.key, 0x5db8d8);
    }

    #[test]
    fn cell_from_key_roundtrip() {
        let c1 = Cell::from_octets(93, 184, 216);
        let c2 = Cell::from_key(c1.key);
        assert_eq!(c1.key, c2.key);
        assert_eq!(c1.octets, c2.octets);
    }

    #[test]
    fn domain_classification() {
        assert_eq!(classify_domain(10, 0, 0), Domain::Private);
        assert_eq!(classify_domain(127, 0, 0), Domain::Loopback);
        assert_eq!(classify_domain(172, 16, 0), Domain::Private);
        assert_eq!(classify_domain(172, 15, 0), Domain::Public);
        assert_eq!(classify_domain(192, 168, 0), Domain::Private);
        assert_eq!(classify_domain(192, 0, 2), Domain::Documentation);
        assert_eq!(classify_domain(224, 0, 0), Domain::Multicast);
        assert_eq!(classify_domain(240, 0, 0), Domain::Reserved);
        assert_eq!(classify_domain(8, 8, 8), Domain::Public);
    }

    #[test]
    fn popcount_extremes() {
        // 1.0.0 → key = 0x010000, popcount = 1
        let low = Cell::from_octets(1, 0, 0);
        assert_eq!(low.popcount, 1);

        // 255.255.255 → key = 0xFFFFFF, popcount = 24
        let high = Cell::from_octets(255, 255, 255);
        assert_eq!(high.popcount, 24);
        assert!((high.popcount_ratio - 1.0).abs() < 1e-10);
    }

    #[test]
    fn aesthetic_bucket_structure() {
        let c = Cell::from_octets(93, 184, 216);
        let bucket = c.aesthetic_bucket();
        // Should be 9 bits max
        assert!(bucket < 256);
    }

    #[test]
    fn neighbor_key_wrapping() {
        // Cell at gx=0, should wrap to gx=255 when going left
        let c = Cell::from_octets(1, 0, 0); // gx=0
        let left = c.neighbor_key(-1, 0);
        assert_eq!(left & 0xFF, 255);

        // Cell at gx=255, should wrap to gx=0 when going right
        let c2 = Cell::from_octets(1, 0, 255); // gx=255
        let right = c2.neighbor_key(1, 0);
        assert_eq!(right & 0xFF, 0);
    }

    #[test]
    fn creature_seed_matches_spec() {
        // From Appendix C.2: hb(0x010000 ^ 0x7c2e9f31) = 0x05470d17
        let c = Cell::from_octets(1, 0, 0);
        assert_eq!(c.creature_seed, 0x05470d17);

        // hb(0xffaa55 ^ 0x7c2e9f31) = 0x0500d59a
        let c2 = Cell::from_octets(255, 170, 85);
        assert_eq!(c2.creature_seed, 0x0500d59a);
    }
}
