//! The arithmetic of explaining a change: the baseline taken from the compare days, the gap, the
//! amount of each cause, and what is left unexplained. Exact decimals, rounded once per figure;
//! the causes and the unexplained part always add up to the gap.

use rust_decimal::{Decimal, RoundingStrategy};

/// Places a rate is rounded to (`-0.2035`).
pub const RATE_PLACES: u32 = 4;

/// How the compare days are brought to the period's length: a sum over `compare_days` days
/// times `period_days / compare_days`. Four Thursdays for one Thursday is a quarter of their sum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scale {
    pub period_days: i64,
    pub compare_days: i64,
}

impl Scale {
    pub fn apply(self, sum: Decimal) -> Decimal {
        if self.compare_days == 0 {
            return Decimal::ZERO;
        }
        sum * Decimal::from(self.period_days) / Decimal::from(self.compare_days)
    }
}

/// What one cause's data says: what its part brought in the period, and the sum of what it
/// brought over the compare days.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CauseFigures {
    pub actual: Decimal,
    pub expected_sum: Decimal,
}

/// One cause that made it into the explanation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExplainedCause {
    /// Position of the cause in the input.
    pub index: usize,
    /// What its part brings on the comparison basis (scaled and rounded).
    pub expected: Decimal,
    /// `actual − expected`; negative when the cause took sales away.
    pub amount: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explained {
    pub baseline: Decimal,
    pub gap: Decimal,
    pub causes: Vec<ExplainedCause>,
    pub unexplained: Decimal,
    /// `gap / baseline`, or `None` when the baseline is zero.
    pub change_rate: Option<Decimal>,
}

pub fn round(v: Decimal, places: u32) -> Decimal {
    v.round_dp_with_strategy(places, RoundingStrategy::MidpointAwayFromZero)
}

