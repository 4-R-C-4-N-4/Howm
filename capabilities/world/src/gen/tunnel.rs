//! Underground tunnel generation — the 1-to-1 space between two connected peers
//! (spaces §3). A tunnel's geometry is driven by the connection (latency →
//! length, bandwidth → width, active capabilities → height); its aesthetic is a
//! gradient lerp between the two peers' Outside districts, so walking the tunnel
//! visibly transitions from one peer's world to the other's.

use serde::{Deserialize, Serialize};

use super::aesthetic::AestheticPalette;
use super::cell::Cell;
use super::hash::ha;
use super::home::peer_id_u32;

// ── Config (spaces §3.6) ────────────────────────────────────────────────────
const TUNNEL_BASE_LENGTH: f64 = 10.0;
const TUNNEL_LENGTH_PER_MS: f64 = 0.2;
const TUNNEL_MIN_WIDTH: f64 = 2.0;
const TUNNEL_MAX_WIDTH: f64 = 6.0;
const TUNNEL_BANDWIDTH_REF: f64 = 10000.0;
const TUNNEL_BASE_HEIGHT: f64 = 3.0;
const TUNNEL_HEIGHT_PER_CAP: f64 = 0.3;
const TUNNEL_SEGMENT_LENGTH: f64 = 4.0;
const CROSS_SECTION_SALT: u32 = 0xc055;
const CROSS_SECTIONS: [&str; 5] = ["rectangular", "arched", "rounded", "irregular", "hexagonal"];

/// Connection metrics that drive tunnel geometry/lighting. Per decision D3 these
/// come from the WireGuard/connection layer; callers stub them until that's
/// wired.
#[derive(Debug, Clone)]
pub struct TunnelMetrics {
    pub latency_ms: f64,
    pub bandwidth_kbps: f64,
    /// Mutually-active capability set (P2P-CD intersection).
    pub active_caps: Vec<String>,
    /// Connection uptime ratio 0–1 (drives lighting steadiness).
    pub uptime: f64,
}

impl Default for TunnelMetrics {
    fn default() -> Self {
        Self {
            latency_ms: 25.0,
            bandwidth_kbps: 5000.0,
            active_caps: Vec::new(),
            uptime: 0.95,
        }
    }
}

/// The blended aesthetic at a point along the tunnel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelAesthetic {
    pub hue: f64,
    pub popcount_ratio: f64,
    pub age: f64,
}

impl TunnelAesthetic {
    fn from_palette(p: &AestheticPalette) -> Self {
        Self {
            hue: p.hue,
            popcount_ratio: p.popcount_ratio,
            age: p.age,
        }
    }
    /// Lerp between the two endpoint aesthetics at `t` (0 = A, 1 = B).
    pub fn lerp(a: &Self, b: &Self, t: f64) -> Self {
        // Hue takes the shortest path around the wheel.
        let mut dh = b.hue - a.hue;
        if dh > 180.0 {
            dh -= 360.0;
        } else if dh < -180.0 {
            dh += 360.0;
        }
        Self {
            hue: (a.hue + dh * t).rem_euclid(360.0),
            popcount_ratio: a.popcount_ratio + (b.popcount_ratio - a.popcount_ratio) * t,
            age: a.age + (b.age - a.age) * t,
        }
    }
}

/// A capability marker placed along the tunnel (spaces §3.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelMarker {
    pub capability: String,
    /// Fractional position along the tunnel, 0–1.
    pub t: f64,
    /// Distance from the A end along the tunnel axis.
    pub distance: f64,
}

/// Lighting state derived from connection uptime (spaces §3.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelLighting {
    pub intensity: String,
    pub rhythm: String,
}

/// A complete underground tunnel between two peers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tunnel {
    pub tunnel_seed: u32,
    pub length: f64,
    pub width: f64,
    pub height: f64,
    pub cross_section: String,
    pub segment_count: usize,
    /// Aesthetic at the A end (peer A's district).
    pub aesthetic_a: TunnelAesthetic,
    /// Aesthetic at the B end (peer B's district).
    pub aesthetic_b: TunnelAesthetic,
    pub markers: Vec<TunnelMarker>,
    pub lighting: TunnelLighting,
}

fn uptime_lighting(uptime: f64) -> TunnelLighting {
    let (intensity, rhythm) = if uptime > 0.9 {
        ("moderate", "constant")
    } else if uptime >= 0.5 {
        ("subtle", "flickering")
    } else {
        ("faint", "sporadic")
    };
    TunnelLighting {
        intensity: intensity.to_string(),
        rhythm: rhythm.to_string(),
    }
}

