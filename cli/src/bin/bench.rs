use std::fs;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use alloy_primitives::{Address, U256};
use clap::Parser;
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use routeviz_core::algo::{self, Algorithm, GasModel, Outcome, SolveOptions};
use routeviz_core::generator::{GenConfig, PoolGenerator};
use routeviz_core::graph::Graph;
use routeviz_core::pool::Pool;
use routeviz_core::token::TokenKind;
use serde::Serialize;

// 1 gwei = typical L2 pricing. Bump to 20+ for mainnet congestion.
const BENCH_GAS_GWEI: f64 = 1.0;
const BENCH_OPTS: SolveOptions = SolveOptions {
    with_trace: false,
    gas: GasModel {
        gas_price_gwei: BENCH_GAS_GWEI,
    },
};

#[derive(Parser)]
#[command(
    about = "routeviz algorithm benchmarks — native Rust timing harness",
    version
)]
struct Args {
    /// Comma-separated num_tokens values.
    #[arg(long, value_delimiter = ',', default_value = "10,30,100,300,1000")]
    sizes: Vec<usize>,
    /// Samples per (size × algorithm) configuration.
    #[arg(long, default_value_t = 250)]
    samples: usize,
    /// RNG seed for the graph generator.
    #[arg(long, default_value_t = 42)]
    seed: u64,
    /// Pair density knob for the generator.
    #[arg(long, default_value_t = 0.35)]
    pair_density: f64,
    /// Output JSON path (deploy writes to web/public/benchmarks.json).
    #[arg(long)]
    out: PathBuf,
    /// (src, dst) pairs sampled per cell for the "vs direct" metric.
    /// 0 to skip.
    #[arg(long, default_value_t = 40)]
    improvement_pairs: usize,
}

#[derive(Serialize)]
struct BenchRow {
    algorithm: &'static str,
    num_tokens: usize,
    num_pools: usize,
    samples: usize,
    median_ms: f64,
    min_ms: f64,
    max_ms: f64,
    /// Avg % improvement over the best direct pool, at 10% of reserves
    /// per pair. None when no valid pair was produced.
    improvement_pct: Option<f64>,
}

#[derive(Serialize)]
struct BenchReport {
    generated_at_unix: u64,
    seed: u64,
    pair_density: f64,
    sizes: Vec<usize>,
    samples_per_config: usize,
    improvement_pairs: usize,
    results: Vec<BenchRow>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if args.samples == 0 {
        return Err("--samples must be > 0".into());
    }

    // (name, Algorithm, optional size cap).
    let algorithms: &[(&'static str, Algorithm, Option<usize>)] = &[
        ("dijkstra", Algorithm::Dijkstra, None),
        ("bellman_ford", Algorithm::BellmanFord, None),
        ("amount_aware", Algorithm::AmountAware, None),
        ("split_dp", Algorithm::SplitDp, None),
        ("split_fw", Algorithm::SplitFw, None),
    ];

    let mut results: Vec<BenchRow> = Vec::with_capacity(args.sizes.len() * algorithms.len());

    for &size in &args.sizes {
        // Arb-free so BF walks all V-1 passes (no early-exit on cycle).
        let cfg = GenConfig {
            num_tokens: size,
            pair_density: args.pair_density,
            price_noise: 0.0,
            seed: args.seed,
            ..GenConfig::default()
        };
        let (tokens, pools) = PoolGenerator::new(cfg).generate();
        let num_pools = pools.len();
        let graph = Graph::new(tokens.clone(), pools.clone());

        let hubs: Vec<_> = tokens
            .iter()
            .filter(|t| matches!(t.kind, TokenKind::Hub))
            .collect();
        assert!(
            hubs.len() >= 2,
            "bench requires at least 2 hubs; got {}",
            hubs.len()
        );
        let src = hubs[0].address;
        let dst = hubs[1].address;
        let amount_in = U256::from(1_000_000u64);

        let improvement_pairs: Vec<(Address, Address, U256)> =
            sample_pairs_with_direct(&pools, args.seed ^ (size as u64), args.improvement_pairs);

        // Warm-up so first-sample outliers don't leak into min_ms.
        for _ in 0..10 {
            let _ =
                algo::solve_with_opts(Algorithm::Dijkstra, &graph, src, dst, amount_in, BENCH_OPTS);
        }

        for &(name, algo_enum, size_cap) in algorithms {
            if let Some(cap) = size_cap
                && size > cap
            {
                continue;
            }
            let mut times_ns: Vec<u128> = Vec::with_capacity(args.samples);
            for _ in 0..args.samples {
                let t0 = Instant::now();
                let result =
                    algo::solve_with_opts(algo_enum, &graph, src, dst, amount_in, BENCH_OPTS);
                let elapsed = t0.elapsed().as_nanos();
                std::hint::black_box(&result);
                times_ns.push(elapsed);
            }
            times_ns.sort_unstable();
            let to_ms = |ns: u128| (ns as f64) / 1_000_000.0;
            let median_ms = to_ms(times_ns[times_ns.len() / 2]);
            let min_ms = to_ms(times_ns[0]);
            let max_ms = to_ms(*times_ns.last().unwrap());

            let improvement_pct =
                measure_improvement(&graph, &pools, algo_enum, &improvement_pairs);

            results.push(BenchRow {
                algorithm: name,
                num_tokens: tokens.len(),
                num_pools,
                samples: args.samples,
                median_ms,
                min_ms,
                max_ms,
                improvement_pct,
            });

            let imp_display = match improvement_pct {
                Some(p) => format!("{:>+7.3}%", p),
                None => "    —   ".to_string(),
            };
            println!(
                "{:<14} n={:<4} pools={:<5} samples={:<5} median={:>7.3} µs  min={:>7.3} µs  max={:>7.3} µs  vs-direct={}",
                name,
                tokens.len(),
                num_pools,
                args.samples,
                median_ms * 1000.0,
                min_ms * 1000.0,
                max_ms * 1000.0,
                imp_display,
            );
        }
    }

    let report = BenchReport {
        generated_at_unix: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        seed: args.seed,
        pair_density: args.pair_density,
        sizes: args.sizes,
        samples_per_config: args.samples,
        improvement_pairs: args.improvement_pairs,
        results,
    };

    if let Some(parent) = args.out.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&report)?;
    fs::write(&args.out, json)?;
    println!(
        "\nWrote {} rows to {}",
        report.results.len(),
        args.out.display()
    );

