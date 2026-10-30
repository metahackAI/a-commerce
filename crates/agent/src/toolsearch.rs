//! Tool search (docs/backend/agent.md §3): the model never sees the whole manifest.
//!
//! 1. Scope: the agent's `tool_scope` ∩ the caller's manifest (already filtered by scopes).
//! 2. Score: BM25 over each tool's name, summary, and field names and descriptions, plus a
//!    share for the agent's home domains.
//! 3. Graph: the top hits pull in, one hop away, the tools that produce what they consume (the
//!    steps before) and that consume what they produce (the steps after).
//! 4. Cut to `max_candidates`, filling up to `min_candidates` with the next best hits.
//!
//! No model call, and deterministic: the same request and manifest give the same list.
//! Embedding similarity (pgvector) is the next layer; it is not built yet.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::Value;

use crate::profile::{AgentProfile, ToolSearchConfig};
use crate::tools::ToolSpec;

/// Why a tool is a candidate.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "via")]
pub enum Via {
    /// Matched the request's words.
    Match,
    /// Produces what a hit consumes.
    Before { of: String, slot: String },
    /// Consumes what a hit produces.
    After { of: String, slot: String },
    /// Already part of the open change set, so a correction can preview it again.
    Pinned,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Candidate {
    pub tool: String,
    pub score: f64,
    #[serde(flatten)]
    pub via: Via,
}

const STOP: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "can", "do", "for", "from", "i", "in", "is",
    "it", "its", "me", "my", "of", "on", "or", "our", "please", "put", "run", "so", "that", "the",
    "their", "them", "this", "to", "up", "we", "what", "when", "which", "with", "you", "your",
];

/// Lowercase words with a light stem (plural `s`), stop words removed.
pub fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .filter(|w| !STOP.contains(&w.as_str()))
        .map(|w| stem(&w))
        .collect()
}

fn stem(w: &str) -> String {
    if w.len() > 3 && w.ends_with("ies") {
        format!("{}y", &w[..w.len() - 3])
    } else if w.len() > 3 && w.ends_with('s') && !w.ends_with("ss") {
        w[..w.len() - 1].to_owned()
    } else {
        w.to_owned()
    }
}

/// The query's words with the related words the configuration names for them.
/// Related words chain ("sweater" → "fit" → "measurements"); each word is expanded once.
pub fn expanded(query: &str, cfg: &ToolSearchConfig) -> Vec<String> {
    let mut out = tokens(query);
    let mut expanded: BTreeSet<String> = BTreeSet::new();
    let mut i = 0;
    while i < out.len() {
        let w = out[i].clone();
        i += 1;
        if !expanded.insert(w.clone()) {
            continue;
        }
        for extra in cfg.related_words.get(&w).into_iter().flatten() {
            out.extend(tokens(extra));
        }
    }
    out
}

/// The text a tool is found by. The name counts three times: it is the strongest signal.
fn document(t: &ToolSpec) -> Vec<String> {
    let name = t.tool.replace('_', " ");
    let mut text = format!("{name} {name} {name} {} {}", t.summary, t.resource());
    if let Some(props) = t.properties() {
        for (field, schema) in props {
            text.push(' ');
            text.push_str(&field.replace('_', " "));
            if let Some(d) = schema.get("description").and_then(Value::as_str) {
                text.push(' ');
                text.push_str(d);
            }
        }
    }
    for s in t.consumes.iter().chain(&t.produces) {
        text.push(' ');
        text.push_str(&s.name.replace('_', " "));
    }
    tokens(&text)
}

/// BM25 scores of `query` against `docs` (k1 = 1.2, b = 0.75).
fn bm25(query: &[String], docs: &[Vec<String>]) -> Vec<f64> {
    let n = docs.len() as f64;
    let avg = docs.iter().map(Vec::len).sum::<usize>() as f64 / n.max(1.0);
    let mut df: BTreeMap<&str, f64> = BTreeMap::new();
    for d in docs {
        let unique: BTreeSet<&str> = d.iter().map(String::as_str).collect();
        for w in unique {
            *df.entry(w).or_default() += 1.0;
        }
    }
    let query: BTreeSet<&str> = query.iter().map(String::as_str).collect();
    docs.iter()
        .map(|d| {
            let len = d.len() as f64;
            query
                .iter()
                .map(|q| {
                    let tf = d.iter().filter(|w| w.as_str() == *q).count() as f64;
                    if tf == 0.0 {
                        return 0.0;
                    }
                    let df = df.get(q).copied().unwrap_or(0.0);
                    let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
                    idf * tf * 2.2 / (tf + 1.2 * (0.25 + 0.75 * len / avg.max(1.0)))
                })
                .sum()
        })
        .collect()
}

