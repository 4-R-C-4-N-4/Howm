/// All generation parameters. No magic numbers in algorithm code — everything
/// references a field here.
///
/// Values marked [TUNE] are starting points to be validated during renderer
/// integration. The struct uses `Default` so we can instantiate with overrides
/// during testing.
#[derive(Debug, Clone)]
pub struct Config {
    // ── Road network ──────────────────────────────────────────────────────
    /// 75% of 0xFF — through-road threshold [TUNE]
    pub fate_through_max: u8,
    /// Next 15% — meeting-point threshold [TUNE]
    pub fate_meeting_max: u8,
    /// Stub length as fraction of terminal-to-seed distance [TUNE]
    pub dead_end_frac: f64,
    /// Endpoint exclusion zone for road-road intersection test
    pub intersect_margin: f64,

    // ── River system ──────────────────────────────────────────────────────
    /// % of gx values hosting a river (~20 in IPv4) [TUNE]
    pub river_density_percent: f64,
    /// Decorrelation constant — MUST NOT CHANGE
    pub river_salt: u32,

    // ── Block system ──────────────────────────────────────────────────────
    /// Vertex snapping resolution (world units)
    pub block_snap: f64,
    /// Minimum face area (world units²); smaller = sliver
    pub block_min_area: f64,
    /// Max half-edge steps per face trace
    pub block_face_iter_limit: u32,
    /// Normalised area above which block is "large"
    pub block_large_threshold: f64,
    /// Normalised area above which block is "medium"
    pub block_medium_threshold: f64,
    /// popcount ratio below which large blocks → water
    pub block_entropy_water: f64,
    /// popcount ratio below which medium blocks → plaza
    pub block_entropy_plaza: f64,
    /// popcount ratio below which rare small plaza appears
    pub block_entropy_sparse_plaza: f64,

    // ── World scale ───────────────────────────────────────────────────────
    /// World units per grid step [TUNE]
    pub scale: f64,
    /// Global jitter factor J; target ~0.75 [TUNE]
    pub jitter_default: f64,
    /// World units/second — comfortable walking pace [TUNE]
    pub player_speed: f64,
    /// Minimum world units between crossing points on shared edge
    pub min_road_spacing: f64,

    // ── Alley system ──────────────────────────────────────────────────────
    /// popcount >= this: no alley
    pub alley_popcount_none: u32,
    /// popcount >= this: dead-end alley
    pub alley_popcount_deadend: u32,
    /// popcount >= this: bisecting alley; below: voronoi gaps
    pub alley_popcount_bisecting: u32,
    /// Fraction of block longest dimension
    pub min_alley_width: f64,
    /// Additional random width range
    pub alley_width_range: f64,
    /// Radians (~17°) from perpendicular
    pub max_alley_angle_deviation: f64,

    // ── Plot subdivision ──────────────────────────────────────────────────
    /// World units² per base plot [TUNE]
    pub plot_area_base: f64,
    /// Max additional plots from popcount ratio
    pub plot_entropy_bonus: u32,
    /// Hard cap on plots per sub-polygon
    pub max_plots_per_block: u32,

    // ── Building height ───────────────────────────────────────────────────
    /// World units [TUNE]
    pub min_height: f64,
    /// World units [TUNE]
    pub max_height: f64,
    /// ± variation per plot [TUNE]
    pub height_jitter_range: f64,
    /// Absolute ceiling = max_height × this
    pub height_multiplier_cap: f64,

    // ── Entry point ───────────────────────────────────────────────────────
    /// World units — shared wall detection
    pub wall_adjacency_tol: f64,
    /// Minimum eligible wall segment
    pub min_door_wall_length: f64,
    /// World units — minimum navigable opening
    pub min_entry_width: f64,
    /// Additional random range
    pub entry_width_range: f64,

    // ── Interior ──────────────────────────────────────────────────────────
    /// Inset distance (world units)
    pub interior_wall_thickness: f64,
    /// Ceiling as fraction of exterior height
    pub interior_height_fraction: f64,
    /// Minimum normalised light level [TUNE]
    pub base_interior_light: f64,

    // ── Public/private rates ──────────────────────────────────────────────
    pub public_rate_building: f64,
    pub public_rate_plaza: f64,
    pub public_rate_park: f64,
    pub public_rate_water: f64,
    pub public_rate_riverbank: f64,

    // ── Domain modifiers on public rate ───────────────────────────────────
    pub domain_mod_public: f64,
    pub domain_mod_private: f64,
    pub domain_mod_loopback: f64,
    pub domain_mod_multicast: f64,
    pub domain_mod_reserved: f64,
    pub domain_mod_documentation: f64,

    // ── Zone system ───────────────────────────────────────────────────────
    /// World units² per base zone [TUNE]
    pub zone_area_base: f64,
    /// Max additional zones from popcount ratio
    pub zone_entropy_bonus: u32,

    // ── Road-edge fixture placement ───────────────────────────────────────
    /// Base world-unit interval
    pub lamp_spacing_base: f64,
    /// World units from road centreline
    pub lamp_offset: f64,

