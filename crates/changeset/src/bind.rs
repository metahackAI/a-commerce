//! `$item[n].slot` placeholders: an item that uses what an earlier item of the same change set
//! produces (a coupon for the promotion item 1 creates) carries the placeholder in its input and
//! plan. The runner replaces it with the real value once item `n` has run.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

/// `Some((position, slot))` when `s` is exactly `$item[n].slot`.
pub fn placeholder(s: &str) -> Option<(i32, &str)> {
    let rest = s.strip_prefix("$item[")?;
    let (n, slot) = rest.split_once("].")?;
    let n: i32 = n.parse().ok()?;
    let ok = n >= 1
        && !slot.is_empty()
        && slot
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    ok.then_some((n, slot))
}

/// Every item position a value refers to, in order, without repeats.
pub fn positions(v: &Value) -> Vec<i32> {
    fn walk(v: &Value, out: &mut Vec<i32>) {
        match v {
            Value::String(s) => {
                if let Some((n, _)) = placeholder(s)
                    && !out.contains(&n)
                {
                    out.push(n);
                }
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            Value::Object(o) => o.values().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(v, &mut out);
    out
}

/// Replaces every placeholder with what item `n` produced. Fails on a placeholder whose item
/// has not run or did not produce that slot.
pub fn bind(v: &Value, produced: &BTreeMap<i32, Map<String, Value>>) -> Result<Value, String> {
    Ok(match v {
        Value::String(s) => match placeholder(s) {
            Some((n, slot)) => produced
                .get(&n)
                .and_then(|m| m.get(slot))
                .cloned()
                .ok_or_else(|| format!("{s}: item {n} has not produced `{slot}`"))?,
            None => v.clone(),
        },
        Value::Array(a) => Value::Array(
            a.iter()
                .map(|x| bind(x, produced))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, x)| Ok((k.clone(), bind(x, produced)?)))
                .collect::<Result<_, String>>()?,
        ),
        other => other.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_placeholders_and_ignores_look_alikes() {
        assert_eq!(
            placeholder("$item[1].promotion_id"),
            Some((1, "promotion_id"))
        );
        assert_eq!(placeholder("$item[0].x"), None);
        assert_eq!(placeholder("$item[a].x"), None);
        assert_eq!(placeholder("promo_1"), None);
        assert_eq!(placeholder("$item[2].Bad"), None);
    }

    #[test]
    fn binds_nested_values_and_reports_missing_ones() {
        let plan = json!({"coupon": {"promotion_id": "$item[1].promotion_id"}, "ids": ["$item[1].promotion_id", "x"]});
        assert_eq!(positions(&plan), vec![1]);
        let mut produced = BTreeMap::new();
        produced.insert(
            1,
            Map::from_iter([("promotion_id".to_owned(), json!("promo_7"))]),
        );
        assert_eq!(
            bind(&plan, &produced).unwrap(),
            json!({"coupon": {"promotion_id": "promo_7"}, "ids": ["promo_7", "x"]})
        );
        assert!(bind(&json!("$item[2].banner_id"), &produced).is_err());
    }
}
