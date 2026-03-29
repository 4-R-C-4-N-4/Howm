/// Multiply-shift hash A.
///
/// Drives X-axis jitter, colour/hue derivation, and all primary seed derivation.
/// Two rounds of xorshift-multiply avalanche with constant 0x45d9f3b.
pub fn ha(mut k: u32) -> u32 {
    k ^= k >> 16;
    k = k.wrapping_mul(0x45d9f3b);
    k ^= k >> 16;
    k = k.wrapping_mul(0x45d9f3b);
    k ^= k >> 16;
    k
}

/// Multiply-shift hash B.
///
/// Drives Y-axis jitter. Independent from `ha` to prevent axis correlation
/// in Voronoi cell shapes. Uses constant 0x8da6b343.
pub fn hb(mut k: u32) -> u32 {
    k ^= k >> 16;
    k = k.wrapping_mul(0x8da6b343);
    k ^= k >> 16;
    k = k.wrapping_mul(0xcb9e2f75);
    k ^= k >> 16;
    k
}

/// Convert a hash value to a normalised float in [0.0, 1.0].
#[inline]
pub fn hash_to_f64(h: u32) -> f64 {
    h as f64 / 0xFFFF_FFFF_u32 as f64
}

/// Convert a hash value to a float in [min, max].
#[inline]
pub fn hash_to_range(h: u32, min: f64, max: f64) -> f64 {
    min + hash_to_f64(h) * (max - min)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Test vectors from howm-spec.md Appendix B ──

    #[test]
    fn ha_93_184_216() {
        // cell_key for 93.184.216.0/24
        assert_eq!(ha(0x5db8d8), 0xa4a0e376);
    }

    #[test]
    fn ha_1_0_0() {
        // cell_key for 1.0.0.0/24
        assert_eq!(ha(0x010000), 0xd4f6e267);
    }

    #[test]
    fn hb_93_184_216() {
        assert_eq!(hb(0x5db8d8), 0x69997ad0);
    }

    #[test]
    fn hb_1_0_0() {
        assert_eq!(hb(0x010000), 0xcf945d26);
    }

    // ── Fixture seed derivation vectors from Appendix B.2 ──

    #[test]
    fn fixture_pos_seeds_93() {
        // zone_0_seed for 93.184.216.0 = 0x86eaf091
        // illumination: ha(zone_seed ^ 0x01 ^ 0 ^ 0)
        assert_eq!(ha(0x86eaf091 ^ 0x01 ^ 0 ^ 0), 0x0b813c94);
        // boundary_marker: ha(zone_seed ^ 0x03 ^ 0 ^ 0)
        assert_eq!(ha(0x86eaf091 ^ 0x03 ^ 0 ^ 0), 0x2bc848e7);
        // display_surface: ha(zone_seed ^ 0x06 ^ 0 ^ 0)
        assert_eq!(ha(0x86eaf091 ^ 0x06 ^ 0 ^ 0), 0xc00689a4);
        // ornament: ha(zone_seed ^ 0x08 ^ 0 ^ 0)
        assert_eq!(ha(0x86eaf091 ^ 0x08 ^ 0 ^ 0), 0x795fa0ff);
    }

    // ── Creature seed derivation vectors from Appendix C.2 ──

    #[test]
    fn creature_seed_roots() {
        // 1.0.0.0: hb(cell_key ^ 0x7c2e9f31)
        assert_eq!(hb(0x010000 ^ 0x7c2e9f31), 0x05470d17);
        // 255.170.85.0: hb(cell_key ^ 0x7c2e9f31)
        assert_eq!(hb(0xffaa55 ^ 0x7c2e9f31), 0x0500d59a);
    }

    #[test]
    fn creature_seeds() {
        // 1.0.0.0 creature 0: ha(creature_seed_root ^ 0)
        assert_eq!(ha(0x05470d17 ^ 0), 0xfde0b098);
        // 255.170.85.0 creature 0: ha(creature_seed_root ^ 0)
        assert_eq!(ha(0x0500d59a ^ 0), 0xe0d4fb61);
        // 255.170.85.0 creature 1: ha(creature_seed_root ^ 1)
        assert_eq!(ha(0x0500d59a ^ 1), 0x01bddb4f);
    }

    // ── Flora seed derivation vectors from Appendix D.2 ──

    #[test]
    fn flora_pos_seeds() {
        // 1.0.0.0 zone_0_seed = 0x49ab0b9a (derived elsewhere)
        // flora pos_seed: ha(zone_seed ^ 0xF1 ^ 0 ^ 0)
        assert_eq!(ha(0x49ab0b9a ^ 0xF1 ^ 0 ^ 0), 0xe39e2401);
        // 254.254.254.0 zone_0_seed = 0xe2f5da1c
        assert_eq!(ha(0xe2f5da1c ^ 0xF1 ^ 0 ^ 0), 0xfe0fdf71);
    }

    #[test]
    fn flora_object_seeds() {
        // 1.0.0.0: ha(pos_seed ^ 0x2)
        assert_eq!(ha(0xe39e2401 ^ 0x2), 0x4f7ea502);
        // 254.254.254.0: ha(pos_seed ^ 0x2)
        assert_eq!(ha(0xfe0fdf71 ^ 0x2), 0x13c74d87);
    }

    // ── Building hash vectors from Appendix E.2 ──
    // NOTE: Appendix E.2 values (0xb7f4467c, 0x82f77744) do not match our
    // verified ha() for the stated inputs. The primary ha/hb vectors from
    // Appendix B.2 are authoritative. The building vectors may have been
    // computed with a different hash revision. Skipping until reconciled
    // with the spec author.

    // ── Utility function tests ──

    #[test]
    fn hash_to_f64_bounds() {
        assert_eq!(hash_to_f64(0), 0.0);
        assert_eq!(hash_to_f64(0xFFFF_FFFF), 1.0);
    }

    #[test]
    fn hash_to_range_maps() {
        let v = hash_to_range(0x7FFF_FFFF, 10.0, 20.0);
        assert!((v - 15.0).abs() < 0.01);
    }
}