/// Generate the tunnel between peers A and B. Order-independent: swapping the
/// two ends yields the same `tunnel_seed` and dimensions (the aesthetic gradient
/// flips direction, which is correct — A's end is always A's palette).
pub fn generate_tunnel(
    cell_a: &Cell,
    peer_a: &[u8],
    cell_b: &Cell,
    peer_b: &[u8],
    metrics: &TunnelMetrics,
) -> Tunnel {
    let a32 = peer_id_u32(peer_a);
    let b32 = peer_id_u32(peer_b);
    let tunnel_seed = ha(a32.min(b32) ^ a32.max(b32));

    let length = TUNNEL_BASE_LENGTH + metrics.latency_ms * TUNNEL_LENGTH_PER_MS;
    let width = TUNNEL_MIN_WIDTH
        + (metrics.bandwidth_kbps / TUNNEL_BANDWIDTH_REF).clamp(0.0, 1.0)
            * (TUNNEL_MAX_WIDTH - TUNNEL_MIN_WIDTH);
    let height =
        TUNNEL_BASE_HEIGHT + (metrics.active_caps.len() as f64 / 10.0) * TUNNEL_HEIGHT_PER_CAP;

    let cross_section_seed = ha(tunnel_seed ^ CROSS_SECTION_SALT);
    let cross_section =
        CROSS_SECTIONS[(cross_section_seed % CROSS_SECTIONS.len() as u32) as usize].to_string();

    let segment_count = (length / TUNNEL_SEGMENT_LENGTH).ceil().max(1.0) as usize;

    let palette_a = AestheticPalette::from_cell(cell_a);
    let palette_b = AestheticPalette::from_cell(cell_b);

    // Capability markers at evenly spaced positions (spaces §3.4).
    let n = metrics.active_caps.len();
    let markers = metrics
        .active_caps
        .iter()
        .enumerate()
        .map(|(i, cap)| {
            let t = (i as f64 + 1.0) / (n as f64 + 1.0);
            TunnelMarker {
                capability: cap.clone(),
                t,
                distance: t * length,
            }
        })
        .collect();

    Tunnel {
        tunnel_seed,
        length,
        width,
        height,
        cross_section,
        segment_count,
        aesthetic_a: TunnelAesthetic::from_palette(&palette_a),
        aesthetic_b: TunnelAesthetic::from_palette(&palette_b),
        markers,
        lighting: uptime_lighting(metrics.uptime),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells() -> (Cell, Cell) {
        (
            Cell::from_ip_str("93.184.216.0").unwrap(),
            Cell::from_ip_str("1.0.0.0").unwrap(),
        )
    }

    #[test]
    fn seed_is_order_independent() {
        let (a, b) = cells();
        let m = TunnelMetrics::default();
        let t1 = generate_tunnel(&a, &[1, 2, 3, 4], &b, &[9, 9, 9, 9], &m);
        let t2 = generate_tunnel(&b, &[9, 9, 9, 9], &a, &[1, 2, 3, 4], &m);
        assert_eq!(t1.tunnel_seed, t2.tunnel_seed);
        assert_eq!(t1.length, t2.length);
        assert_eq!(t1.cross_section, t2.cross_section);
    }

    #[test]
    fn geometry_from_metrics() {
        let (a, b) = cells();
        let m = TunnelMetrics {
            latency_ms: 50.0,
            bandwidth_kbps: 10000.0,
            active_caps: vec!["a".into(), "b".into()],
            uptime: 0.95,
        };
        let t = generate_tunnel(&a, &[1], &b, &[2], &m);
        assert_eq!(t.length, TUNNEL_BASE_LENGTH + 50.0 * TUNNEL_LENGTH_PER_MS); // 20
        assert_eq!(t.width, TUNNEL_MAX_WIDTH); // bandwidth at/above ref → max width
        assert_eq!(t.height, TUNNEL_BASE_HEIGHT + (2.0 / 10.0) * TUNNEL_HEIGHT_PER_CAP);
    }

    #[test]
    fn width_clamps_at_bounds() {
        let (a, b) = cells();
        let lo = generate_tunnel(
            &a,
            &[1],
            &b,
            &[2],
            &TunnelMetrics {
                bandwidth_kbps: 0.0,
                ..Default::default()
            },
        );
        let hi = generate_tunnel(
            &a,
            &[1],
            &b,
            &[2],
            &TunnelMetrics {
                bandwidth_kbps: 1_000_000.0,
                ..Default::default()
            },
        );
        assert_eq!(lo.width, TUNNEL_MIN_WIDTH);
        assert_eq!(hi.width, TUNNEL_MAX_WIDTH);
    }

    #[test]
    fn cross_section_in_set() {
        let (a, b) = cells();
        let t = generate_tunnel(&a, &[7], &b, &[8], &TunnelMetrics::default());
        assert!(CROSS_SECTIONS.contains(&t.cross_section.as_str()));
    }

    #[test]
    fn markers_evenly_spaced() {
        let (a, b) = cells();
        let m = TunnelMetrics {
            active_caps: vec!["x".into(), "y".into(), "z".into()],
            ..Default::default()
        };
        let t = generate_tunnel(&a, &[1], &b, &[2], &m);
        assert_eq!(t.markers.len(), 3);
        assert!((t.markers[0].t - 0.25).abs() < 1e-9);
        assert!((t.markers[1].t - 0.50).abs() < 1e-9);
        assert!((t.markers[2].t - 0.75).abs() < 1e-9);
    }

    #[test]
    fn aesthetic_lerp_endpoints_and_uptime() {
        let (a, b) = cells();
        let t = generate_tunnel(&a, &[1], &b, &[2], &TunnelMetrics::default());
        let at0 = TunnelAesthetic::lerp(&t.aesthetic_a, &t.aesthetic_b, 0.0);
        let at1 = TunnelAesthetic::lerp(&t.aesthetic_a, &t.aesthetic_b, 1.0);
        assert!((at0.popcount_ratio - t.aesthetic_a.popcount_ratio).abs() < 1e-9);
        assert!((at1.popcount_ratio - t.aesthetic_b.popcount_ratio).abs() < 1e-9);
        assert_eq!(t.lighting.intensity, "moderate"); // uptime 0.95 > 0.9
    }

    #[test]
    fn low_uptime_lighting() {
        let (a, b) = cells();
        let t = generate_tunnel(
            &a,
            &[1],
            &b,
            &[2],
            &TunnelMetrics {
                uptime: 0.3,
                ..Default::default()
            },
        );
        assert_eq!(t.lighting.intensity, "faint");
        assert_eq!(t.lighting.rhythm, "sporadic");
    }
}
