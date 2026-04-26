use std::str::FromStr;

use alloy_primitives::U256;
use clap::{Parser, Subcommand, ValueEnum};
use routeviz_core::algo::{self, Algorithm, Outcome};
use routeviz_core::generator::{GenConfig, PoolGenerator};
use routeviz_core::graph::Graph;
use routeviz_core::pool::Pool;
use routeviz_core::token::{Token, TokenKind};

#[derive(Parser)]
#[command(about = "routeviz — DEX routing + arbitrage CLI", version)]
struct Args {
    /// Seed for the deterministic graph generator.
    #[arg(long, default_value_t = 42, global = true)]
    seed: u64,
    /// Number of tokens to generate (capped at the catalog size).
    #[arg(long, default_value_t = 20, global = true)]
    num_tokens: usize,
    /// Price noise. 0.0 (default) = strictly arb-free; positive values
    /// sprinkle log-normal perturbations that can introduce negative cycles.
    #[arg(long, default_value_t = 0.0, global = true)]
    price_noise: f64,
    /// Pair density (hub↔spoke only; hub↔hub is always 1.0).
    #[arg(long, default_value_t = 0.35, global = true)]
    pair_density: f64,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print a summary of the generated graph (tokens, pools, hub/spoke split).
    Info,
    /// Run a solve from `--from` to `--to` and print the outcome.
    Solve {
        #[arg(long, default_value = "dijkstra")]
        algo: AlgoArg,
        /// Source token symbol (e.g. WETH, USDC).
        #[arg(long)]
        from: String,
        /// Destination token symbol (e.g. USDT, DAI).
        #[arg(long)]
        to: String,
        /// Amount in, expressed as a decimal in the source token's units.
        #[arg(long, default_value = "1")]
        amount: String,
    },
}

#[derive(Copy, Clone, ValueEnum)]
enum AlgoArg {
    Dijkstra,
    BellmanFord,
    AmountAware,
    SplitDp,
    SplitFw,
}

