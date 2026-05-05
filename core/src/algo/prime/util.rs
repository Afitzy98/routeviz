use alloy_primitives::U256;

// Lossy U256 → f64 used wherever we need to feed a U256 into floating-point
// math (marginal-rate derivatives, gradient-method comparisons, etc.). Never
// fed back into U256 swap arithmetic.
pub(super) fn u256_to_f64(v: U256) -> f64 {
    let limbs = v.as_limbs();
    limbs[0] as f64
        + limbs[1] as f64 * 1.8446744073709552e19   // 2^64
        + limbs[2] as f64 * 3.402823669209385e38    // 2^128
        + limbs[3] as f64 * 6.277_101_735_386_68e57 // 2^192
}
