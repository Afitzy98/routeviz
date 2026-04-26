use alloy_primitives::{Address, U256};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pool {
    pub address: Address,
    pub token_a: Address,
    pub token_b: Address,
    pub reserve_a: U256,
    pub reserve_b: U256,
    pub fee_bps: u16,
    /// Venue tag, e.g. "Uniswap V2".
    pub venue: String,
}

impl Pool {
    pub fn reserves_for(&self, in_token: Address) -> (U256, U256) {
        if in_token == self.token_a {
            (self.reserve_a, self.reserve_b)
        } else if in_token == self.token_b {
            (self.reserve_b, self.reserve_a)
        } else {
            panic!("token {} is not part of pool {}", in_token, self.address);
        }
    }

    pub fn other_token(&self, in_token: Address) -> Address {
        if in_token == self.token_a {
            self.token_b
        } else if in_token == self.token_b {
            self.token_a
        } else {
            panic!("token {} is not part of pool {}", in_token, self.address);
        }
    }

    // Uniswap-V2 constant-product. Single floor-division at the end
    // matches on-chain semantics.
    pub fn output_amount(&self, in_token: Address, amount_in: U256) -> U256 {
        if amount_in.is_zero() {
            return U256::ZERO;
        }
        let (r_in, r_out) = self.reserves_for(in_token);
        let fee_denom = U256::from(10_000u64);
        let fee_mult = U256::from(10_000u64 - self.fee_bps as u64);
        let amount_in_with_fee = amount_in * fee_mult;
        let numerator = amount_in_with_fee * r_out;
        let denominator = r_in * fee_denom + amount_in_with_fee;
        numerator / denominator
    }

    pub fn marginal_rate(&self, in_token: Address) -> f64 {
        let (r_in, r_out) = self.reserves_for(in_token);
        let r_in_f = u256_to_f64(r_in);
        let r_out_f = u256_to_f64(r_out);
        let fee_mult = (10_000.0 - self.fee_bps as f64) / 10_000.0;
        r_out_f / r_in_f * fee_mult
    }

    pub fn log_weight(&self, in_token: Address) -> f64 {
        -self.marginal_rate(in_token).ln()
    }

    // Chain `output_amount` through a token-path + matching pool addresses.
    // Linear pool lookup — fine at visualiser scale.
    pub fn simulate_path(
        path: &[Address],
        pools_used: &[Address],
        pools: &[Pool],
        amount_in: U256,
    ) -> U256 {
        assert_eq!(path.len(), pools_used.len() + 1);
        let mut amount = amount_in;
        for (hop, pool_addr) in pools_used.iter().enumerate() {
            let pool = pools
                .iter()
                .find(|p| p.address == *pool_addr)
                .expect("simulate_path: pool address not found in pools slice");
            amount = pool.output_amount(path[hop], amount);
        }
        amount
    }
}

