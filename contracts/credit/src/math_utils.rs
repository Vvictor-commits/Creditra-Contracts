// SPDX-License-Identifier: MIT

//! Pure integer arithmetic helpers used across the credit contract.

#![warn(missing_docs)]

// # Fixed-Point Interest Math Utilities
//
// This module provides deterministic, integer-only arithmetic helpers for
// computing interest accruals inside the Creditra credit contract.
//
// ## Scaling Factor
//
// All intermediate products are scaled by `SCALE = 10^18` before division so
// that the final result retains sub-unit precision up to 18 decimal places.
// The caller chooses whether the remainder is discarded (floor) or rounded up
// (ceiling) via the [`Rounding`] enum.
//
// ## Basis Points
//
// Interest rates are expressed in **basis points** (bps), where
// `1 bps = 0.01% = 1 / 10_000`.  The annual rate in bps is therefore divided
// by `BPS_DENOMINATOR = 10_000` when computing the fractional rate.
//
// ## Annual Seconds
//
// Time is measured in ledger seconds.  One Julian year is defined as
// `SECONDS_PER_YEAR = 31_557_600` (365.25 × 86 400), matching the convention
// used by most on-chain interest protocols.
//
// ## Overflow Safety
//
// The prorate helper promotes all operands to `u128` before multiplying.
// The worst-case intermediate product is:
//
// ```text
// principal  ≤ i128::MAX  ≈ 1.7 × 10^38
// rate_bps   ≤ 10_000
// time_delta ≤ u64::MAX   ≈ 1.8 × 10^19
// SCALE      = 10^18
// ```
//
// `principal × rate_bps × time_delta` can reach ~3 × 10^61, which overflows
// `u128` (max ~3.4 × 10^38).  To prevent this the multiplication is split
// into two checked steps:
//
// 1. `a = principal × rate_bps`  — fits in u128 for any realistic principal
//    (≤ 10^28 × 10^4 = 10^32 < 10^38).
// 2. `b = a × time_delta`        — checked; panics on overflow.
//
// The denominator `BPS_DENOMINATOR × SECONDS_PER_YEAR` is pre-computed as a
// `u128` constant so the final division is a single operation.

/// Scaling factor used for fixed-point intermediate arithmetic (10^18).
pub const SCALE: u128 = 1_000_000_000_000_000_000_u128;

/// Number of basis points in 100%.
pub const BPS_DENOMINATOR: u128 = 10_000;

/// Seconds in a 365-day year.
pub const SECONDS_PER_YEAR: u128 = 31_536_000;

const BPS_YEAR_DENOMINATOR: u128 = BPS_DENOMINATOR * SECONDS_PER_YEAR;

/// Rounding mode for integer division helpers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rounding {
    /// Truncate the remainder.
    Floor,
    /// Round up when a non-zero remainder exists.
    Ceil,
}

/// Multiply `value` by `numerator` and divide by `denominator`.
///
/// The result is rounded according to `rounding`.
pub fn mul_div(value: u128, numerator: u128, denominator: u128, rounding: Rounding) -> u128 {
    assert!(denominator != 0, "math_utils: division by zero");

    let product = value
        .checked_mul(numerator)
        .expect("math_utils: multiplication overflow");
    let quotient = product / denominator;

    match rounding {
        Rounding::Floor => quotient,
        Rounding::Ceil => {
            if product % denominator == 0 {
                quotient
            } else {
                quotient.checked_add(1).expect("math_utils: ceil overflow")
            }
        }
    }
}

/// Multiply `amount` by `SCALE`.
pub fn scale_up(amount: u128) -> u128 {
    amount
        .checked_mul(SCALE)
        .expect("math_utils: scale_up overflow")
}

/// Divide `amount` by `SCALE` using the requested rounding mode.
pub fn scale_down(amount: u128, rounding: Rounding) -> u128 {
    let quotient = amount / SCALE;

    match rounding {
        Rounding::Floor => quotient,
        Rounding::Ceil => {
            if amount % SCALE == 0 {
                quotient
            } else {
                quotient
                    .checked_add(1)
                    .expect("math_utils: scale_down ceil overflow")
            }
        }
    }
}