/// Splits `period − baseline` into the causes and the rest. Causes whose amount rounds to zero
/// are left out; the others are ordered by how much they move the figure in the gap's direction
/// (the biggest drop first when sales fell), and at most `max_causes` are kept. Whatever the kept
/// causes do not cover is `unexplained`, so the parts add up to the gap exactly.
pub fn explain(
    period: Decimal,
    compare_sum: Decimal,
    scale: Scale,
    causes: &[CauseFigures],
    places: u32,
    max_causes: usize,
) -> Explained {
    let baseline = round(scale.apply(compare_sum), places);
    let gap = period - baseline;
    let mut kept: Vec<ExplainedCause> = causes
        .iter()
        .enumerate()
        .map(|(index, c)| {
            let expected = round(scale.apply(c.expected_sum), places);
            ExplainedCause {
                index,
                expected,
                amount: round(c.actual, places) - expected,
            }
        })
        .filter(|c| !c.amount.is_zero())
        .collect();
    // In the gap's direction first: the biggest drop when sales fell, the biggest gain when they
    // rose, the biggest effect either way when they did not move.
    let key = |c: &ExplainedCause| {
        if gap < Decimal::ZERO {
            c.amount
        } else if gap > Decimal::ZERO {
            -c.amount
        } else {
            -c.amount.abs()
        }
    };
    kept.sort_by(|a, b| key(a).cmp(&key(b)).then(a.index.cmp(&b.index)));
    kept.truncate(max_causes);
    let explained: Decimal = kept.iter().map(|c| c.amount).sum();
    let change_rate = (!baseline.is_zero()).then(|| round(gap / baseline, RATE_PLACES));
    Explained {
        baseline,
        gap,
        causes: kept,
        unexplained: gap - explained,
        change_rate,
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn one_of_four() -> Scale {
        Scale {
            period_days: 1,
            compare_days: 4,
        }
    }

    #[test]
    fn scene_b3_splits_the_thursday_drop_exactly() {
        // Thu Nov 5 against Oct 8, 15, 22 and 29: 22,700 + 23,400 + 22,900 + 22,600 = 91,600.
        let e = explain(
            dec("18240.00"),
            dec("91600.00"),
            one_of_four(),
            &[
                // The sold-out sizes made 10,800.00 over the four Thursdays and nothing on Nov 5.
                CauseFigures {
                    actual: Decimal::ZERO,
                    expected_sum: dec("10800.00"),
                },
                // The weekly email brought 5,200.00 over the four Thursdays; it did not go out.
                CauseFigures {
                    actual: Decimal::ZERO,
                    expected_sum: dec("5200.00"),
                },
            ],
            2,
            5,
        );
        assert_eq!(e.baseline, dec("22900.00"));
        assert_eq!(e.gap, dec("-4660.00"));
        let amounts: Vec<Decimal> = e.causes.iter().map(|c| c.amount).collect();
        assert_eq!(amounts, [dec("-2700.00"), dec("-1300.00")]);
        assert_eq!(e.causes[0].expected, dec("2700.00"));
        assert_eq!(e.unexplained, dec("-660.00"));
        assert_eq!(e.change_rate, Some(dec("-0.2035")));
        // Orders: 201 against 988 over four Thursdays.
        let o = explain(dec("201"), dec("988"), one_of_four(), &[], 2, 5);
        assert_eq!((o.baseline, o.gap), (dec("247"), dec("-46")));
    }

    #[test]
    fn the_parts_always_add_up_to_the_gap() {
        // Thirds do not divide evenly: every figure is rounded once and the rest is unexplained.
        let scale = Scale {
            period_days: 1,
            compare_days: 3,
        };
        let causes = [
            CauseFigures {
                actual: dec("10.00"),
                expected_sum: dec("100.00"),
            },
            CauseFigures {
                actual: dec("0.00"),
                expected_sum: dec("50.01"),
            },
            CauseFigures {
                actual: dec("7.77"),
                expected_sum: dec("0.00"),
            },
        ];
        let e = explain(dec("995.55"), dec("3333.33"), scale, &causes, 2, 5);
        assert_eq!(e.baseline, dec("1111.11"));
        let sum: Decimal = e.causes.iter().map(|c| c.amount).sum();
        assert_eq!(sum + e.unexplained, e.gap);
        assert_eq!(e.causes.len(), 3);
        assert_eq!(e.causes[0].amount, dec("-23.33"));
        assert_eq!(e.causes[1].amount, dec("-16.67"));
        assert_eq!(
            e.causes[2].amount,
            dec("7.77"),
            "a cause against the gap comes last"
        );
    }

    #[test]
    fn a_rise_lists_the_biggest_gain_first_and_cuts_to_max_causes() {
        let scale = Scale {
            period_days: 7,
            compare_days: 7,
        };
        let causes = [
            CauseFigures {
                actual: dec("300"),
                expected_sum: dec("100"),
            },
            CauseFigures {
                actual: dec("900"),
                expected_sum: dec("100"),
            },
            CauseFigures {
                actual: dec("100"),
                expected_sum: dec("100"),
            },
        ];
        let e = explain(dec("12000"), dec("10000"), scale, &causes, 2, 1);
        assert_eq!(e.gap, dec("2000"));
        assert_eq!(
            e.causes.len(),
            1,
            "the zero cause is dropped, the rest cut to one"
        );
        assert_eq!((e.causes[0].index, e.causes[0].amount), (1, dec("800")));
        assert_eq!(e.unexplained, dec("1200"));
    }

    #[test]
    fn no_baseline_means_no_rate() {
        let e = explain(dec("10"), Decimal::ZERO, one_of_four(), &[], 2, 5);
        assert_eq!(e.change_rate, None);
        assert_eq!(e.unexplained, dec("10"));
        assert_eq!(
            Scale {
                period_days: 1,
                compare_days: 0
            }
            .apply(dec("5")),
            Decimal::ZERO
        );
    }
}
