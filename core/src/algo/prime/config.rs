// Tunables for the PRIME pipeline. All defaults match the values from the
// original V1 implementation and the paper's Algorithm 3 description.

#[derive(Debug, Clone)]
pub struct AsgmConfig {
    pub max_iter: usize,
    pub epsilon: f64,
    pub armijo_c: f64,
    pub armijo_beta: f64,
    pub armijo_delta_0: f64,
    pub armijo_min_delta: f64,
}

impl Default for AsgmConfig {
    fn default() -> Self {
        Self {
            max_iter: 50,
            epsilon: 1e-6,
            armijo_c: 1e-4,
            armijo_beta: 0.5,
            armijo_delta_0: 0.5,
            // ~10 halvings max; β=0.5 captures >99.998% at sub-ms.
            armijo_min_delta: 1e-4,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PrimeConfig {
    /// Cap on Stage-1 path admissions on top of the seed.
    pub max_paths: usize,
    /// Hop cap inside FindPath SPFA.
    pub max_hops: usize,
    /// Top-K by liquidity used as hub set.
    pub hub_count: usize,
    /// How many shortcuts to keep per ordered hub pair.
    pub shortcuts_per_hub_pair: usize,
    /// Max non-hub intermediates per shortcut path.
    pub shortcut_max_intermediates: usize,
    pub asgm: AsgmConfig,
}

impl Default for PrimeConfig {
    fn default() -> Self {
        Self {
            max_paths: 8,
            max_hops: 4,
            hub_count: 30,
            shortcuts_per_hub_pair: 3,
            shortcut_max_intermediates: 2,
            asgm: AsgmConfig::default(),
        }
    }
}
