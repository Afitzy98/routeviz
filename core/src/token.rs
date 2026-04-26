use alloy_primitives::Address;
use serde::{Deserialize, Serialize};

// A token's role in the graph's topology. Hub tokens (ETH, stablecoins, BTC)
// carry most of the TVL and act as routing intermediates; spokes are the
// long tail of less-liquid tokens that mostly pair with hubs rather than
// with each other. Mirrors how real DEX liquidity concentrates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenKind {
    Hub,
    Spoke,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Token {
    pub address: Address,
    pub symbol: String,
    pub decimals: u8,
    pub true_price_usd: f64,
    pub kind: TokenKind,
}
