use alloy_primitives::{Address, U256};
use rand::seq::SliceRandom;
use rand::{Rng, RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rand_distr::{Distribution, LogNormal};
use serde::{Deserialize, Serialize};

use crate::pool::Pool;
use crate::token::{Token, TokenKind};

// Bump when the generator's output shape changes; shared-seed URLs
// check this to refuse stale graphs.
pub const GENERATOR_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenConfig {
    pub version: u32,
    pub num_tokens: usize,
    pub price_noise: f64,
    pub liquidity_spread: f64,
    pub pair_density: f64,
    pub seed: u64,
}

impl Default for GenConfig {
    fn default() -> Self {
        Self {
            version: GENERATOR_VERSION,
            num_tokens: 20,
            // Arb-free by default; users get a cycle by bumping the
            // slider or clicking Inject arb on the Arbitrage tab.
            price_noise: 0.0,
            liquidity_spread: 1.5,
            pair_density: 0.35,
            seed: 42,
        }
    }
}

// Constant-product venues. `weight` drives both the pool's existence
// probability for a given pair and its relative TVL.
#[derive(Debug, Clone, Copy)]
struct VenueSpec {
    name: &'static str,
    fee_bps: u16,
    weight: f64,
}

const VENUES: &[VenueSpec] = &[
    VenueSpec {
        name: "Uniswap V2",
        fee_bps: 30,
        weight: 1.00,
    },
    VenueSpec {
        name: "SushiSwap",
        fee_bps: 30,
        weight: 0.60,
    },
    VenueSpec {
        name: "PancakeSwap",
        fee_bps: 25,
        weight: 0.50,
    },
    VenueSpec {
        name: "Biswap",
        fee_bps: 10,
        weight: 0.30,
    },
];

// 5 hubs (WETH, USDC, USDT, WBTC, DAI) + a long tail of named spokes.
// Prices are illustrative.
#[derive(Debug, Clone, Copy)]
struct TokenSpec {
    symbol: &'static str,
    decimals: u8,
    typical_price_usd: f64,
    kind: TokenKind,
}

const TOKEN_CATALOG: &[TokenSpec] = &[
    // Hubs.
    TokenSpec {
        symbol: "WETH",
        decimals: 18,
        typical_price_usd: 3000.0,
        kind: TokenKind::Hub,
    },
    TokenSpec {
        symbol: "USDC",
        decimals: 6,
        typical_price_usd: 1.0,
        kind: TokenKind::Hub,
    },
    TokenSpec {
        symbol: "USDT",
        decimals: 6,
        typical_price_usd: 1.0,
        kind: TokenKind::Hub,
    },
    TokenSpec {
        symbol: "WBTC",
        decimals: 8,
        typical_price_usd: 100000.0,
        kind: TokenKind::Hub,
    },
    TokenSpec {
        symbol: "DAI",
        decimals: 18,
        typical_price_usd: 1.0,
        kind: TokenKind::Hub,
    },
    // Spokes.
    TokenSpec {
        symbol: "LINK",
        decimals: 18,
        typical_price_usd: 15.0,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "UNI",
        decimals: 18,
        typical_price_usd: 10.0,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "AAVE",
        decimals: 18,
        typical_price_usd: 100.0,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "CRV",
        decimals: 18,
        typical_price_usd: 0.5,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "MKR",
        decimals: 18,
        typical_price_usd: 2500.0,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "SNX",
        decimals: 18,
        typical_price_usd: 3.0,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "COMP",
        decimals: 18,
        typical_price_usd: 50.0,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "LDO",
        decimals: 18,
        typical_price_usd: 2.5,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "ARB",
        decimals: 18,
        typical_price_usd: 1.0,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "OP",
        decimals: 18,
        typical_price_usd: 2.0,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "PEPE",
        decimals: 18,
        typical_price_usd: 0.00001,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "SHIB",
        decimals: 18,
        typical_price_usd: 0.00002,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "GRT",
        decimals: 18,
        typical_price_usd: 0.2,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "FXS",
        decimals: 18,
        typical_price_usd: 2.5,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "MATIC",
        decimals: 18,
        typical_price_usd: 0.5,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "1INCH",
        decimals: 18,
        typical_price_usd: 0.3,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "SUSHI",
        decimals: 18,
        typical_price_usd: 1.0,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "YFI",
        decimals: 18,
        typical_price_usd: 8000.0,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "BAL",
        decimals: 18,
        typical_price_usd: 3.0,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "RPL",
        decimals: 18,
        typical_price_usd: 30.0,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "ENS",
        decimals: 18,
        typical_price_usd: 25.0,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "APE",
        decimals: 18,
        typical_price_usd: 1.5,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "CVX",
        decimals: 18,
        typical_price_usd: 3.5,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "GMX",
        decimals: 18,
        typical_price_usd: 35.0,
        kind: TokenKind::Spoke,
    },
    TokenSpec {
        symbol: "PENDLE",
        decimals: 18,
        typical_price_usd: 4.0,
        kind: TokenKind::Spoke,
    },
];

const MIN_HUBS: usize = 2;
const HUB_RATIO_DIVISOR: usize = 3; // target_hubs = num_tokens / 3

pub struct PoolGenerator {
    config: GenConfig,
    rng: ChaCha8Rng,
}

impl PoolGenerator {
    pub fn new(config: GenConfig) -> Self {
        let rng = ChaCha8Rng::seed_from_u64(config.seed);
        Self { config, rng }
    }

    pub fn config(&self) -> &GenConfig {
        &self.config
    }

    pub fn generate(&mut self) -> (Vec<Token>, Vec<Pool>) {
        let tokens = self.generate_tokens();
        let pools = self.generate_pools(&tokens);
        (tokens, pools)
    }

    fn generate_tokens(&mut self) -> Vec<Token> {
        let requested = self.config.num_tokens;
        let catalog_size = TOKEN_CATALOG.len();
        let from_catalog = requested.min(catalog_size);
        let synthetic_spokes = requested.saturating_sub(catalog_size);

        let max_hubs = TOKEN_CATALOG
            .iter()
            .filter(|t| matches!(t.kind, TokenKind::Hub))
            .count();
        // Hubs-first allocation: ~1 hub per 3 tokens, clamped to the
        // catalog's hub count. When the requested size exceeds the catalog,
        // we still take all available hubs and overflow into synthetic
        // spokes — this is the bench-scale path.
        let target_hubs = (requested / HUB_RATIO_DIVISOR)
            .clamp(MIN_HUBS, max_hubs)
            .min(from_catalog);
        let target_catalog_spokes = from_catalog - target_hubs;

        let mut hubs: Vec<&TokenSpec> = TOKEN_CATALOG
            .iter()
            .filter(|t| matches!(t.kind, TokenKind::Hub))
            .collect();
        let mut spokes: Vec<&TokenSpec> = TOKEN_CATALOG
            .iter()
            .filter(|t| matches!(t.kind, TokenKind::Spoke))
            .collect();
        hubs.shuffle(&mut self.rng);
        spokes.shuffle(&mut self.rng);
        hubs.truncate(target_hubs);
        spokes.truncate(target_catalog_spokes);

        // ±2 % jitter around each catalog token's typical price so different
        // seeds produce varied absolute prices without losing their anchor.
        let jitter = LogNormal::new(0.0, 0.02).expect("valid LogNormal params");
        // Synthetic tokens: wider log-normal around $1 spanning four orders
        // of magnitude — matches the long-tail ERC-20 price distribution
        // well enough for benchmarking.
        let synth_price = LogNormal::new(0.0, 2.0).expect("valid LogNormal params");

        let mut tokens = Vec::with_capacity(requested);
        // Hubs first, so tokens[0..hub_count] are all hubs.
        for spec in hubs.into_iter().chain(spokes) {
            let mut bytes = [0u8; 20];
            self.rng.fill_bytes(&mut bytes);
            let price = spec.typical_price_usd * jitter.sample(&mut self.rng);
            tokens.push(Token {
                address: Address::from(bytes),
                symbol: spec.symbol.to_string(),
                decimals: spec.decimals,
                true_price_usd: price,
                kind: spec.kind,
            });
        }
        // Synthetic spokes named T<catalog_size>... so they're visually
        // distinct from the real tickers.
        for i in 0..synthetic_spokes {
            let mut bytes = [0u8; 20];
            self.rng.fill_bytes(&mut bytes);
            let price: f64 = synth_price.sample(&mut self.rng);
            let price = price.max(1e-6);
            tokens.push(Token {
                address: Address::from(bytes),
                symbol: format!("T{}", catalog_size + i),
                decimals: 18,
                true_price_usd: price,
                kind: TokenKind::Spoke,
            });
        }
        tokens
    }

    fn generate_pools(&mut self, tokens: &[Token]) -> Vec<Pool> {
        if tokens.len() < 2 {
            return Vec::new();
        }
        let n = tokens.len();

        // Hub-hub pairs are deepest, hub-spoke shallower, spoke-spoke
        // doesn't exist (spokes route through hubs).
        let tvl_hub_hub = LogNormal::new((50e6_f64).ln(), self.config.liquidity_spread)
            .expect("valid LogNormal params");
        let tvl_hub_spoke = LogNormal::new((5e6_f64).ln(), self.config.liquidity_spread)
            .expect("valid LogNormal params");
        let noise_dist =
            LogNormal::new(0.0, self.config.price_noise).expect("valid LogNormal params");

        let mut pools = Vec::new();
        for i in 0..n {
            for j in (i + 1)..n {
                let token_a = &tokens[i];
                let token_b = &tokens[j];

                let (pair_probability, tvl_dist) = match (token_a.kind, token_b.kind) {
                    (TokenKind::Hub, TokenKind::Hub) => (1.0, &tvl_hub_hub),
                    (TokenKind::Hub, TokenKind::Spoke) | (TokenKind::Spoke, TokenKind::Hub) => {
                        (self.config.pair_density * 1.5, &tvl_hub_spoke)
                    }
                    (TokenKind::Spoke, TokenKind::Spoke) => continue,
                };
                let pair_probability = pair_probability.clamp(0.0, 1.0);

                // Each venue rolls independently — a pair can host 0..=N
                // parallel pools. Uniswap V2 (weight 1.0) is always
                // present on hub-hub pairs.
                for venue in VENUES {
                    let venue_probability = (pair_probability * venue.weight).clamp(0.0, 1.0);
                    if self.rng.random::<f64>() > venue_probability {
                        continue;
                    }

                    // TVL scales by venue weight — forks trail Uniswap.
                    let tvl = tvl_dist.sample(&mut self.rng) * venue.weight;
                    let noise = noise_dist.sample(&mut self.rng);

                    let reserve_a_human = (tvl / 2.0) / token_a.true_price_usd;
                    let reserve_b_human = (tvl / 2.0) / token_b.true_price_usd * noise;

                    let reserve_a = to_base_units_u256(reserve_a_human, token_a.decimals);
                    let reserve_b = to_base_units_u256(reserve_b_human, token_b.decimals);

                    if reserve_a.is_zero() || reserve_b.is_zero() {
                        continue;
                    }

                    let mut addr_bytes = [0u8; 20];
                    self.rng.fill_bytes(&mut addr_bytes);
                    pools.push(Pool {
                        address: Address::from(addr_bytes),
                        token_a: token_a.address,
                        token_b: token_b.address,
                        reserve_a,
                        reserve_b,
                        fee_bps: venue.fee_bps,
                        venue: venue.name.to_string(),
                    });
                }
            }
        }
        pools
    }

    // Shift one random pool's reserves to create a cycle profit.
    // magnitude is a fractional multiplier (0.05 = 5%).
    pub fn inject_arb(&mut self, pools: &mut [Pool], magnitude: f64) {
        assert!(!pools.is_empty(), "inject_arb needs at least one pool");
        let idx = self.rng.random_range(0..pools.len());
        let pool = &mut pools[idx];
        let bps = (magnitude * 10_000.0).round() as u64;
        let num = U256::from(10_000u64 + bps);
        let denom = U256::from(10_000u64);
        pool.reserve_a = pool.reserve_a * num / denom;
        pool.reserve_b = pool.reserve_b * denom / num;
    }
}

fn to_base_units_u256(human_amount: f64, decimals: u8) -> U256 {
    if !human_amount.is_finite() || human_amount <= 0.0 {
        return U256::ZERO;
    }
    let base = human_amount * 10f64.powi(decimals as i32);
    if !base.is_finite() {
        return U256::ZERO;
    }
    if base >= u128::MAX as f64 {
        return U256::from(u128::MAX);
    }
    U256::from(base as u128)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn same_seed_produces_identical_output() {
        let cfg = GenConfig::default();
        let (t1, p1) = PoolGenerator::new(cfg.clone()).generate();
        let (t2, p2) = PoolGenerator::new(cfg).generate();
        assert_eq!(t1, t2);
        assert_eq!(p1, p2);
    }

    #[test]
    fn different_seed_produces_different_output() {
        let (t1, _) = PoolGenerator::new(GenConfig {
            seed: 1,
            ..Default::default()
        })
        .generate();
        let (t2, _) = PoolGenerator::new(GenConfig {
            seed: 2,
            ..Default::default()
        })
        .generate();
        assert_ne!(t1, t2);
    }

    #[test]
    fn produces_requested_number_of_tokens() {
        let cfg = GenConfig {
            num_tokens: 15,
            ..Default::default()
        };
        let (tokens, _) = PoolGenerator::new(cfg).generate();
        assert_eq!(tokens.len(), 15);
    }

    #[test]
    fn num_tokens_beyond_catalog_pads_with_synthetic_spokes() {
        let requested = 100;
        let cfg = GenConfig {
            num_tokens: requested,
            ..Default::default()
        };
        let (tokens, _) = PoolGenerator::new(cfg).generate();
        assert_eq!(tokens.len(), requested);
        let catalog_symbols: HashSet<&'static str> =
            TOKEN_CATALOG.iter().map(|t| t.symbol).collect();
        let synthetic = tokens
            .iter()
            .filter(|t| !catalog_symbols.contains(t.symbol.as_str()))
            .count();
        assert_eq!(synthetic, requested - TOKEN_CATALOG.len());
        // All overflow tokens must be spokes — hubs never exceed the
        // catalog's hub count.
        for t in &tokens {
            if !catalog_symbols.contains(t.symbol.as_str()) {
                assert!(matches!(t.kind, TokenKind::Spoke));
            }
        }
    }

    #[test]
    fn default_token_decimals_come_from_catalog_set() {
        // At default num_tokens (20) every token comes from the catalog,
        // so decimals must be in the catalog's supported set.
        let (tokens, _) = PoolGenerator::new(GenConfig::default()).generate();
        for t in &tokens {
            assert!(
                t.decimals == 6 || t.decimals == 8 || t.decimals == 18,
                "unexpected decimals {}",
                t.decimals
            );
        }
    }

    #[test]
    fn default_token_symbols_are_real_tickers() {
        // At default num_tokens (20) no synthetic padding kicks in, so
        // every symbol must be a catalog entry.
        let (tokens, _) = PoolGenerator::new(GenConfig::default()).generate();
        let catalog_symbols: HashSet<&'static str> =
            TOKEN_CATALOG.iter().map(|t| t.symbol).collect();
        for t in &tokens {
            assert!(
                catalog_symbols.contains(t.symbol.as_str()),
                "symbol {} not in catalog",
                t.symbol
            );
        }
    }

    #[test]
    fn token_addresses_are_unique() {
        let (tokens, _) = PoolGenerator::new(GenConfig::default()).generate();
        let unique: HashSet<_> = tokens.iter().map(|t| t.address).collect();
        assert_eq!(unique.len(), tokens.len());
    }

    #[test]
    fn pool_addresses_are_unique() {
        let (_, pools) = PoolGenerator::new(GenConfig::default()).generate();
        let unique: HashSet<_> = pools.iter().map(|p| p.address).collect();
        assert_eq!(unique.len(), pools.len());
    }

    #[test]
    fn hubs_are_always_present_at_default_size() {
        let (tokens, _) = PoolGenerator::new(GenConfig::default()).generate();
        let hub_count = tokens
            .iter()
            .filter(|t| matches!(t.kind, TokenKind::Hub))
            .count();
        assert!(hub_count >= 2, "expected ≥ 2 hubs, got {}", hub_count);
    }

    #[test]
    fn every_hub_hub_pair_has_a_pool_at_default_config() {
        let (tokens, pools) = PoolGenerator::new(GenConfig::default()).generate();
        let hub_addresses: Vec<Address> = tokens
            .iter()
            .filter(|t| matches!(t.kind, TokenKind::Hub))
            .map(|t| t.address)
            .collect();
        for i in 0..hub_addresses.len() {
            for j in (i + 1)..hub_addresses.len() {
                let (a, b) = (hub_addresses[i], hub_addresses[j]);
                let exists = pools.iter().any(|p| {
                    (p.token_a == a && p.token_b == b) || (p.token_a == b && p.token_b == a)
                });
                assert!(exists, "missing hub-hub pool between {} and {}", a, b);
            }
        }
    }

    #[test]
    fn hub_hub_pairs_have_multiple_parallel_venues() {
        // With 4 V2-style venues at weights [1.0, 0.6, 0.5, 0.3], the
        // expected venue count per hub-hub pair is ~2.4, and Uniswap V2
        // is always present. At least some hub-hub pair should host > 1
        // venue pool.
        let (tokens, pools) = PoolGenerator::new(GenConfig::default()).generate();
        let hub_addresses: HashSet<Address> = tokens
            .iter()
            .filter(|t| matches!(t.kind, TokenKind::Hub))
            .map(|t| t.address)
            .collect();
        let mut by_pair: HashMap<(Address, Address), usize> = HashMap::new();
        for p in &pools {
            if hub_addresses.contains(&p.token_a) && hub_addresses.contains(&p.token_b) {
                let key = if p.token_a < p.token_b {
                    (p.token_a, p.token_b)
                } else {
                    (p.token_b, p.token_a)
                };
                *by_pair.entry(key).or_insert(0) += 1;
            }
        }
        assert!(
            by_pair.values().any(|&n| n > 1),
            "no hub-hub pair hosts parallel venue pools: {by_pair:?}"
        );
    }

    #[test]
    fn venues_span_multiple_factories() {
        // Sanity: the generator produces pools across at least two of the
        // four catalog venues at default config.
        let (_, pools) = PoolGenerator::new(GenConfig::default()).generate();
        let venues: HashSet<&str> = pools.iter().map(|p| p.venue.as_str()).collect();
        assert!(venues.len() >= 2, "only one venue represented: {venues:?}");
        // Uniswap V2 is the 1.0-weight venue — always present if any pool is.
        assert!(venues.contains("Uniswap V2"));
    }

    #[test]
    fn no_pool_connects_two_spokes() {
        let (tokens, pools) = PoolGenerator::new(GenConfig::default()).generate();
        let kind_by_addr: HashMap<Address, TokenKind> =
            tokens.iter().map(|t| (t.address, t.kind)).collect();
        for p in &pools {
            let ka = kind_by_addr[&p.token_a];
            let kb = kind_by_addr[&p.token_b];
            assert!(
                !matches!((ka, kb), (TokenKind::Spoke, TokenKind::Spoke)),
                "pool {} connects two spokes ({} / {})",
                p.address,
                p.token_a,
                p.token_b,
            );
        }
    }

    #[test]
    fn hubs_have_higher_average_degree_than_spokes() {
        let (tokens, pools) = PoolGenerator::new(GenConfig::default()).generate();
        let kind_by_addr: HashMap<Address, TokenKind> =
            tokens.iter().map(|t| (t.address, t.kind)).collect();
        let mut degree: HashMap<Address, usize> =
            tokens.iter().map(|t| (t.address, 0usize)).collect();
        for p in &pools {
            *degree.get_mut(&p.token_a).unwrap() += 1;
            *degree.get_mut(&p.token_b).unwrap() += 1;
        }
        let mut hub_sum = 0usize;
        let mut hub_count = 0usize;
        let mut spoke_sum = 0usize;
        let mut spoke_count = 0usize;
        for (addr, &d) in &degree {
            match kind_by_addr[addr] {
                TokenKind::Hub => {
                    hub_sum += d;
                    hub_count += 1;
                }
                TokenKind::Spoke => {
                    spoke_sum += d;
                    spoke_count += 1;
                }
            }
        }
        let hub_avg = hub_sum as f64 / hub_count.max(1) as f64;
        let spoke_avg = spoke_sum as f64 / spoke_count.max(1) as f64;
        assert!(
            hub_avg > spoke_avg,
            "hub avg degree {} not greater than spoke avg {}",
            hub_avg,
            spoke_avg
        );
    }

    #[test]
    fn pool_marginal_rates_approximate_true_prices_at_zero_noise() {
        // Venues carry real-world fees (10-30 bps), so marginal rates sit
        // 0.1-0.3% below the true ratio even at zero noise. The 1 %
        // tolerance below easily covers that.
        let cfg = GenConfig {
            price_noise: 0.0,
            num_tokens: 15,
            pair_density: 0.5,
            ..Default::default()
        };
        let (tokens, pools) = PoolGenerator::new(cfg).generate();
        assert!(!pools.is_empty());
        for pool in &pools {
            let token_a = tokens.iter().find(|t| t.address == pool.token_a).unwrap();
            let token_b = tokens.iter().find(|t| t.address == pool.token_b).unwrap();
            let actual_raw = pool.marginal_rate(pool.token_a);
            let actual_human =
                actual_raw * 10f64.powi(token_a.decimals as i32 - token_b.decimals as i32);
            let true_rate = token_a.true_price_usd / token_b.true_price_usd;
            let rel_err = (actual_human / true_rate - 1.0).abs();
            assert!(
                rel_err < 0.01,
                "pool {}/{}: rate {} vs true {} (rel err {})",
                token_a.symbol,
                token_b.symbol,
                actual_human,
                true_rate,
                rel_err
            );
        }
    }

    #[test]
    fn every_pool_has_finite_log_weight_in_both_directions() {
        let (_, pools) = PoolGenerator::new(GenConfig::default()).generate();
        for pool in &pools {
            let w_ab = pool.log_weight(pool.token_a);
            let w_ba = pool.log_weight(pool.token_b);
            assert!(w_ab.is_finite());
            assert!(w_ba.is_finite());
        }
    }

    #[test]
    fn generated_graph_constructs_without_panic() {
        let (tokens, pools) = PoolGenerator::new(GenConfig::default()).generate();
        let g = Graph::new(tokens.clone(), pools.clone());
        assert_eq!(g.num_tokens(), tokens.len());
        assert_eq!(g.num_pools(), pools.len());
    }

    #[test]
    fn inject_arb_modifies_exactly_one_pool() {
        let mut pg = PoolGenerator::new(GenConfig::default());
        let (_, mut pools) = pg.generate();
        let before = pools.clone();
        pg.inject_arb(&mut pools, 0.05);
        let changed = pools
            .iter()
            .zip(before.iter())
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(changed, 1);
    }

    #[test]
    fn inject_arb_preserves_pool_topology() {
        let mut pg = PoolGenerator::new(GenConfig::default());
        let (_, mut pools) = pg.generate();
        let before = pools.clone();
        pg.inject_arb(&mut pools, 0.05);
        for (after, b) in pools.iter().zip(before.iter()) {
            assert_eq!(after.address, b.address);
            assert_eq!(after.token_a, b.token_a);
            assert_eq!(after.token_b, b.token_b);
            assert_eq!(after.fee_bps, b.fee_bps);
        }
    }

    #[test]
    fn inject_arb_shifts_the_marginal_rate() {
        let mut pg = PoolGenerator::new(GenConfig::default());
        let (_, mut pools) = pg.generate();
        let before_rates: Vec<f64> = pools.iter().map(|p| p.marginal_rate(p.token_a)).collect();
        pg.inject_arb(&mut pools, 0.05);
        let after_rates: Vec<f64> = pools.iter().map(|p| p.marginal_rate(p.token_a)).collect();
        let shifted = before_rates
            .iter()
            .zip(after_rates.iter())
            .any(|(b, a)| (a / b - 1.0).abs() > 0.05);
        assert!(shifted);
    }
}