/// Apply a basis-point rate to an amount.
pub fn apply_bps(amount: u128, rate_bps: u32, rounding: Rounding) -> u128 {
    mul_div(amount, rate_bps as u128, BPS_DENOMINATOR, rounding)
}

/// Compute prorated interest for an elapsed time interval.
pub fn prorate_interest(
    principal: u128,
    rate_bps: u32,
    elapsed_secs: u64,
    rounding: Rounding,
) -> u128 {
    if principal == 0 || rate_bps == 0 || elapsed_secs == 0 {
        return 0;
    }

    let step1 = principal
        .checked_mul(rate_bps as u128)
        .expect("math_utils: prorate step1 overflow");
    let step2 = step1
        .checked_mul(elapsed_secs as u128)
        .expect("math_utils: prorate step2 overflow");

    let quotient = step2 / BPS_YEAR_DENOMINATOR;
    match rounding {
        Rounding::Floor => quotient,
        Rounding::Ceil => {
            if step2 % BPS_YEAR_DENOMINATOR == 0 {
                quotient
            } else {
                quotient
                    .checked_add(1)
                    .expect("math_utils: prorate ceil overflow")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul_div_basic() {
        assert_eq!(mul_div(1_000, 300, 10_000, Rounding::Floor), 30);
    }

    #[test]
    fn mul_div_truncates_toward_zero() {
        // 7 * 1 / 3 = 2.33… → 2
        assert_eq!(mul_div(7, 1, 3, Rounding::Floor), 2);
    }

    #[test]
    fn mul_div_identity_denominator() {
        assert_eq!(mul_div(42, 1, 1, Rounding::Floor), 42);
    }

    #[test]
    // ── apply_bps ────────────────────────────────────────────────────────────

    #[test]
    fn apply_bps_half_percent_truncates() {
        assert_eq!(apply_bps(200, 50, Rounding::Floor), 1);
    }

    #[test]
    fn apply_bps_sub_unit_truncates_to_zero() {
        assert_eq!(apply_bps(50, 1, Rounding::Floor), 0);
    }
    // ── mul_div ───────────────────────────────────────────────────────────────

    #[test]
    fn mul_div_exact_floor() {
        // 1 000 × 3 / 10 = 300 exactly
        assert_eq!(mul_div(1_000, 3, 10, Rounding::Floor), 300);
        assert_eq!(mul_div(1_001, 3, 10, Rounding::Floor), 300);
        assert_eq!(mul_div(1_001, 3, 10, Rounding::Ceil), 301);
    }

    #[test]
    fn apply_bps_matches_basic_basis_point_math() {
        assert_eq!(apply_bps(10_000, 300, Rounding::Floor), 300);
    }

    #[test]
    fn apply_bps_full_rate() {
        assert_eq!(apply_bps(500, 10_000, Rounding::Floor), 500);
        // 10 000 tokens × 10 000 bps (100 %) = 10 000 tokens
        assert_eq!(apply_bps(10_000, 10_000, Rounding::Floor), 10_000);
    }

    #[test]
    fn apply_bps_zero_rate() {
        assert_eq!(apply_bps(1_000_000, 0, Rounding::Floor), 0);
    }

    // ── prorate_interest ─────────────────────────────────────────────────────

    #[test]
    fn prorate_interest_one_day() {
        // 5% annual on 1_000_000 for 1 day
        assert_eq!(
            prorate_interest(1_000_000, 500, 86_400, Rounding::Floor),
            137
        );
    }

    #[test]
    fn prorate_interest_zero_elapsed() {
        assert_eq!(prorate_interest(1_000_000, 500, 0, Rounding::Floor), 0);
        assert_eq!(apply_bps(1_000_000, 0, Rounding::Floor), 0);
        assert_eq!(apply_bps(1_000_000, 0, Rounding::Ceil), 0);
    }

    #[test]
    fn apply_bps_zero_amount() {
        assert_eq!(apply_bps(0, 300, Rounding::Floor), 0);
        assert_eq!(apply_bps(0, 300, Rounding::Ceil), 0);
    }

    #[test]
    fn apply_bps_one_bps_small_amount_floor() {
        // 1 token × 1 bps = 0.0001 → floor → 0
        assert_eq!(apply_bps(1, 1, Rounding::Floor), 0);
        assert_eq!(apply_bps(1, 1, Rounding::Ceil), 1);
    }

    #[test]
    fn apply_bps_one_bps_threshold_floor() {
        // 10 000 tokens × 1 bps = 1 token exactly
        assert_eq!(apply_bps(10_000, 1, Rounding::Floor), 1);
    }

    #[test]
    fn apply_bps_large_amount() {
        // i128::MAX as u128 × 1 bps / 10_000
        let large: u128 = i128::MAX as u128;
        let expected = large / 10_000;
        assert_eq!(apply_bps(large, 1, Rounding::Floor), expected);
    }

    // ── prorate_interest ──────────────────────────────────────────────────────

    #[test]
    fn prorate_interest_one_full_year_floor() {
        // 10 000 tokens at 300 bps for exactly one year → 300 tokens
        let interest = prorate_interest(10_000, 300, SECONDS_PER_YEAR as u64, Rounding::Floor);
        assert_eq!(interest, 300);
    }

    #[test]
    fn prorate_interest_one_full_year_ceil() {
        // Exact result → ceil should equal floor
        let interest = prorate_interest(10_000, 300, SECONDS_PER_YEAR as u64, Rounding::Ceil);
        assert_eq!(interest, 300);
    }

    #[test]
    fn prorate_interest_half_year() {
        // 10 000 tokens at 300 bps for half a year → 150 tokens
        let half_year = (SECONDS_PER_YEAR / 2) as u64;
        let interest = prorate_interest(10_000, 300, half_year, Rounding::Floor);
        assert_eq!(interest, 150);
    }

    #[test]
    fn prorate_interest_small_principal_one_day_floor() {
        // 10 000 tokens at 300 bps for one day
        // = 10_000 × 300 × 86_400 / 315_576_000_000
        // = 259_200_000 / 315_576_000_000 ≈ 0.000821 → floor → 0
        let interest = prorate_interest(10_000, 300, 86_400, Rounding::Floor);
        assert_eq!(interest, 0);
    }

    #[test]
    fn prorate_interest_one_day_ceil() {
        // Same as above but ceil → 1
        let interest = prorate_interest(10_000, 300, 86_400, Rounding::Ceil);
        assert_eq!(interest, 1);
    }

    #[test]
    fn prorate_interest_zero_principal() {
        assert_eq!(prorate_interest(0, 500, 86_400, Rounding::Floor), 0);
    }

    #[test]
    fn prorate_interest_full_year() {
        // 10% on 100_000 for exactly 1 year = 10_000
        assert_eq!(
            prorate_interest(100_000, 1_000, 31_536_000, Rounding::Floor),
            10_000
        );
    }

    #[test]
    fn prorate_interest_one_hour() {
        // 5% on 1_000_000 for 3_600 s ≈ 5
        assert_eq!(
            prorate_interest(1_000_000, 500, 3_600, Rounding::Floor),
            5
        );
        assert_eq!(prorate_interest(0, 300, 86_400, Rounding::Floor), 0);
        assert_eq!(prorate_interest(10_000, 0, 86_400, Rounding::Floor), 0);
        assert_eq!(prorate_interest(10_000, 300, 0, Rounding::Floor), 0);
    }

    #[test]
    fn prorate_interest_matches_one_year_example() {
        assert_eq!(
            prorate_interest(10_000, 300, SECONDS_PER_YEAR as u64, Rounding::Floor),
            300
        );
    }

    #[test]
    fn scale_helpers_round_trip_on_exact_values() {
        let scaled = scale_up(42);
        assert_eq!(scale_down(scaled, Rounding::Floor), 42);
        assert_eq!(scale_down(scaled, Rounding::Ceil), 42);
    }
}
