//! Password rules (`customer.yml` `password`) and the sign-in lockout. Only the person-only
//! endpoints call these: no tool takes, shows or sets a password. Passwords are hashed with
//! argon2id (`acom_auth::password`) and never logged.

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

/// The password rules of the store's policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rules {
    pub min_length: usize,
    pub max_length: usize,
    /// `letter`, `upper`, `lower`, `digit`, `symbol`.
    pub require_classes: Vec<String>,
    /// `contains_login_id`, `sequential_run_3`, `known_breached`.
    pub reject: Vec<String>,
    pub history_block: usize,
}

impl Rules {
    /// Reads `customer.password` (its values are the store's; nothing here is a default).
    pub(crate) fn from_policy(p: &Value) -> Option<Self> {
        let words = |k: &str| -> Vec<String> {
            p.get(k)
                .and_then(Value::as_array)
                .map(|l| {
                    l.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };
        let n = |k: &str| {
            p.get(k)
                .and_then(Value::as_u64)
                .and_then(|v| usize::try_from(v).ok())
        };
        Some(Self {
            min_length: n("min_length")?,
            max_length: n("max_length")?,
            require_classes: words("require_classes"),
            reject: words("reject"),
            history_block: n("history_block").unwrap_or(0),
        })
    }

    /// The rules a new password breaks, by name (`min_length`, `require_digit`,
    /// `contains_login_id` ...); empty when it is acceptable. `known_breached` needs a breach
    /// list provider, which the store has not connected, so it is not checked.
    pub(crate) fn broken(&self, password: &str, identifiers: &[&str]) -> Vec<String> {
        let mut out = Vec::new();
        let len = password.chars().count();
        if len < self.min_length {
            out.push("min_length".to_owned());
        }
        if len > self.max_length {
            out.push("max_length".to_owned());
        }
        for class in &self.require_classes {
            let has = match class.as_str() {
                "letter" => password.chars().any(char::is_alphabetic),
                "upper" => password.chars().any(char::is_uppercase),
                "lower" => password.chars().any(char::is_lowercase),
                "digit" => password.chars().any(|c| c.is_ascii_digit()),
                "symbol" => password
                    .chars()
                    .any(|c| !c.is_alphanumeric() && !c.is_whitespace()),
                _ => true,
            };
            if !has {
                out.push(format!("require_{class}"));
            }
        }
        let lower = password.to_lowercase();
        for rule in &self.reject {
            let broken = match rule.as_str() {
                "contains_login_id" => identifiers.iter().any(|id| {
                    let local = id.split('@').next().unwrap_or("").to_lowercase();
                    local.chars().count() >= 3 && lower.contains(&local)
                }),
                "sequential_run_3" => has_run(&lower, 3),
                _ => false,
            };
            if broken {
                out.push(rule.clone());
            }
        }
        out
    }
}

/// Whether `s` has `n` characters in a row that go up or down by one (`abc`, `321`).
fn has_run(s: &str, n: usize) -> bool {
    let c: Vec<u32> = s.chars().map(u32::from).collect();
    c.windows(n).any(|w| {
        let up = w.windows(2).all(|p| p[1] == p[0] + 1);
        let down = w.windows(2).all(|p| p[0] == p[1] + 1);
        (up || down)
            && w.iter()
                .all(|x| char::from_u32(*x).is_some_and(char::is_alphanumeric))
    })
}

/// Whether the password equals the current one or one of the last `history_block` ones.
pub(crate) fn reused(
    password: &str,
    current: Option<&str>,
    history: &[String],
    block: usize,
) -> bool {
    current
        .into_iter()
        .chain(history.iter().map(String::as_str))
        .take(block.max(1))
        .any(|h| acom_auth::password::verify(password, h))
}

/// The history to keep after a change: the old hash first, at most `block` entries.
pub(crate) fn next_history(current: Option<&str>, history: &[String], block: usize) -> Vec<String> {
    current
        .map(str::to_owned)
        .into_iter()
        .chain(history.iter().cloned())
        .take(block)
        .collect()
}

/// The lockout policy (`password.lockout`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Lockout {
    pub failed_attempts: i32,
    pub window: Duration,
    pub lock: Duration,
}

impl Lockout {
    pub(crate) fn from_policy(p: &Value) -> Option<Self> {
        let n = |k: &str| p.get(k).and_then(Value::as_i64);
        Some(Self {
            failed_attempts: i32::try_from(n("failed_attempts")?).ok()?,
            window: Duration::minutes(n("window_minutes")?),
            lock: Duration::minutes(n("lock_minutes")?),
        })
    }

    /// After one more wrong password at `now`: the new count, the start of the window, and the
    /// time the account is locked until (when this attempt reaches the limit).
    pub(crate) fn after_failure(
        &self,
        count: i32,
        first: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> (i32, DateTime<Utc>, Option<DateTime<Utc>>) {
        let (count, first) = match first {
            Some(f) if now - f <= self.window => (count + 1, f),
            _ => (1, now),
        };
        let locked = (count >= self.failed_attempts).then(|| now + self.lock);
        (count, first, locked)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn rules() -> Rules {
        Rules::from_policy(&json!({"min_length": 8, "max_length": 64,
            "require_classes": ["letter", "digit"],
            "reject": ["contains_login_id", "sequential_run_3", "known_breached"],
            "history_block": 3}))
        .unwrap()
    }

    #[test]
    fn names_every_broken_rule() {
        let r = rules();
        assert!(
            r.broken("harbor-lantern-42", &["alex.kim@example.com"])
                .is_empty()
        );
        assert_eq!(r.broken("short1", &[]), ["min_length"]);
        assert_eq!(r.broken("onlyletters", &[]), ["require_digit"]);
        assert_eq!(
            r.broken("alex.kim-2026", &["alex.kim@example.com"]),
            ["contains_login_id"]
        );
        assert_eq!(r.broken("river-abc-77", &[]), ["sequential_run_3"]);
        assert_eq!(r.broken("river-951-77", &[]), Vec::<String>::new());
    }

    #[test]
    fn keeps_history_and_counts_failures_in_a_window() {
        assert_eq!(
            next_history(Some("h3"), &["h2".into(), "h1".into(), "h0".into()], 3),
            ["h3", "h2", "h1"]
        );
        let l = Lockout::from_policy(
            &json!({"failed_attempts": 5, "window_minutes": 15, "lock_minutes": 30}),
        )
        .unwrap();
        let t0 = DateTime::<Utc>::UNIX_EPOCH;
        assert_eq!(l.after_failure(0, None, t0), (1, t0, None));
        let (n, f, locked) = l.after_failure(4, Some(t0), t0 + Duration::minutes(10));
        assert_eq!((n, f), (5, t0));
        assert_eq!(locked, Some(t0 + Duration::minutes(40)));
        // Outside the window the count starts again.
        let late = t0 + Duration::minutes(20);
        assert_eq!(l.after_failure(4, Some(t0), late), (1, late, None));
    }
}