    Ok(())
}

// Up to n distinct (src, dst, amount) tuples drawn from the existing
// pools, with amount = 10% of the src-side reserve.
fn sample_pairs_with_direct(pools: &[Pool], seed: u64, n: usize) -> Vec<(Address, Address, U256)> {
    if n == 0 || pools.is_empty() {
        return Vec::new();
    }
    let mut rng = StdRng::seed_from_u64(seed);
    let mut indices: Vec<usize> = (0..pools.len()).collect();
    indices.shuffle(&mut rng);
    indices.truncate(n);
    let mut out: Vec<(Address, Address, U256)> = Vec::with_capacity(indices.len());
    for i in indices {
        let pool = &pools[i];
        let amount = pool.reserve_a / U256::from(10u64);
        if amount.is_zero() {
            continue;
        }
        out.push((pool.token_a, pool.token_b, amount));
    }
    out
}

// Average % improvement of `algo`'s net output vs the best direct pool.
// None if no pair produced a usable result.
fn measure_improvement(
    graph: &Graph,
    pools: &[Pool],
    algo: Algorithm,
    pairs: &[(Address, Address, U256)],
) -> Option<f64> {
    if pairs.is_empty() {
        return None;
    }
    let mut ratios: Vec<f64> = Vec::with_capacity(pairs.len());
    for &(src, dst, amount) in pairs {
        let r = algo::solve_with_opts(algo, graph, src, dst, amount, BENCH_OPTS);
        let (algo_gross, algo_gas) = match r.outcome {
            Outcome::Found {
                amount_out,
                gas_cost,
                ..
            } => (amount_out, gas_cost),
            Outcome::FoundSplit {
                amount_out,
                gas_cost,
                ..
            } => (amount_out, gas_cost),
            _ => continue,
        };
        if algo_gross.is_zero() {
            continue;
        }
        let algo_net = algo_gross.saturating_sub(algo_gas);
        // Best direct pool output at this amount, then compute net of
        // single-hop gas via the same GasModel as algo so the
        // comparison is honest.
        let mut best_direct = U256::ZERO;
        let mut best_direct_decimals: u8 = 18;
        let mut best_direct_price: f64 = 0.0;
        for pool in pools {
            let matches = (pool.token_a == src && pool.token_b == dst)
                || (pool.token_a == dst && pool.token_b == src);
            if !matches {
                continue;
            }
            let out = pool.output_amount(src, amount);
            if out > best_direct {
                best_direct = out;
                let dst_token = graph.tokens.iter().find(|t| t.address == dst);
                if let Some(t) = dst_token {
                    best_direct_decimals = t.decimals;
                    best_direct_price = t.true_price_usd;
                }
            }
        }
        if best_direct.is_zero() {
            continue;
        }
        let direct_gas_units = BENCH_OPTS.gas.gas_units(1, 1);
        let direct_gas = BENCH_OPTS.gas.gas_to_dst_token(
            direct_gas_units,
            best_direct_price,
            best_direct_decimals,
        );
        let direct_net = best_direct.saturating_sub(direct_gas);
        let algo_f = algo_net.to_string().parse::<f64>().ok()?;
        let direct_f = direct_net.to_string().parse::<f64>().ok()?;
        if direct_f <= 0.0 {
            continue;
        }
        ratios.push((algo_f / direct_f - 1.0) * 100.0);
    }
    if ratios.is_empty() {
        return None;
    }
    Some(ratios.iter().sum::<f64>() / ratios.len() as f64)
}