    // ── Flora ─────────────────────────────────────────────────────────────
    /// World units — dense road-edge flora
    pub min_flora_spacing: f64,
    /// World units — sparse road-edge flora
    pub max_flora_spacing: f64,
    /// inverted_age below this: no surface growth
    pub surface_growth_age_threshold: f64,

    // ── Creature timing ───────────────────────────────────────────────────
    /// Time slot duration for zone assignment [TUNE]
    pub creature_interval_ms: u64,
    /// Lerp duration at slot boundary
    pub transition_duration_ms: u64,

    // ── Idle behaviour ────────────────────────────────────────────────────
    /// Bitmask → 1–4 behaviours per creature
    pub idle_count_mask: u32,

    // ── Conveyance routing ────────────────────────────────────────────────
    /// Minimum loop period
    pub conveyance_loop_base_ms: u64,

    // ── Time of day ───────────────────────────────────────────────────────
    /// 1:1 with UTC — FIXED, not configurable
    pub day_duration_ms: u64,
    /// Fraction of day (20:00 UTC)
    pub night_start: f64,
    /// Fraction of day (06:00 UTC)
    pub night_end: f64,

    // ── Weather ───────────────────────────────────────────────────────────
    /// Wind re-roll interval
    pub wind_interval_ms: u64,
    /// Precipitation re-roll interval
    pub weather_interval_ms: u64,
    pub rain_base_public: f64,
    pub rain_base_private: f64,
    pub rain_base_loopback: f64,
    pub rain_base_multicast: f64,
    pub rain_base_reserved: f64,
    pub rain_base_documentation: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // Road network
            fate_through_max: 0xC0,
            fate_meeting_max: 0xE8,
            dead_end_frac: 0.35,
            intersect_margin: 0.02,

            // River system
            river_density_percent: 8.0,
            river_salt: 0xa3f1b7c5,

            // Block system
            block_snap: 0.5,
            block_min_area: 60.0,
            block_face_iter_limit: 300,
            block_large_threshold: 2.2,
            block_medium_threshold: 1.3,
            block_entropy_water: 0.35,
            block_entropy_plaza: 0.42,
            block_entropy_sparse_plaza: 0.25,

            // World scale
            scale: 200.0,
            jitter_default: 0.72,
            player_speed: 8.0,
            min_road_spacing: 28.0,

            // Alley system
            alley_popcount_none: 20,
            alley_popcount_deadend: 15,
            alley_popcount_bisecting: 10,
            min_alley_width: 0.08,
            alley_width_range: 0.06,
            max_alley_angle_deviation: 0.3,

            // Plot subdivision
            plot_area_base: 800.0,
            plot_entropy_bonus: 3,
            max_plots_per_block: 8,

            // Building height
            min_height: 1.0,
            max_height: 12.0,
            height_jitter_range: 2.0,
            height_multiplier_cap: 3.5,

            // Entry point
            wall_adjacency_tol: 0.5,
            min_door_wall_length: 0.5,
            min_entry_width: 0.8,
            entry_width_range: 0.6,

            // Interior
            interior_wall_thickness: 0.15,
            interior_height_fraction: 0.85,
            base_interior_light: 0.4,

            // Public/private rates
            public_rate_building: 0.25,
            public_rate_plaza: 0.80,
            public_rate_park: 1.00,
            public_rate_water: 0.50,
            public_rate_riverbank: 0.40,

            // Domain modifiers
            domain_mod_public: 0.00,
            domain_mod_private: -0.15,
            domain_mod_loopback: -0.20,
            domain_mod_multicast: 0.20,
            domain_mod_reserved: -0.10,
            domain_mod_documentation: 0.10,

            // Zone system
            zone_area_base: 400.0,
            zone_entropy_bonus: 4,

            // Road-edge fixtures
            lamp_spacing_base: 35.0,
            lamp_offset: 3.5,

            // Flora
            min_flora_spacing: 6.0,
            max_flora_spacing: 40.0,
            surface_growth_age_threshold: 0.4,

            // Creature timing
            creature_interval_ms: 45_000,
            transition_duration_ms: 3_000,

            // Idle behaviour
            idle_count_mask: 0x3,

            // Conveyance routing
            conveyance_loop_base_ms: 20_000,

            // Time of day
            day_duration_ms: 86_400_000,
            night_start: 0.833,
            night_end: 0.25,

            // Weather
            wind_interval_ms: 120_000,
            weather_interval_ms: 600_000,
            rain_base_public: 0.10,
            rain_base_private: 0.08,
            rain_base_loopback: 0.00,
            rain_base_multicast: 0.20,
            rain_base_reserved: 0.35,
            rain_base_documentation: 0.05,
        }
    }
}

/// Global config instance. In production this is Default; tests can override.
pub fn config() -> &'static Config {
    use std::sync::OnceLock;
    static CONFIG: OnceLock<Config> = OnceLock::new();
    CONFIG.get_or_init(Config::default)
}