/// Candidates for one request, best first. `pinned` tools (the open change set's) always stay.
pub fn search(
    manifest: &[ToolSpec],
    profile: &AgentProfile,
    query: &str,
    pinned: &[&str],
    cfg: &ToolSearchConfig,
) -> Vec<Candidate> {
    let scoped: Vec<&ToolSpec> = manifest.iter().filter(|t| profile.allows(t)).collect();
    if scoped.is_empty() {
        return Vec::new();
    }
    let docs: Vec<Vec<String>> = scoped.iter().map(|t| document(t)).collect();
    let raw = bm25(&expanded(query, cfg), &docs);
    let mut hits: Vec<(f64, &ToolSpec)> = scoped
        .iter()
        .zip(raw)
        .map(|(t, s)| {
            let boost = if s > 0.0 && profile.is_home(t.domain()) {
                1.0 + cfg.home_domain_boost
            } else {
                1.0
            };
            (s * boost, *t)
        })
        .collect();
    hits.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.tool.cmp(&b.1.tool)));

    let mut out: Vec<Candidate> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut push = |c: Candidate, out: &mut Vec<Candidate>| {
        if seen.insert(c.tool.clone()) {
            out.push(c);
        }
    };
    for p in pinned {
        if scoped.iter().any(|t| t.tool == *p) {
            push(
                Candidate {
                    tool: (*p).to_owned(),
                    score: 0.0,
                    via: Via::Pinned,
                },
                &mut out,
            );
        }
    }
    let limit = cfg.max_candidates + out.len();

    // The strongest hits first, then their neighbours, then the remaining hits.
    let matched: Vec<&(f64, &ToolSpec)> = hits.iter().filter(|(s, _)| *s > 0.0).collect();
    let anchors = matched.len().min(3);
    for (s, t) in matched.iter().take(anchors) {
        push(
            Candidate {
                tool: t.tool.clone(),
                score: *s,
                via: Via::Match,
            },
            &mut out,
        );
    }
    for (_, anchor) in matched.iter().take(anchors) {
        for (neighbour, via) in neighbours(anchor, &scoped) {
            if out.len() >= limit {
                break;
            }
            let score = hits
                .iter()
                .find(|(_, t)| t.tool == neighbour.tool)
                .map_or(0.0, |(s, _)| *s);
            push(
                Candidate {
                    tool: neighbour.tool.clone(),
                    score,
                    via,
                },
                &mut out,
            );
        }
    }
    for (s, t) in &matched {
        if out.len() >= limit {
            break;
        }
        push(
            Candidate {
                tool: t.tool.clone(),
                score: *s,
                via: Via::Match,
            },
            &mut out,
        );
    }
    // Too few matches: fill with the scope's reads, the agent's home domains first, so the
    // planner can still look things up ("Light jacket for spring trips" names no tool).
    let floor = cfg.min_candidates + out.iter().filter(|c| c.via == Via::Pinned).count();
    let mut unmatched: Vec<&ToolSpec> = hits
        .iter()
        .filter(|(s, t)| *s == 0.0 && !t.is_write())
        .map(|(_, t)| *t)
        .collect();
    unmatched.sort_by_key(|t| (!profile.is_home(t.domain()), t.tool.clone()));
    for t in unmatched {
        if out.len() >= floor {
            break;
        }
        push(
            Candidate {
                tool: t.tool.clone(),
                score: 0.0,
                via: Via::Match,
            },
            &mut out,
        );
    }
    out
}

