use alloy_primitives::U256;

// Uniswap-V2-ish gas costs. 1-hop swap = 21k + 30k + 90k = 141k gas;
// each additional hop adds 90k; each additional leg adds 30k for its
// router call.
pub const BASE_TX_GAS: u64 = 21_000;
pub const ROUTER_OVERHEAD_GAS: u64 = 30_000;
pub const PER_HOP_GAS: u64 = 90_000;

pub const ETH_PRICE_USD: f64 = 3000.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GasModel {
    /// Gwei. Zero disables gas accounting.
    pub gas_price_gwei: f64,
}

impl Default for GasModel {
    fn default() -> Self {
        Self::off()
    }
}

impl GasModel {
    pub fn off() -> Self {
        Self {
            gas_price_gwei: 0.0,
        }
    }

    pub fn at_gwei(gwei: f64) -> Self {
        Self {
            gas_price_gwei: gwei.max(0.0),
        }
    }

    pub fn enabled(&self) -> bool {
        self.gas_price_gwei > 0.0
    }

    pub fn gas_units(&self, num_legs: usize, total_hops: usize) -> u64 {
        if !self.enabled() || num_legs == 0 || total_hops == 0 {
            return 0;
        }
        BASE_TX_GAS + (num_legs as u64) * ROUTER_OVERHEAD_GAS + (total_hops as u64) * PER_HOP_GAS
    }

    /// gas_units → ETH (× gwei × 1e-9) → USD (× ETH_PRICE_USD) → dst
    /// base units (÷ dst_price_usd × 10^decimals).
    pub fn gas_to_dst_token(&self, gas_units: u64, dst_price_usd: f64, dst_decimals: u8) -> U256 {
        if !self.enabled() || gas_units == 0 {
            return U256::ZERO;
        }
        if !dst_price_usd.is_finite() || dst_price_usd <= 0.0 {
            return U256::ZERO;
        }
        if dst_decimals > 30 {
            return U256::ZERO;
        }
        let gas_eth = (gas_units as f64) * self.gas_price_gwei * 1e-9;
        let gas_usd = gas_eth * ETH_PRICE_USD;
        let dst_amount_human = gas_usd / dst_price_usd;
        let scale = 10f64.powi(dst_decimals as i32);
        let base_units = dst_amount_human * scale;
        if !base_units.is_finite() || base_units <= 0.0 {
            return U256::ZERO;
        }
        if base_units < (u128::MAX as f64) {
            U256::from(base_units as u128)
        } else {
            U256::MAX
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_model_returns_zero() {
        let g = GasModel::off();
        assert_eq!(g.gas_units(1, 1), 0);
        assert_eq!(g.gas_to_dst_token(141_000, 3000.0, 18), U256::ZERO);
    }

    #[test]
    fn gas_units_match_realistic_v2_swap() {
        let g = GasModel::at_gwei(20.0);
        assert_eq!(g.gas_units(1, 1), 141_000);
        assert_eq!(g.gas_units(1, 2), 231_000);
        assert_eq!(g.gas_units(5, 10), 1_071_000);
    }

    #[test]
    fn one_hop_gas_at_20_gwei_is_about_eight_dollars_in_usdc() {
        // 141,000 × 20 × 1e-9 × 3000 / 1.0 × 10^6 = 8,460,000 base units.
        let g = GasModel::at_gwei(20.0);
        let cost = g.gas_to_dst_token(141_000, 1.0, 6);
        // Allow ±1 base unit of float-cast slop.
        let expected = U256::from(8_460_000u64);
        let diff = if cost > expected {
            cost - expected
        } else {
            expected - cost
        };
        assert!(diff <= U256::from(1u64), "cost = {cost}, expected ≈ 8.46M");
    }

    #[test]
    fn one_hop_gas_in_weth_at_18_decimals() {
        // 141,000 × 20 × 1e-9 = 2.82e-3 ETH ≈ 2.82e15 base units.
        let g = GasModel::at_gwei(20.0);
        let cost = g.gas_to_dst_token(141_000, 3000.0, 18);
        let expected = U256::from(2_820_000_000_000_000u64);
        let diff = if cost > expected {
            cost - expected
        } else {
            expected - cost
        };
        // f64 → u128 cast can drift a handful of base units.
        assert!(
            diff <= U256::from(1_000_000_000u64),
            "cost = {cost}, expected ≈ 2.82e15"
        );
    }
}
