// SPDX-License-Identifier: MIT

//! Risk parameter management for credit lines.

#![warn(missing_docs)]

use crate::auth::require_admin_auth;
use crate::events::publish_risk_parameters_updated;
use crate::storage::{
    assert_not_paused, assert_ts_monotonic, persist_credit_line, rate_cfg_key, rate_formula_key,
};
use crate::types::{
    ContractError, CreditLineData, CreditStatus, RateChangeConfig, RateFormulaConfig,
};
use soroban_sdk::{Address, Env};

/// Maximum interest rate in basis points (100%).
pub const MAX_INTEREST_RATE_BPS: u32 = 10_000;

/// Maximum risk score on the normalized 0-100 scale.
pub const MAX_RISK_SCORE: u32 = 100;

/// Compute interest rate from risk score using piecewise-linear formula.
///
/// # Formula
/// ```text
/// raw_rate = base_rate_bps + (risk_score * slope_bps_per_score)
/// effective_rate = clamp(raw_rate, min_rate_bps, min(max_rate_bps, MAX_INTEREST_RATE_BPS))
/// ```
///
/// Uses saturating arithmetic to prevent overflow — if the multiplication
/// overflows u32, it saturates to `u32::MAX` and is then clamped by the
/// upper bound.
///
/// # Arguments
/// * `cfg` — The rate formula configuration.
/// * `risk_score` — The borrower's risk score (0–100).
///
/// # Returns
/// The computed effective interest rate in basis points.
pub fn compute_rate_from_score(cfg: &RateFormulaConfig, risk_score: u32) -> u32 {
    let raw = cfg
        .base_rate_bps
        .saturating_add(risk_score.saturating_mul(cfg.slope_bps_per_score));
    let upper = cfg.max_rate_bps.min(MAX_INTEREST_RATE_BPS);
    raw.clamp(cfg.min_rate_bps, upper)
}

/// Store admin-configured rate-change guardrails.
pub fn set_rate_change_limits(env: Env, max_rate_change_bps: u32, rate_change_min_interval: u64) {
    assert_not_paused(&env);
    require_admin_auth(&env);

    let cfg = RateChangeConfig {
        max_rate_change_bps,
        rate_change_min_interval,
    };
    env.storage().instance().set(&rate_cfg_key(&env), &cfg);
}

/// Retrieve the current rate-change guardrails, if configured.
pub fn get_rate_change_limits(env: Env) -> Option<RateChangeConfig> {
    env.storage().instance().get(&rate_cfg_key(&env))
}

/// Retrieve the dynamic rate-formula configuration, if configured.
pub fn get_rate_formula_config(env: Env) -> Option<RateFormulaConfig> {
    env.storage()
        .instance()
        .get::<_, RateFormulaConfig>(&rate_formula_key(&env))
}

/// Update the borrower's credit limit, risk score, and effective rate.
pub fn update_risk_parameters(
    env: Env,
    borrower: Address,
    credit_limit: i128,
    interest_rate_bps: u32,
    risk_score: u32,
) {
    assert_not_paused(&env);
    require_admin_auth(&env);

    let stored_line: CreditLineData = crate::storage::get_credit_line(&env, &borrower)
        .unwrap_or_else(|| env.panic_with_error(ContractError::CreditLineNotFound));
    let previous_utilized = stored_line.utilized_amount;

    let mut credit_line = crate::accrual::apply_accrual(&env, stored_line);

    if credit_limit < 0 {
        env.panic_with_error(ContractError::NegativeLimit);
    }
    if risk_score > MAX_RISK_SCORE {
        env.panic_with_error(ContractError::ScoreTooHigh);
    }

    let effective_rate = if let Some(formula_cfg) = get_rate_formula_config(env.clone()) {
        compute_rate_from_score(&formula_cfg, risk_score)
    } else {
        interest_rate_bps
    };

    if effective_rate > MAX_INTEREST_RATE_BPS {
        env.panic_with_error(ContractError::RateTooHigh);
    }

    if effective_rate != credit_line.interest_rate_bps {
        if let Some(cfg) = get_rate_change_limits(env.clone()) {
            let delta = effective_rate.abs_diff(credit_line.interest_rate_bps);
            if delta > cfg.max_rate_change_bps {
                env.panic_with_error(ContractError::RateTooHigh);
            }

            if cfg.rate_change_min_interval > 0 && credit_line.last_rate_update_ts != 0 {
                let now = env.ledger().timestamp();
                let elapsed = now.saturating_sub(credit_line.last_rate_update_ts);
                if elapsed < cfg.rate_change_min_interval {
                    env.panic_with_error(ContractError::RateTooHigh);
                }
            }
        }

        let new_ts = env.ledger().timestamp();
        assert_ts_monotonic(&env, credit_line.last_rate_update_ts, new_ts);
        credit_line.last_rate_update_ts = new_ts;
    }

    if credit_limit < credit_line.utilized_amount {
        credit_line.status = CreditStatus::Restricted;
    } else if credit_line.status == CreditStatus::Restricted {
        credit_line.status = CreditStatus::Active;
    }

    credit_line.credit_limit = credit_limit;
    credit_line.interest_rate_bps = effective_rate;
    credit_line.risk_score = risk_score;

    persist_credit_line(&env, &borrower, &credit_line, previous_utilized);
    publish_risk_parameters_updated(&env, &borrower, credit_limit, effective_rate, risk_score);
}

/// Return the current rate-change guardrail configuration, if any.
///
/// # Parameters
/// - `env`: The Soroban environment.
///
/// # Returns
/// `Some(RateChangeConfig)` if guardrails have been configured via
/// [`set_rate_change_limits`], or `None` if no configuration exists (meaning
/// rate changes are unconstrained).
pub fn get_rate_change_limits(env: Env) -> Option<RateChangeConfig> {
    env.storage().instance().get(&rate_cfg_key(&env))
}

/// Retrieve the rate formula configuration from instance storage, if set.
///
/// # Storage
/// - **Type**: Instance storage (shared TTL with all instance keys)
/// - **Key**: `Symbol("rate_form")`
/// - **TTL Note**: Shares instance TTL — extend alongside other instance keys.
pub fn get_rate_formula_config(env: Env) -> Option<RateFormulaConfig> {
    env.storage()
        .instance()
        .get::<_, RateFormulaConfig>(&rate_formula_key(&env))
}