/// One hop in the tool graph: producers of what `t` consumes (reads first), and consumers of
/// what it produces, each in name order.
fn neighbours<'a>(t: &ToolSpec, scoped: &[&'a ToolSpec]) -> Vec<(&'a ToolSpec, Via)> {
    let mut before: Vec<(&ToolSpec, Via)> = Vec::new();
    for slot in &t.consumes {
        let mut producers: Vec<&&ToolSpec> = scoped
            .iter()
            .filter(|o| o.tool != t.tool && o.produces.iter().any(|p| p.name == slot.name))
            .collect();
        producers.sort_by_key(|o| (o.is_write(), o.tool.clone()));
        if let Some(p) = producers.first() {
            before.push((
                p,
                Via::Before {
                    of: t.tool.clone(),
                    slot: slot.name.clone(),
                },
            ));
        }
    }
    let mut after: Vec<(&ToolSpec, Via)> = Vec::new();
    for slot in &t.produces {
        let mut consumers: Vec<&&ToolSpec> = scoped
            .iter()
            .filter(|o| o.tool != t.tool && o.consumes.iter().any(|c| c.name == slot.name))
            .collect();
        consumers.sort_by_key(|o| o.tool.clone());
        for c in consumers {
            after.push((
                c,
                Via::After {
                    of: t.tool.clone(),
                    slot: slot.name.clone(),
                },
            ));
        }
    }
    before.into_iter().chain(after).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::fixtures::repo;
    use crate::tools::fixtures::manifest;

    fn names(c: &[Candidate]) -> Vec<&str> {
        c.iter().map(|c| c.tool.as_str()).collect()
    }

    #[test]
    fn finds_the_vip_sale_tools_and_their_neighbours() {
        let p = repo();
        let promotion = p.get("promotion").unwrap();
        let found = search(
            &manifest(),
            promotion,
            "Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page.",
            &[],
            &p.runtime.tool_search,
        );
        let n = names(&found);
        assert!(n.len() <= p.runtime.tool_search.max_candidates, "{n:?}");
        assert!(n[..3].contains(&"set_sale_prices"), "{found:#?}");
        for t in ["search_products", "create_coupon", "schedule_banner"] {
            assert!(n.contains(&t), "{t} missing from {n:?}");
        }
        // Shopper tools are out of the promotion agent's scope.
        assert!(!n.contains(&"add_to_cart"));
        // No word of the request names a coupon: it comes in as the step after the sale.
        let coupon = found.iter().find(|c| c.tool == "create_coupon").unwrap();
        assert_eq!(
            coupon.via,
            Via::After {
                of: "set_sale_prices".into(),
                slot: "promotion_id".into()
            }
        );
    }

    #[test]
    fn is_deterministic_and_keeps_pinned_tools() {
        let p = repo();
        let promotion = p.get("promotion").unwrap();
        let q = "Drop the low-stock ones. Give the overshirt 15% instead.";
        let a = search(
            &manifest(),
            promotion,
            q,
            &["set_sale_prices"],
            &p.runtime.tool_search,
        );
        let b = search(
            &manifest(),
            promotion,
            q,
            &["set_sale_prices"],
            &p.runtime.tool_search,
        );
        assert_eq!(a, b);
        assert_eq!(a[0].tool, "set_sale_prices");
        assert_eq!(a[0].via, Via::Pinned);
        assert!(a.len() >= p.runtime.tool_search.min_candidates);
    }

    #[test]
    fn shopper_search_stays_in_scope() {
        let p = repo();
        let shopper = p.get("shopper").unwrap();
        let found = search(
            &manifest(),
            shopper,
            "Add the windbreaker in M to my cart",
            &[],
            &p.runtime.tool_search,
        );
        let n = names(&found);
        assert_eq!(n[0], "add_to_cart", "{found:#?}");
        assert!(n.contains(&"search_products"));
        assert!(!n.contains(&"set_sale_prices"));
    }

    #[test]
    fn with_no_match_the_home_domains_reads_come_first() {
        let p = repo();
        let shopper = p.get("shopper").unwrap();
        let found = search(
            &manifest(),
            shopper,
            "Light jacket for spring trips",
            &[],
            &p.runtime.tool_search,
        );
        assert!(names(&found).contains(&"search_products"), "{found:?}");
    }

    #[test]
    fn related_words_widen_the_query() {
        let p = repo();
        let q = expanded("Will M fit in the wool overcoat?", &p.runtime.tool_search);
        assert!(q.iter().any(|w| w == "measurement"), "{q:?}");
        assert!(q.iter().any(|w| w == "overcoat"));
        let q = expanded("What if I just wear a sweater?", &p.runtime.tool_search);
        assert!(q.iter().any(|w| w == "measurement"), "chains: {q:?}");
    }

    #[test]
    fn tokens_drop_stop_words_and_plurals() {
        assert_eq!(
            tokens("Put the VIP members' sale on summer outerwear"),
            ["vip", "member", "sale", "summer", "outerwear"]
        );
        assert_eq!(tokens("categories"), ["category"]);
    }
}
