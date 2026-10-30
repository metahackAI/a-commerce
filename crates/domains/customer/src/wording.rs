//! Pure text rules: masking personal data for operators, the one-line memory texts shoppers see,
//! the backstop checks on what may be remembered, and postal code formats.

/// `Alex Kim` → `A*** K***`. Operators see this unless they unmask with a reason.
pub(crate) fn mask_name(name: &str) -> String {
    name.split_whitespace()
        .map(|w| match w.chars().next() {
            Some(c) => format!("{c}***"),
            None => String::new(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `alex.kim@example.com` → `a***@example.com`.
pub(crate) fn mask_email(email: &str) -> String {
    match email.split_once('@') {
        Some((local, domain)) => {
            let first = local.chars().next().map(String::from).unwrap_or_default();
            format!("{first}***@{domain}")
        }
        None => "***".to_owned(),
    }
}

/// `12 Harbor St, Apt 3` → `12 H***`: the house number stays, the street is hidden.
pub(crate) fn mask_street(street: &str) -> String {
    let mut words = street.split_whitespace();
    let Some(first) = words.next() else {
        return String::new();
    };
    if first.chars().all(|c| c.is_ascii_digit()) {
        match words.next().and_then(|w| w.chars().next()) {
            Some(c) => format!("{first} {c}***"),
            None => first.to_owned(),
        }
    } else {
        mask_name(first)
    }
}

/// `11201` → `112**`.
pub(crate) fn mask_postal_code(code: &str) -> String {
    code.chars()
        .enumerate()
        .map(|(i, c)| {
            if i < 3 || !c.is_alphanumeric() {
                c
            } else {
                '*'
            }
        })
        .collect()
}

/// First letter upper case.
pub(crate) fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// The two texts of a memory: the line kept on the profile (`Natural fabrics`) and the note shown
/// right after the agent remembers it (`Remembered: you prefer natural fabrics`).
pub(crate) fn memory_texts(key: &str, value: &str) -> (String, String) {
    let value = value.trim();
    let yes = matches!(value.to_ascii_lowercase().as_str(), "yes" | "true");
    match key {
        "size" => (
            format!("Size {value}"),
            format!("Remembered: you wear size {value}"),
        ),
        "fabric_preference" => (capitalize(value), format!("Remembered: you prefer {value}")),
        "compares_specs" if yes => (
            "Compares specs".to_owned(),
            "Remembered: you like to compare specs".to_owned(),
        ),
        "travel_priority" if !value.to_ascii_lowercase().contains("travel") => {
            let verb = if value.contains(" and ") || value.ends_with('s') {
                "matter"
            } else {
                "matters"
            };
            (
                format!("{} {verb} when traveling", capitalize(value)),
                format!("Remembered: {value} {verb} to you when traveling"),
            )
        }
        _ => (capitalize(value), format!("Remembered: {value}")),
    }
}

/// Why a value must not be remembered as sensitive data (payment or health), if it is.
/// The agent is told not to send such values; this is the server's backstop.
pub(crate) fn sensitive_reason(value: &str) -> Option<&'static str> {
    let digits: String = value.chars().filter(|c| !matches!(c, ' ' | '-')).collect();
    let mut run = 0;
    for c in digits.chars() {
        run = if c.is_ascii_digit() { run + 1 } else { 0 };
        if run >= 12 {
            return Some("It looks like a card or account number.");
        }
    }
    let lower = value.to_ascii_lowercase();
    const PAYMENT: &[&str] = &[
        "card number",
        "cvv",
        "cvc",
        "expiry date",
        "password",
        "bank account",
        "routing number",
        "social security",
    ];
    const HEALTH: &[&str] = &[
        "diagnos",
        "medical",
        "medication",
        "prescription",
        "pregnan",
        "disease",
        "disorder",
        "allerg",
        "therapy",
    ];
    if PAYMENT.iter().any(|w| lower.contains(w)) {
        return Some("It is payment or login data.");
    }
    if HEALTH.iter().any(|w| lower.contains(w)) {
        return Some("It is health information.");
    }
    None
}

/// Whether a value is about someone other than the shopper (a gift recipient). Those facts are
/// used in the conversation only and never stored.
pub(crate) fn about_someone_else(value: &str) -> bool {
    let lower = format!(" {} ", value.to_ascii_lowercase());
    const OTHERS: &[&str] = &[
        "dad",
        "father",
        "mom",
        "mother",
        "mum",
        "wife",
        "husband",
        "son",
        "daughter",
        "friend",
        "partner",
        "brother",
        "sister",
        "grandma",
        "grandpa",
        "boyfriend",
        "girlfriend",
        "boss",
        "colleague",
        "uncle",
        "aunt",
        "nephew",
        "niece",
        "cousin",
    ];
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .collect();
    let names_someone = words.iter().any(|w| {
        OTHERS
            .iter()
            .any(|o| *w == *o || w.strip_suffix("'s") == Some(o) || *w == format!("{o}s"))
    });
    let third_person = words
        .iter()
        .any(|w| matches!(*w, "he" | "she" | "his" | "him" | "her" | "hers"));
    names_someone || third_person || lower.contains(" gift for ")
}

/// Whether a postal code has the shape the country uses. Countries we do not know only need
/// letters, digits, spaces or dashes.
pub(crate) fn postal_code_ok(country: &str, code: &str) -> bool {
    let code = code.trim();
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    match country {
        "US" => match code.split_once('-') {
            Some((zip, plus)) => digits(zip, 5) && digits(plus, 4),
            None => digits(code, 5),
        },
        "KR" | "DE" | "FR" | "ES" | "IT" => digits(code, 5),
        "JP" => code
            .split_once('-')
            .is_some_and(|(a, b)| digits(a, 3) && digits(b, 4)),
        "CA" => {
            let c: String = code.chars().filter(|c| *c != ' ').collect();
            c.len() == 6
                && c.chars().enumerate().all(|(i, ch)| {
                    if i % 2 == 0 {
                        ch.is_ascii_uppercase()
                    } else {
                        ch.is_ascii_digit()
                    }
                })
        }
        _ => {
            !code.is_empty()
                && code.len() <= 12
                && code
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-')
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_names_emails_and_streets() {
        assert_eq!(mask_name("Alex Kim"), "A*** K***");
        assert_eq!(mask_email("alex.kim@example.com"), "a***@example.com");
        assert_eq!(mask_street("12 Harbor St, Apt 3"), "12 H***");
        assert_eq!(mask_postal_code("11201"), "112**");
    }

    #[test]
    fn writes_memory_lines_the_shopper_sees() {
        assert_eq!(
            memory_texts("travel_priority", "weight and packed size"),
            (
                "Weight and packed size matter when traveling".to_owned(),
                "Remembered: weight and packed size matter to you when traveling".to_owned()
            )
        );
        assert_eq!(memory_texts("size", "M").0, "Size M");
        assert_eq!(
            memory_texts("fabric_preference", "natural fabrics").0,
            "Natural fabrics"
        );
        assert_eq!(memory_texts("compares_specs", "yes").0, "Compares specs");
        assert_eq!(
            memory_texts("travel_priority", "travels light").0,
            "Travels light"
        );
        assert_eq!(
            memory_texts("fit_note", "you layer hoodies under coats").1,
            "Remembered: you layer hoodies under coats"
        );
        assert_eq!(
            memory_texts("layout_preference", "photos over details").0,
            "Photos over details"
        );
    }

    #[test]
    fn refuses_payment_health_and_other_peoples_data() {
        assert!(sensitive_reason("card 4111 1111 1111 1111").is_some());
        assert!(sensitive_reason("allergic to wool").is_some());
        assert!(sensitive_reason("weight and packed size").is_none());
        assert!(sensitive_reason("size 32").is_none());
        assert!(about_someone_else("my dad wears L"));
        assert!(about_someone_else("he hikes a lot"));
        assert!(about_someone_else("gift for my father"));
        assert!(!about_someone_else("you layer hoodies under coats"));
        assert!(!about_someone_else("natural fabrics"));
        assert!(!about_someone_else("photos over details"));
    }

    #[test]
    fn checks_postal_codes_by_country() {
        assert!(postal_code_ok("US", "11201"));
        assert!(postal_code_ok("US", "11201-1234"));
        assert!(!postal_code_ok("US", "1120"));
        assert!(!postal_code_ok("US", "ABCDE"));
        assert!(postal_code_ok("JP", "100-0001"));
        assert!(postal_code_ok("CA", "K1A 0B1"));
        assert!(postal_code_ok("GB", "SW1A 1AA"));
        assert!(!postal_code_ok("GB", ""));
    }
}