impl From<AlgoArg> for Algorithm {
    fn from(a: AlgoArg) -> Self {
        match a {
            AlgoArg::Dijkstra => Algorithm::Dijkstra,
            AlgoArg::BellmanFord => Algorithm::BellmanFord,
            AlgoArg::AmountAware => Algorithm::AmountAware,
            AlgoArg::SplitDp => Algorithm::SplitDp,
            AlgoArg::SplitFw => Algorithm::SplitFw,
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let cfg = GenConfig {
        seed: args.seed,
        num_tokens: args.num_tokens,
        price_noise: args.price_noise,
        pair_density: args.pair_density,
        ..GenConfig::default()
    };
    let (tokens, pools) = PoolGenerator::new(cfg.clone()).generate();

    match args.cmd {
        Cmd::Info => {
            print_info(&cfg, &tokens, &pools);
        }
        Cmd::Solve {
            algo,
            from,
            to,
            amount,
        } => {
            let graph = Graph::new(tokens.clone(), pools);
            let src = find_by_symbol(&tokens, &from)?;
            let dst = find_by_symbol(&tokens, &to)?;
            let amount_in = parse_amount(&amount, src.decimals)?;
            let result = algo::solve(algo.into(), &graph, src.address, dst.address, amount_in);
            print_result(&result.outcome, &graph, src, dst);
        }
    }
    Ok(())
}

fn print_info(cfg: &GenConfig, tokens: &[Token], pools: &[Pool]) {
    let hubs: Vec<_> = tokens
        .iter()
        .filter(|t| matches!(t.kind, TokenKind::Hub))
        .collect();
    let spokes: Vec<_> = tokens
        .iter()
        .filter(|t| matches!(t.kind, TokenKind::Spoke))
        .collect();
    let mut by_venue: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for p in pools {
        *by_venue.entry(p.venue.as_str()).or_insert(0) += 1;
    }
    println!("routeviz graph (seed {})", cfg.seed);
    println!(
        "  tokens:      {}  (hubs {}, spokes {})",
        tokens.len(),
        hubs.len(),
        spokes.len()
    );
    println!("  pools:       {}", pools.len());
    for (venue, count) in &by_venue {
        println!("    {:<14} {}", venue, count);
    }
    println!("  price_noise: {}", cfg.price_noise);
    println!();
    println!("Hubs:");
    for t in &hubs {
        println!(
            "  {:<6} decimals={:<3} price=${:.4}",
            t.symbol, t.decimals, t.true_price_usd
        );
    }
    println!();
    println!("Spokes:");
    for t in &spokes {
        println!(
            "  {:<6} decimals={:<3} price=${:.6}",
            t.symbol, t.decimals, t.true_price_usd
        );
    }
}

fn print_result(outcome: &Outcome, graph: &Graph, src: &Token, dst: &Token) {
    match outcome {
        Outcome::Found {
            path,
            pools_used,
            total_log_weight,
            product_of_rates,
            amount_in,
            amount_out,
            ..
        } => {
            let symbols: Vec<&str> = path
                .iter()
                .map(|addr| {
                    graph
                        .tokens
                        .iter()
                        .find(|t| t.address == *addr)
                        .map(|t| t.symbol.as_str())
                        .unwrap_or("?")
                })
                .collect();
            println!("Found route {} → {}", src.symbol, dst.symbol);
            println!("  hops:             {}", pools_used.len());
            println!("  path:             {}", symbols.join(" → "));
            println!("  total log-weight: {:.6}", total_log_weight);
            println!("  product of rates: {:.6e}", product_of_rates);
            println!(
                "  input:            {} {}",
                format_amount(amount_in, src.decimals),
                src.symbol
            );
            println!(
                "  output:           {} {}",
                format_amount(amount_out, dst.decimals),
                dst.symbol
            );
        }
        Outcome::FoundSplit {
            legs,
            amount_in,
            amount_out,
            ..
        } => {
            println!("Found split route {} → {}", src.symbol, dst.symbol);
            println!("  legs:             {}", legs.len());
            println!(
                "  input:            {} {}",
                format_amount(amount_in, src.decimals),
                src.symbol
            );
            println!(
                "  output:           {} {}",
                format_amount(amount_out, dst.decimals),
                dst.symbol
            );
            let total_in_f = {
                let s = amount_in.to_string().parse::<f64>().unwrap_or(0.0);
                if s <= 0.0 { 1.0 } else { s }
            };
            for (i, leg) in legs.iter().enumerate() {
                let symbols: Vec<&str> = leg
                    .path
                    .iter()
                    .map(|addr| {
                        graph
                            .tokens
                            .iter()
                            .find(|t| t.address == *addr)
                            .map(|t| t.symbol.as_str())
                            .unwrap_or("?")
                    })
                    .collect();
                let leg_in_f = leg.amount_in.to_string().parse::<f64>().unwrap_or(0.0);
                let pct = (leg_in_f / total_in_f) * 100.0;
                println!(
                    "  leg {}: {:>5.1}%  {:<30}  {} {}  →  {} {}",
                    i + 1,
                    pct,
                    symbols.join(" → "),
                    format_amount(&leg.amount_in, src.decimals),
                    src.symbol,
                    format_amount(&leg.amount_out, dst.decimals),
                    dst.symbol
                );
            }
        }
        Outcome::NegativeCycle {
            cycle,
            pools_used,
            product_of_rates,
            amount_in,
            cycle_output,
            ..
        } => {
            let symbols: Vec<&str> = cycle
                .iter()
                .map(|addr| {
                    graph
                        .tokens
                        .iter()
                        .find(|t| t.address == *addr)
                        .map(|t| t.symbol.as_str())
                        .unwrap_or("?")
                })
                .collect();
            let entry = graph
                .tokens
                .iter()
                .find(|t| t.address == cycle[0])
                .expect("cycle[0] must be a graph token");
            println!("Arbitrage cycle detected (reachable from {})", src.symbol);
            println!("  hops:             {}", pools_used.len());
            println!(
                "  cycle:            {} → {}",
                symbols.join(" → "),
                symbols[0]
            );
            println!("  product of rates: {:.6}", product_of_rates);
            println!(
                "  input:            {} {}",
                format_amount(amount_in, entry.decimals),
                entry.symbol
            );
            println!(
                "  cycle output:     {} {}",
                format_amount(cycle_output, entry.decimals),
                entry.symbol
            );
        }
        Outcome::NoPath => {
            println!("No path from {} to {}", src.symbol, dst.symbol);
        }
    }
}

fn find_by_symbol<'a>(tokens: &'a [Token], symbol: &str) -> Result<&'a Token, String> {
    tokens
        .iter()
        .find(|t| t.symbol.eq_ignore_ascii_case(symbol))
        .ok_or_else(|| {
            let available: Vec<&str> = tokens.iter().map(|t| t.symbol.as_str()).collect();
            format!(
                "unknown token '{}'. available: {}",
                symbol,
                available.join(", ")
            )
        })
}

fn parse_amount(human: &str, decimals: u8) -> Result<U256, String> {
    let trimmed = human.trim();
    if trimmed.is_empty() {
        return Err("amount must be non-empty".into());
    }
    let (whole, frac) = match trimmed.split_once('.') {
        Some((w, f)) => (w, f),
        None => (trimmed, ""),
    };
    if !whole.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("invalid amount: {}", human));
    }
    let dec = decimals as usize;
    let frac_padded: String = frac
        .chars()
        .chain(std::iter::repeat('0'))
        .take(dec)
        .collect();
    let combined = format!("{}{}", whole, frac_padded);
    let trimmed_combined = combined.trim_start_matches('0');
    let repr = if trimmed_combined.is_empty() {
        "0"
    } else {
        trimmed_combined
    };
    U256::from_str(repr).map_err(|e| format!("parse U256: {e}"))
}

fn format_amount(value: &U256, decimals: u8) -> String {
    if value.is_zero() {
        return "0".to_string();
    }
    let s = value.to_string();
    let dec = decimals as usize;
    if s.len() <= dec {
        let padded = format!("{:0>width$}", s, width = dec);
        let trimmed_frac = padded.trim_end_matches('0');
        if trimmed_frac.is_empty() {
            "0".to_string()
        } else {
            format!("0.{}", trimmed_frac)
        }
    } else {
        let split = s.len() - dec;
        let whole = &s[..split];
        let frac = s[split..].trim_end_matches('0');
        if frac.is_empty() {
            whole.to_string()
        } else {
            format!("{}.{}", whole, frac)
        }
    }
}