// Lossy — display + routing layer only, never feeds back into U256 swap math.
fn u256_to_f64(x: U256) -> f64 {
    let limbs = x.as_limbs();
    (limbs[0] as f64)
        + (limbs[1] as f64) * 2f64.powi(64)
        + (limbs[2] as f64) * 2f64.powi(128)
        + (limbs[3] as f64) * 2f64.powi(192)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(x: u64) -> U256 {
        U256::from(x)
    }

    // Deterministic test addresses: addr(1) = 0x0101…01, addr(2) = 0x0202…02, etc.
    fn addr(byte: u8) -> Address {
        Address::from([byte; 20])
    }

    fn sym_pool(pool_byte: u8, a: Address, b: Address, reserve: U256, fee_bps: u16) -> Pool {
        Pool {
            address: addr(pool_byte),
            token_a: a,
            token_b: b,
            reserve_a: reserve,
            reserve_b: reserve,
            fee_bps,
            venue: "Test".to_string(),
        }
    }

    #[test]
    fn output_amount_matches_hand_computation() {
        // Reserves (1000, 1000), fee 0. Swap 100 of token_a.
        //   amount_in_with_fee = 100 * 10_000 = 1_000_000
        //   numerator          = 1_000_000 * 1000 = 1_000_000_000
        //   denominator        = 1000 * 10_000 + 1_000_000 = 11_000_000
        //   amount_out         = 1_000_000_000 / 11_000_000 = 90 (floor of 90.909…)
        let a = addr(1);
        let b = addr(2);
        let pool = sym_pool(0xAA, a, b, u(1000), 0);
        assert_eq!(pool.output_amount(a, u(100)), u(90));
    }

    #[test]
    fn zero_input_gives_zero_output() {
        let a = addr(1);
        let b = addr(2);
        let pool = Pool {
            address: addr(0xAA),
            token_a: a,
            token_b: b,
            reserve_a: u(1_000_000),
            reserve_b: u(500_000),
            fee_bps: 30,
            venue: "Test".to_string(),
        };
        assert_eq!(pool.output_amount(a, U256::ZERO), U256::ZERO);
    }

    #[test]
    fn k_strictly_grows_after_swap_with_fee() {
        let a = addr(1);
        let b = addr(2);
        let pool = sym_pool(0xAA, a, b, u(1_000_000), 30);
        let amount_in = u(10_000);
        let amount_out = pool.output_amount(a, amount_in);
        let new_reserve_a = pool.reserve_a + amount_in;
        let new_reserve_b = pool.reserve_b - amount_out;
        assert!(new_reserve_a * new_reserve_b > pool.reserve_a * pool.reserve_b);
    }

    #[test]
    fn zero_fee_k_grows_only_by_rounding_slack() {
        // At zero fee the product is preserved up to a floor-division residue,
        // bounded by the new denominator (r_in + amount_in_with_fee).
        let a = addr(1);
        let b = addr(2);
        let pool = sym_pool(0xAA, a, b, u(1_000_000), 0);
        let amount_in = u(10_000);
        let amount_out = pool.output_amount(a, amount_in);
        let new_reserve_a = pool.reserve_a + amount_in;
        let new_reserve_b = pool.reserve_b - amount_out;
        let slack = new_reserve_a * new_reserve_b - pool.reserve_a * pool.reserve_b;
        assert!(slack <= new_reserve_a);
    }

    #[test]
    fn simulate_path_single_hop_equals_output_amount() {
        let a = addr(1);
        let b = addr(2);
        let pool = Pool {
            address: addr(0xAA),
            token_a: a,
            token_b: b,
            reserve_a: u(1_000_000),
            reserve_b: u(500_000),
            fee_bps: 30,
            venue: "Test".to_string(),
        };
        let pools = vec![pool.clone()];
        let amount_in = u(1_000);
        let direct = pool.output_amount(a, amount_in);
        let simulated = Pool::simulate_path(&[a, b], &[pool.address], &pools, amount_in);
        assert_eq!(direct, simulated);
    }

    #[test]
    fn three_hop_round_trip_zero_fee_loses_at_most_rounding() {
        let t0 = addr(1);
        let t1 = addr(2);
        let t2 = addr(3);
        let reserve = u(1_000_000_000);
        let pools = vec![
            sym_pool(0xA0, t0, t1, reserve, 0),
            sym_pool(0xA1, t1, t2, reserve, 0),
            sym_pool(0xA2, t2, t0, reserve, 0),
        ];
        let pool_addrs: Vec<Address> = pools.iter().map(|p| p.address).collect();
        let amount_in = u(1_000);
        let out = Pool::simulate_path(&[t0, t1, t2, t0], &pool_addrs, &pools, amount_in);
        assert!(out <= amount_in);
        assert!(amount_in - out < u(10));
    }

    #[test]
    fn three_hop_round_trip_with_fees_loses_approximately_three_fees() {
        // 30 bps × 3 hops → target ~0.9 % loss.
        let t0 = addr(1);
        let t1 = addr(2);
        let t2 = addr(3);
        let reserve = u(1_000_000_000);
        let pools = vec![
            sym_pool(0xA0, t0, t1, reserve, 30),
            sym_pool(0xA1, t1, t2, reserve, 30),
            sym_pool(0xA2, t2, t0, reserve, 30),
        ];
        let pool_addrs: Vec<Address> = pools.iter().map(|p| p.address).collect();
        let amount_in = u(1_000_000);
        let out = Pool::simulate_path(&[t0, t1, t2, t0], &pool_addrs, &pools, amount_in);
        assert!(out < amount_in);
        // 1_000_000 * 0.997^3 ≈ 991_027 — allow a 1 % band.
        assert!(out >= u(985_000) && out <= u(995_000));
    }

    #[test]
    fn marginal_rate_and_log_weight_are_finite_and_consistent() {
        let a = addr(1);
        let b = addr(2);
        let pool = Pool {
            address: addr(0xAA),
            token_a: a,
            token_b: b,
            reserve_a: u(1_000_000),
            reserve_b: u(2_000_000),
            fee_bps: 30,
            venue: "Test".to_string(),
        };
        let rate = pool.marginal_rate(a);
        assert!(rate.is_finite());
        // (2_000_000 / 1_000_000) * 0.997 = 1.994
        assert!((rate - 1.994).abs() < 1e-9);
        assert_eq!(pool.log_weight(a), -rate.ln());
    }

    #[test]
    fn swapping_opposite_direction_uses_reversed_reserves() {
        let a = addr(1);
        let b = addr(2);
        let pool = Pool {
            address: addr(0xAA),
            token_a: a,
            token_b: b,
            reserve_a: u(1_000_000),
            reserve_b: u(500_000),
            fee_bps: 0,
            venue: "Test".to_string(),
        };
        let ab = pool.output_amount(a, u(10_000));
        let ba = pool.output_amount(b, u(10_000));
        assert!(ba > ab);
    }

    #[test]
    fn output_amount_at_wei_scale() {
        // 1_000 WETH vs 3_000_000 USDC (both scaled to 18 decimals for the test).
        let weth = addr(1);
        let usdc = addr(2);
        let ten_pow_18 = U256::from(1_000_000_000_000_000_000u64);
        let pool = Pool {
            address: addr(0xAA),
            token_a: weth,
            token_b: usdc,
            reserve_a: U256::from(1_000u64) * ten_pow_18,
            reserve_b: U256::from(3_000_000u64) * ten_pow_18,
            fee_bps: 30,
            venue: "Test".to_string(),
        };
        // 1 WETH in → roughly 3_000 USDC out, minus slippage and fee.
        let out = pool.output_amount(weth, ten_pow_18);
        let approx_out_f = u256_to_f64(out) / u256_to_f64(ten_pow_18);
        assert!(approx_out_f > 2_980.0 && approx_out_f < 3_000.0);
    }

    #[test]
    fn other_token_resolves_both_directions() {
        let a = addr(3);
        let b = addr(7);
        let pool = sym_pool(0xAA, a, b, u(1), 0);
        assert_eq!(pool.other_token(a), b);
        assert_eq!(pool.other_token(b), a);
    }

    #[test]
    #[should_panic]
    fn other_token_panics_for_unrelated_token() {
        sym_pool(0xAA, addr(3), addr(7), u(1), 0).other_token(addr(42));
    }
}
