use mizan_metering::UsageChargeInput;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Microcredits(pub i64);

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RoutePrice {
    pub input_microcredits_per_1m_tokens: i64,
    pub output_microcredits_per_1m_tokens: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct UsageCharge {
    pub input: Microcredits,
    pub output: Microcredits,
    pub total: Microcredits,
}

pub fn calculate_usage_charge(input: UsageChargeInput, price: RoutePrice) -> UsageCharge {
    let input_charge = ceil_div(
        input.prompt_tokens as i128 * price.input_microcredits_per_1m_tokens as i128,
        1_000_000,
    );
    let output_charge = ceil_div(
        input.completion_tokens as i128 * price.output_microcredits_per_1m_tokens as i128,
        1_000_000,
    );

    UsageCharge {
        input: Microcredits(input_charge as i64),
        output: Microcredits(output_charge as i64),
        total: Microcredits((input_charge + output_charge) as i64),
    }
}

fn ceil_div(value: i128, divisor: i128) -> i128 {
    if value == 0 {
        return 0;
    }

    (value + divisor - 1) / divisor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_up_fractional_microcredit_charges() {
        let charge = calculate_usage_charge(
            UsageChargeInput {
                prompt_tokens: 1,
                completion_tokens: 1,
            },
            RoutePrice {
                input_microcredits_per_1m_tokens: 1,
                output_microcredits_per_1m_tokens: 1,
            },
        );

        assert_eq!(charge.input, Microcredits(1));
        assert_eq!(charge.output, Microcredits(1));
        assert_eq!(charge.total, Microcredits(2));
    }
}
