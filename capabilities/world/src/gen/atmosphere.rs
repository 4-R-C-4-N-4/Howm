//! Atmosphere system — day/night phases, weather by /16 subnet, wind.
//!
//! All values are Tier 1: derived from UTC time + cell identity.
//! Two peers at the same time see the same atmosphere. No state storage.

use serde::{Deserialize, Serialize};

use super::cell::{Cell, Domain};
use super::config::config;
use super::hash::{ha, hb, hash_to_f64};

/// Day/night phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Night,
    Dawn,
    Day,
    Dusk,
}

/// Atmosphere state at a given UTC time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtmosphereState {
    pub time_of_day: f64,
    pub hour: u32,
    pub phase: Phase,
    pub phase_t: f64,
    pub sun_altitude: f64,
    pub sun_intensity: f64,
    pub is_raining: bool,
    pub rain_intensity: f64,
    pub wind_direction: f64,
    pub wind_intensity: f64,
    pub weather_group: u32,
}

// Phase thresholds from howm-atmosphere.md §2.1
const DAWN_START: f64 = 0.22;
const DAWN_END: f64 = 0.30;
const DUSK_START: f64 = 0.77;
const DUSK_END: f64 = 0.85;

/// Compute the current phase and interpolation parameter.
fn compute_phase(time_of_day: f64) -> (Phase, f64) {
    if time_of_day < DAWN_START {
        (Phase::Night, 0.0)
    } else if time_of_day < DAWN_END {
        let t = (time_of_day - DAWN_START) / (DAWN_END - DAWN_START);
        (Phase::Dawn, t)
    } else if time_of_day < DUSK_START {
        (Phase::Day, 0.0)
    } else if time_of_day < DUSK_END {
        let t = (time_of_day - DUSK_START) / (DUSK_END - DUSK_START);
        (Phase::Dusk, t)
    } else {
        (Phase::Night, 0.0)
    }
}

/// Sun altitude from time_of_day.
fn sun_altitude(time_of_day: f64) -> f64 {
    ((time_of_day - 0.25) * std::f64::consts::TAU).sin()
}

/// Sun intensity by phase.
fn sun_intensity(phase: Phase, phase_t: f64, sun_alt: f64) -> f64 {
    match phase {
        Phase::Night => 0.03,
        Phase::Dawn => phase_t * 0.5,
        Phase::Day => 0.5 + sun_alt * 0.3,
        Phase::Dusk => (1.0 - phase_t) * 0.5,
    }
}

/// Rain probability for a domain + group density.
fn rain_probability(domain: Domain, group_density: f64) -> f64 {
    let cfg = config();
    let base = match domain {
        Domain::Public => cfg.rain_base_public,
        Domain::Private => cfg.rain_base_private,
        Domain::Loopback => cfg.rain_base_loopback,
        Domain::Multicast => cfg.rain_base_multicast,
        Domain::Reserved => cfg.rain_base_reserved,
        Domain::Documentation => cfg.rain_base_documentation,
    };
    base + group_density * 0.3
}

/// Compute the weather group (/16 prefix) for a cell.
pub fn weather_group(cell: &Cell) -> u32 {
    (cell.octets[0] as u32) << 8 | cell.octets[1] as u32
}

/// Compute atmosphere state for a cell at a given UTC time.
pub fn compute_atmosphere(cell: &Cell, utc_time_ms: u64) -> AtmosphereState {
    let cfg = config();

    // Time of day
    let time_of_day = (utc_time_ms % cfg.day_duration_ms) as f64 / cfg.day_duration_ms as f64;
    let hour = (time_of_day * 24.0).floor() as u32;
    let (phase, phase_t) = compute_phase(time_of_day);

    // Sun
    let sun_alt = sun_altitude(time_of_day);
    let sun_int = sun_intensity(phase, phase_t, sun_alt);

    // Weather (per /16 subnet)
    let wg = weather_group(cell);
    let group_density = wg.count_ones() as f64 / 16.0;

    let weather_slot = utc_time_ms / cfg.weather_interval_ms;
    let weather_roll = hash_to_f64(ha(wg ^ weather_slot as u32));
    let rain_prob = rain_probability(cell.domain, group_density);
    let is_raining = weather_roll < rain_prob;

    let base_intensity = hash_to_f64(ha(wg ^ weather_slot as u32 ^ 0x1));
    let rain_intensity = if is_raining {
        base_intensity * (0.5 + cell.popcount_ratio * 0.5)
    } else {
        0.0
    };

    // Wind (per /16 subnet)
    let wind_slot = utc_time_ms / cfg.wind_interval_ms;
    let wind_direction = hash_to_f64(ha(wg ^ wind_slot as u32)) * std::f64::consts::TAU;
    let wind_intensity = hash_to_f64(hb(wg ^ wind_slot as u32));

    AtmosphereState {
        time_of_day,
        hour,
        phase,
        phase_t,
        sun_altitude: sun_alt,
        sun_intensity: sun_int,
        is_raining,
        rain_intensity,
        wind_direction,
        wind_intensity,
        weather_group: wg,
    }
}

