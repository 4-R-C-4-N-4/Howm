use serde::{Deserialize, Serialize};

use super::cell::{Cell, Domain};
use super::config::config;

/// The aesthetic palette for a district — all continuous parameters that
/// drive downstream form decisions. Derived deterministically from the cell.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AestheticPalette {
    /// Primary complexity axis. 0.0 (crystalline, minimal) to 1.0 (baroque, overgrown).
    pub popcount_ratio: f64,

    /// Inverse of popcount_ratio.
    pub inverse_popcount_ratio: f64,

    /// Raw popcount integer (0–24).
    pub popcount: u32,

    /// Age axis. 0.0 (ancient) to 1.0 (recent).
    pub age: f64,

    /// Inverted age. 1.0 (ancient) to 0.0 (recent).
    pub inverted_age: f64,

    /// Domain classification.
    pub domain: Domain,
    pub domain_id: u32,

    /// Hue identity, 0–360°.
    pub hue: f64,

    /// Coarse bucket for form_id derivation.
    pub aesthetic_bucket: u32,

    /// Material selection seed.
    pub material_seed: u32,

    /// Creature selection seed.
    pub creature_seed: u32,
}

impl AestheticPalette {
    /// Derive the full aesthetic palette from a cell.
    pub fn from_cell(cell: &Cell) -> Self {
        Self {
            popcount_ratio: cell.popcount_ratio,
            inverse_popcount_ratio: 1.0 - cell.popcount_ratio,
            popcount: cell.popcount,
            age: cell.age,
            inverted_age: cell.inverted_age,
            domain: cell.domain,
            domain_id: cell.domain.id(),
            hue: cell.hue,
            aesthetic_bucket: cell.aesthetic_bucket(),
            material_seed: cell.material_seed,
            creature_seed: cell.creature_seed,
        }
    }

    /// Domain modifier on public building rate.
    pub fn domain_public_rate_modifier(&self) -> f64 {
        let cfg = config();
        match self.domain {
            Domain::Public => cfg.domain_mod_public,
            Domain::Private => cfg.domain_mod_private,
            Domain::Loopback => cfg.domain_mod_loopback,
            Domain::Multicast => cfg.domain_mod_multicast,
            Domain::Reserved => cfg.domain_mod_reserved,
            Domain::Documentation => cfg.domain_mod_documentation,
        }
    }

    /// Base public rate for a given block type, including popcount and domain modifiers.
    pub fn public_rate(&self, block_type: &str) -> f64 {
        let cfg = config();
        let base = match block_type {
            "building" => cfg.public_rate_building,
            "plaza" => cfg.public_rate_plaza,
            "park" => cfg.public_rate_park,
            "water" => cfg.public_rate_water,
            "riverbank" => cfg.public_rate_riverbank,
            _ => cfg.public_rate_building,
        };
        (base + self.popcount_ratio * 0.2 + self.domain_public_rate_modifier())
            .clamp(0.0, 1.0)
    }

    /// Rain base probability for this domain.
    pub fn rain_base(&self) -> f64 {
        let cfg = config();
        match self.domain {
            Domain::Public => cfg.rain_base_public,
            Domain::Private => cfg.rain_base_private,
            Domain::Loopback => cfg.rain_base_loopback,
            Domain::Multicast => cfg.rain_base_multicast,
            Domain::Reserved => cfg.rain_base_reserved,
            Domain::Documentation => cfg.rain_base_documentation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_1_0_0() {
        let cell = Cell::from_octets(1, 0, 0);
        let p = AestheticPalette::from_cell(&cell);
        assert_eq!(p.popcount, 1);
        assert!((p.popcount_ratio - 1.0 / 24.0).abs() < 1e-10);
        assert!((p.age - 1.0 / 765.0).abs() < 1e-10);
        assert!((p.inverted_age - (1.0 - 1.0 / 765.0)).abs() < 1e-10);
        assert_eq!(p.domain, Domain::Public);
    }

    #[test]
    fn palette_254_254_254() {
        let cell = Cell::from_octets(254, 254, 254);
        let p = AestheticPalette::from_cell(&cell);
        // popcount of 0xFEFEFE = 21
        assert_eq!(p.popcount, 0xFEFEFE_u32.count_ones());
        // age = (254+254+254)/765 ≈ 0.996
        assert!((p.age - 762.0 / 765.0).abs() < 1e-3);
        assert_eq!(p.domain, Domain::Reserved); // 254.x is reserved
    }

    #[test]
    fn public_rate_scales_with_popcount() {
        let low = Cell::from_octets(1, 0, 0);
        let high = Cell::from_octets(255, 255, 254);
        let p_low = AestheticPalette::from_cell(&low);
        let p_high = AestheticPalette::from_cell(&high);
        // Higher popcount → higher public rate
        assert!(p_high.public_rate("building") > p_low.public_rate("building"));
    }
}
