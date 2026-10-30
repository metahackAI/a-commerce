//! Messaging rules as pure functions: quiet hours (`messaging.quiet_hours`), whether a
//! shopper can be reached on a channel, the default subject line, how a recipient's name is
//! masked in previews, and template variables (`messaging.template.variables_declared`).

use acom_policy::PolicySet;
use chrono::NaiveTime;
use serde_json::Value;

/// `messaging.quiet_hours`: a window in store time, and the purposes it holds back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuietHours {
    pub start: NaiveTime,
    pub end: NaiveTime,
    pub applies_to: Vec<String>,
}

impl QuietHours {
    pub fn from_policy(policy: &PolicySet) -> Option<Self> {
        let q = policy.lookup("messaging.messaging.quiet_hours")?;
        let time = |k: &str| NaiveTime::parse_from_str(q.get(k)?.as_str()?, "%H:%M").ok();
        Some(Self {
            start: time("start")?,
            end: time("end")?,
            applies_to: q
                .get("applies_to")
                .and_then(Value::as_array)
                .map(|l| {
                    l.iter()
                        .filter_map(|w| w.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    /// Whether messages for this purpose (`marketing`, `transactional`) are held back.
    pub fn applies(&self, purpose: &str) -> bool {
        self.applies_to.iter().any(|p| p == purpose)
    }

    /// Whether a store-local time falls inside the window (which may cross midnight).
    pub fn contains(&self, local: NaiveTime) -> bool {
        if self.start <= self.end {
            local >= self.start && local < self.end
        } else {
            local >= self.start || local < self.end
        }
    }
}

/// Whether a shopper with these details can be reached on `channel`: email and sms need the
/// address or number on file, push and in-app messages an active account. A withdrawn
/// member is reached on no channel.
pub fn reachable(
    channel: &str,
    email: Option<&str>,
    telephone: Option<&str>,
    member_state: &str,
) -> bool {
    let present = |v: Option<&str>| v.is_some_and(|s| !s.trim().is_empty());
    if member_state == "withdrawn" {
        return false;
    }
    match channel {
        "email" => present(email),
        "sms" => present(telephone),
        "push" | "in_app" => member_state == "active",
        _ => false,
    }
}

/// Channels that show a subject line.
pub fn has_subject(channel: &str) -> bool {
    matches!(channel, "email" | "push")
}

/// The subject when none is given: what the message is about and the order number.
pub fn default_subject(kind: &str, order_number: Option<&str>) -> String {
    match (kind, order_number) {
        ("refund_done", Some(n)) => format!("Your refund for order {n} is on its way"),
        ("ask_for_photos" | "decline", Some(n)) => format!("About your return for order {n}"),
        (_, Some(n)) => format!("About your order {n}"),
        (_, None) => "A message from the store".to_owned(),
    }
}

/// `R*** P***`: the first letter of each name, so a preview shows who without showing the name.
pub fn masked_name(given: Option<&str>, family: Option<&str>, name: &str) -> String {
    let parts: Vec<&str> = match (given, family) {
        (Some(g), Some(f)) => vec![g, f],
        (Some(g), None) => vec![g],
        _ => name.split_whitespace().collect(),
    };
    parts
        .iter()
        .filter_map(|p| p.chars().next())
        .map(|c| format!("{}***", c.to_uppercase()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The variable names a text uses, written `{{name}}`, in order of first use.
pub fn variables_used(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            break;
        };
        let name = after[..end].trim().to_owned();
        if !name.is_empty() && !out.contains(&name) {
            out.push(name);
        }
        rest = &after[end + 2..];
    }
    out
}

/// Variables the texts use but the template does not declare.
pub fn undeclared(texts: &[&str], declared: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for t in texts {
        for v in variables_used(t) {
            if !declared.contains(&v) && !out.contains(&v) {
                out.push(v);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> PolicySet {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../policies");
        let texts: Vec<(&str, String)> = acom_policy::FILES
            .iter()
            .map(|f| {
                (
                    *f,
                    std::fs::read_to_string(format!("{dir}/{f}.yml")).unwrap(),
                )
            })
            .collect();
        PolicySet::from_defaults(texts.iter().map(|(f, t)| (*f, t.as_str()))).unwrap()
    }

    fn t(s: &str) -> NaiveTime {
        NaiveTime::parse_from_str(s, "%H:%M").unwrap()
    }

    #[test]
    fn quiet_hours_cross_midnight_and_hold_only_marketing() {
        let q = QuietHours::from_policy(&policy()).unwrap();
        assert_eq!((q.start, q.end), (t("21:00"), t("08:00")));
        assert!(q.contains(t("22:30")));
        assert!(q.contains(t("07:59")));
        assert!(!q.contains(t("08:00")));
        assert!(!q.contains(t("10:40")));
        assert!(q.applies("marketing"));
        assert!(!q.applies("transactional"));
        let day = QuietHours {
            start: t("12:00"),
            end: t("13:00"),
            applies_to: vec!["transactional".into()],
        };
        assert!(day.contains(t("12:30")));
        assert!(!day.contains(t("13:00")));
    }

    #[test]
    fn channels_need_a_way_to_reach_the_shopper() {
        assert!(reachable(
            "email",
            Some("riley.park@example.com"),
            None,
            "active"
        ));
        assert!(!reachable(
            "sms",
            Some("riley.park@example.com"),
            None,
            "active"
        ));
        assert!(reachable("sms", None, Some("+15555550100"), "active"));
        assert!(reachable("in_app", None, None, "active"));
        assert!(!reachable("push", None, None, "withdrawn"));
        assert!(!reachable("email", Some(" "), None, "active"));
        assert!(!reachable(
            "email",
            Some("gone@example.com"),
            None,
            "withdrawn"
        ));
    }

    #[test]
    fn scene_b2_messages_read_as_the_spec_shows() {
        assert_eq!(
            masked_name(Some("Riley"), Some("Park"), "Riley Park"),
            "R*** P***"
        );
        assert_eq!(masked_name(None, None, "Alex"), "A***");
        assert_eq!(
            default_subject("ask_for_photos", Some("20366")),
            "About your return for order 20366"
        );
        assert_eq!(
            default_subject("refund_done", Some("20402")),
            "Your refund for order 20402 is on its way"
        );
        assert!(has_subject("email"));
        assert!(!has_subject("sms"));
    }

    #[test]
    fn template_variables_must_be_declared() {
        let text = "Hi {{customer_name}}, {{ product_name }} is back. {{customer_name}}!";
        assert_eq!(variables_used(text), ["customer_name", "product_name"]);
        let declared = vec!["customer_name".to_owned()];
        assert_eq!(
            undeclared(&[text, "Back: {{size}}"], &declared),
            ["product_name", "size"]
        );
        assert!(undeclared(&["No variables here"], &declared).is_empty());
    }
}