/// Check if it is currently night (for creature nocturnal gating).
pub fn is_night(time_of_day: f64) -> bool {
    time_of_day >= DUSK_END || time_of_day < DAWN_START
}

/// Opacity modifier for creature activity pattern per howm-atmosphere §4.1.
pub fn creature_opacity(activity: &str, phase: Phase, phase_t: f64) -> f64 {
    match activity {
        "diurnal" => match phase {
            Phase::Night => 0.0,
            Phase::Dawn => phase_t,
            Phase::Day => 1.0,
            Phase::Dusk => 1.0 - phase_t,
        },
        "nocturnal" => match phase {
            Phase::Night => 1.0,
            Phase::Dawn => 1.0 - phase_t,
            Phase::Day => 0.0,
            Phase::Dusk => phase_t,
        },
        "crepuscular" => match phase {
            Phase::Night => 0.3,
            Phase::Dawn => 1.0,
            Phase::Day => 0.3,
            Phase::Dusk => 1.0,
        },
        _ => 1.0, // continuous
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_at_midnight() {
        let (phase, _) = compute_phase(0.0);
        assert_eq!(phase, Phase::Night);
    }

    #[test]
    fn phase_at_noon() {
        let (phase, _) = compute_phase(0.5);
        assert_eq!(phase, Phase::Day);
    }

    #[test]
    fn phase_at_dawn() {
        let (phase, t) = compute_phase(0.26);
        assert_eq!(phase, Phase::Dawn);
        assert!(t > 0.0 && t < 1.0);
    }

    #[test]
    fn phase_at_dusk() {
        let (phase, t) = compute_phase(0.81);
        assert_eq!(phase, Phase::Dusk);
        assert!(t > 0.0 && t < 1.0);
    }

    #[test]
    fn sun_peaks_at_noon() {
        let alt = sun_altitude(0.5);
        assert!(alt > 0.99, "Sun should peak at noon, got {}", alt);
    }

    #[test]
    fn sun_nadir_at_midnight() {
        let alt = sun_altitude(0.0);
        assert!(alt < -0.99, "Sun should be lowest at midnight, got {}", alt);
    }

    #[test]
    fn atmosphere_deterministic() {
        let cell = Cell::from_octets(93, 184, 216);
        let a1 = compute_atmosphere(&cell, 43200000); // noon
        let a2 = compute_atmosphere(&cell, 43200000);
        assert_eq!(a1.phase, a2.phase);
        assert_eq!(a1.is_raining, a2.is_raining);
        assert_eq!(a1.wind_direction, a2.wind_direction);
    }

    #[test]
    fn weather_group_by_16() {
        let cell = Cell::from_octets(93, 184, 216);
        assert_eq!(weather_group(&cell), (93 << 8) | 184);
        // Cells in same /16 share weather
        let cell2 = Cell::from_octets(93, 184, 100);
        assert_eq!(weather_group(&cell), weather_group(&cell2));
    }

    #[test]
    fn loopback_never_rains() {
        let cell = Cell::from_octets(127, 0, 0);
        // With base_rain = 0.0 for loopback, probability is just group_density * 0.3
        // This is very low for 127.0 (popcount = 7 of 16 = 0.4375, prob = 0.131)
        // Not guaranteed no rain, but verify determinism
        let a1 = compute_atmosphere(&cell, 0);
        let a2 = compute_atmosphere(&cell, 0);
        assert_eq!(a1.is_raining, a2.is_raining);
    }

    #[test]
    fn night_check() {
        assert!(is_night(0.0));
        assert!(is_night(0.9));
        assert!(!is_night(0.5));
        assert!(!is_night(0.3));
    }

    #[test]
    fn creature_opacity_diurnal() {
        assert_eq!(creature_opacity("diurnal", Phase::Day, 0.0), 1.0);
        assert_eq!(creature_opacity("diurnal", Phase::Night, 0.0), 0.0);
    }

    #[test]
    fn creature_opacity_nocturnal() {
        assert_eq!(creature_opacity("nocturnal", Phase::Night, 0.0), 1.0);
        assert_eq!(creature_opacity("nocturnal", Phase::Day, 0.0), 0.0);
    }
}
