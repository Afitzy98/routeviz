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
    /// Number of (graph, pair) samples per size. Each sample generates a
    /// fresh graph at seed = base_seed + i, picks one (src, dst, amount)
    /// pair, and times + measures every algo on it. 1000 keeps CI runs
    /// under ~3 min and is plenty for stable histograms; bump higher
    /// for one-off local releases if you want smoother tails.
    #[arg(long, default_value_t = 1000)]
    samples: usize,
    /// Base RNG seed; sample i uses seed = base + i.
    #[arg(long, default_value_t = 42)]
    seed: u64,
    /// Pair density knob for the generator.
    #[arg(long, default_value_t = 0.35)]
    pair_density: f64,
    /// Output JSON path (deploy writes to web/public/benchmarks.json).
    #[arg(long)]
    out: PathBuf,
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
    /// Per-pair % improvements that the average above is computed from.
    /// Empty when no measurable pair existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    improvement_ratios: Vec<f64>,
    /// Raw per-sample timings in ms. Same length as `samples`; min/max/
    /// median are derived from this vec.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    times_ms: Vec<f64>,
}

#[derive(Serialize)]
struct BenchReport {
    generated_at_unix: u64,
    seed: u64,
    pair_density: f64,
    sizes: Vec<usize>,
    samples_per_config: usize,
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
        let cfg_template = GenConfig {
            num_tokens: size,
            pair_density: args.pair_density,
            price_noise: 0.0,
            seed: args.seed,
            ..GenConfig::default()
        };

        // Warm-up: settle caches/branch predictors using one disposable graph.
        {
            let (toks_w, pools_w) = PoolGenerator::new(cfg_template.clone()).generate();
            let g_w = Graph::new(toks_w, pools_w.clone());
            let pairs_w = sample_pairs_with_direct(&pools_w, args.seed, 1);
            if let Some(&(s_, d_, a_)) = pairs_w.first() {
                for _ in 0..10 {
                    let _ =
                        algo::solve_with_opts(Algorithm::Dijkstra, &g_w, s_, d_, a_, BENCH_OPTS);
                }
            }
        }

        // Per-algo accumulators.
        let mut times_ms: Vec<Vec<f64>> = vec![Vec::with_capacity(args.samples); algorithms.len()];
        let mut ratios: Vec<Vec<f64>> = vec![Vec::with_capacity(args.samples); algorithms.len()];
        let mut num_pools: usize = 0;

        print!("  n={size}: {} samples", args.samples);
        use std::io::Write;
        std::io::stdout().flush().ok();
        let report_every = (args.samples / 20).max(1);

        for s in 0..args.samples {
            let cfg_s = GenConfig {
                seed: args.seed.wrapping_add(s as u64),
                ..cfg_template.clone()
            };
            let (toks_s, pools_s) = PoolGenerator::new(cfg_s).generate();
            num_pools = pools_s.len();
            let graph_s = Graph::new(toks_s, pools_s.clone());
            let pair_seed = args.seed ^ ((size as u64).wrapping_mul(0x9E37_79B9) ^ s as u64);
            let pairs_s = sample_pairs_with_direct(&pools_s, pair_seed, 1);
            let Some(&(src, dst, amount)) = pairs_s.first() else {
                continue;
            };

            for (i, &(_, algo_enum, size_cap)) in algorithms.iter().enumerate() {
                if let Some(cap) = size_cap
                    && size > cap
                {
                    continue;
                }
                let t0 = Instant::now();
                let result =
                    algo::solve_with_opts(algo_enum, &graph_s, src, dst, amount, BENCH_OPTS);
                let elapsed_ns = t0.elapsed().as_nanos();
                let elapsed_ms = (elapsed_ns as f64) / 1_000_000.0;
                times_ms[i].push(elapsed_ms);

                if let Some(ratio) = pair_ratio(&result, &pools_s, &graph_s, src, dst, amount) {
                    ratios[i].push(ratio);
                }
                std::hint::black_box(&result);
            }

            if (s + 1) % report_every == 0 {
                print!(".");
                std::io::stdout().flush().ok();
            }
        }
        println!(" done");

        for (i, &(name, _, size_cap)) in algorithms.iter().enumerate() {
            if let Some(cap) = size_cap
                && size > cap
            {
                continue;
            }
            let t = std::mem::take(&mut times_ms[i]);
            if t.is_empty() {
                continue;
            }
            // Sorted copy for median/min/max; the unsorted (in-collection-
            // order) sample list is what we serialise so the FE can bin
            // its own histogram without re-sorting.
            let mut t_sorted = t.clone();
            t_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let median_ms = t_sorted[t_sorted.len() / 2];
            let min_ms = t_sorted[0];
            let max_ms = t_sorted[t_sorted.len() - 1];
            // Drop the now-needless duplicate.
            drop(t_sorted);

            let improvement_ratios = std::mem::take(&mut ratios[i]);
            let improvement_pct = if improvement_ratios.is_empty() {
                None
            } else {
                Some(improvement_ratios.iter().sum::<f64>() / improvement_ratios.len() as f64)
            };

            let imp_display = match improvement_pct {
                Some(p) => format!("{:>+7.3}%", p),
                None => "    —   ".to_string(),
            };
            println!(
                "{:<14} n={:<4} pools={:<5} samples={:<5} median={:>7.3} µs  min={:>7.3} µs  max={:>7.3} µs  vs-direct={}  ratios={}",
                name,
                size,
                num_pools,
                args.samples,
                median_ms * 1000.0,
                min_ms * 1000.0,
                max_ms * 1000.0,
                imp_display,
                improvement_ratios.len(),
            );

            results.push(BenchRow {
                algorithm: name,
                num_tokens: size,
                num_pools,
                samples: args.samples,
                median_ms,
                min_ms,
                max_ms,
                improvement_pct,
                improvement_ratios,
                times_ms: t,
            });
        }
    }

    let report = BenchReport {
        generated_at_unix: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        seed: args.seed,
        pair_density: args.pair_density,
        sizes: args.sizes,
        samples_per_config: args.samples,
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

// % improvement of an already-computed solve result vs the best direct
// pool at the same (src, dst, amount). None if either side rounded to
// zero or no direct pool exists.
fn pair_ratio(
    result: &algo::SolveResult,
    pools: &[Pool],
    graph: &Graph,
    src: Address,
    dst: Address,
    amount: U256,
) -> Option<f64> {
    let (algo_gross, algo_gas) = match result.outcome {
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
        _ => return None,
    };
    if algo_gross.is_zero() {
        return None;
    }
    let algo_net = algo_gross.saturating_sub(algo_gas);

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
            if let Some(t) = graph.tokens.iter().find(|t| t.address == dst) {
                best_direct_decimals = t.decimals;
                best_direct_price = t.true_price_usd;
            }
        }
    }
    if best_direct.is_zero() {
        return None;
    }
    let direct_gas_units = BENCH_OPTS.gas.gas_units(1, 1);
    let direct_gas =
        BENCH_OPTS
            .gas
            .gas_to_dst_token(direct_gas_units, best_direct_price, best_direct_decimals);
    let direct_net = best_direct.saturating_sub(direct_gas);
    let algo_f = algo_net.to_string().parse::<f64>().ok()?;
    let direct_f = direct_net.to_string().parse::<f64>().ok()?;
    if direct_f <= 0.0 {
        return None;
    }
    Some((algo_f / direct_f - 1.0) * 100.0)
}
