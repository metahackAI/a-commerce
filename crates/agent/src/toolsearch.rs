//! Tool search (docs/backend/agent.md §3): the model never sees the whole manifest.
//!
//! 1. Scope: the agent's `tool_scope` ∩ the caller's manifest (already filtered by scopes).
//! 2. Score: BM25 over each tool's name, summary, and field names and descriptions, plus a
//!    share for the agent's home domains.
//! 3. Anchors: the best hit, then the hits that best answer the words still open, so that a
//!    request asking for two things gets a start on both.
//! 4. Graph: the anchors pull in, one hop away, the tools that produce what they consume (the
//!    steps before) and that consume what they produce (the steps after). They take turns.
//! 5. Cut to `max_candidates`, filling up to `min_candidates` with the next best hits.
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

/// How well a read of the same records must fit the words, as a share of an anchor's score, to
/// come with it.
const SIBLING_SHARE: f64 = 0.6;

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

/// BM25 scores of each of `query`'s words against each of `docs` (k1 = 1.2, b = 0.75): one row
/// per document, one column per word.
fn bm25(query: &[String], docs: &[Vec<String>]) -> Vec<Vec<f64>> {
    let n = docs.len() as f64;
    let avg = docs.iter().map(Vec::len).sum::<usize>() as f64 / n.max(1.0);
    let mut df: BTreeMap<&str, f64> = BTreeMap::new();
    for d in docs {
        let unique: BTreeSet<&str> = d.iter().map(String::as_str).collect();
        for w in unique {
            *df.entry(w).or_default() += 1.0;
        }
    }
    docs.iter()
        .map(|d| {
            let len = d.len() as f64;
            query
                .iter()
                .map(|q| {
                    let tf = d.iter().filter(|w| *w == q).count() as f64;
                    if tf == 0.0 {
                        return 0.0;
                    }
                    let df = df.get(q.as_str()).copied().unwrap_or(0.0);
                    let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
                    idf * tf * 2.2 / (tf + 1.2 * (0.25 + 0.75 * len / avg.max(1.0)))
                })
                .collect()
        })
        .collect()
}

/// A tool scored against the request: `by_word` holds its share of `score` per query word.
struct Hit<'a> {
    tool: &'a ToolSpec,
    score: f64,
    by_word: Vec<f64>,
}

/// The hits the graph grows from: the best one, then each time the one that best answers what
/// the others left open. A request can ask for several things ("a sale on outerwear, and put it
/// on the main page"), and the strongest hits may all answer the same one: coverage-based
/// diversification, as in xQuAD. A tool covers a word in proportion to its score on that word
/// against the best tool's; pinned tools are candidates anyway, so their share is covered too.
fn anchors<'h, 'a>(hits: &'h [Hit<'a>], pinned: &[String], count: usize) -> Vec<&'h Hit<'a>> {
    let mut rest: Vec<&Hit> = hits
        .iter()
        .filter(|h| h.score > 0.0 && !pinned.contains(&h.tool.tool))
        .collect();
    if rest.is_empty() {
        return Vec::new();
    }
    let words = rest[0].by_word.len();
    let best: Vec<f64> = (0..words)
        .map(|w| hits.iter().map(|h| h.by_word[w]).fold(0.0, f64::max))
        .collect();
    let mut open = vec![1.0; words];
    let cover = |h: &Hit, open: &mut [f64]| {
        for w in 0..words {
            if best[w] > 0.0 {
                open[w] *= 1.0 - h.by_word[w] / best[w];
            }
        }
    };
    for h in hits.iter().filter(|h| pinned.contains(&h.tool.tool)) {
        cover(h, &mut open);
    }
    let first = rest.remove(0);
    cover(first, &mut open);
    let mut picked = vec![first];
    while picked.len() < count {
        let gain = |h: &Hit| -> f64 { (0..words).map(|w| open[w] * h.by_word[w]).sum() };
        // `rest` is best first, so the first of equals wins and the choice stays deterministic.
        let Some(next) = rest
            .iter()
            .copied()
            .reduce(|a, b| if gain(b) > gain(a) { b } else { a })
            .filter(|h| gain(h) > 0.0)
        else {
            break;
        };
        cover(next, &mut open);
        rest.retain(|h| h.tool.tool != next.tool.tool);
        picked.push(next);
    }
    picked
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
    let mut words = expanded(query, cfg);
    words.sort();
    words.dedup();
    let mut hits: Vec<Hit> = scoped
        .iter()
        .zip(bm25(&words, &docs))
        .map(|(t, by_word)| {
            let boost = if profile.is_home(t.domain()) {
                1.0 + cfg.home_domain_boost
            } else {
                1.0
            };
            let by_word: Vec<f64> = by_word.into_iter().map(|s| s * boost).collect();
            Hit {
                tool: t,
                score: by_word.iter().sum(),
                by_word,
            }
        })
        .collect();
    hits.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.tool.tool.cmp(&b.tool.tool))
    });

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
    let mut limit = cfg.max_candidates + out.len();

    // The anchors first, then their neighbours, then the remaining hits.
    let pinned_tools: Vec<String> = out.iter().map(|c| c.tool.clone()).collect();
    let mut picked = anchors(&hits, &pinned_tools, 3);
    for h in &picked {
        push(
            Candidate {
                tool: h.tool.tool.clone(),
                score: h.score,
                via: Via::Match,
            },
            &mut out,
        );
        // A tool's words are its records' words: "show my wishlist" matches adding to the
        // wishlist as well as listing it, "what changed" one log entry (which needs its id) as
        // well as the search over them. When the words fit both about as well, the best other
        // read of the same records comes with it, so asking about something can read it.
        let needs_more = h.tool.is_write() || h.tool.tool.starts_with("get_");
        if needs_more
            && let Some(read) = hits.iter().find(|r| {
                !r.tool.is_write()
                    && r.tool.tool != h.tool.tool
                    && r.score >= h.score * SIBLING_SHARE
                    && r.tool.domain() == h.tool.domain()
                    && r.tool.resource() == h.tool.resource()
            })
            && !out.iter().any(|c| c.tool == read.tool.tool)
        {
            // On top of the room the graph's next steps have.
            limit += 1;
            push(
                Candidate {
                    tool: read.tool.tool.clone(),
                    score: read.score,
                    via: Via::Match,
                },
                &mut out,
            );
        }
    }
    // A plan is a chain of writes, so the graph follows the writes first: a write's neighbours
    // are the plan's next steps, a read's only what can be done with what it lists.
    picked.sort_by_key(|h| !h.tool.is_write());
    let score_of = |tool: &str| {
        hits.iter()
            .find(|h| h.tool.tool == tool)
            .map_or(0.0, |h| h.score)
    };
    let expanded: Vec<Vec<(&ToolSpec, Via)>> =
        picked.iter().map(|h| neighbours(h.tool, &scoped)).collect();
    // A tool next to several anchors joins them into one plan, so it comes first.
    let links = |tool: &str| {
        expanded
            .iter()
            .filter(|n| n.iter().any(|(t, _)| t.tool == tool))
            .count()
    };
    let mut queues: Vec<_> = expanded
        .iter()
        .map(|n| {
            let mut n = n.clone();
            n.sort_by(|(a, via_a), (b, via_b)| {
                let after = |via: &Via| matches!(via, Via::After { .. });
                after(via_a)
                    .cmp(&after(via_b))
                    .then_with(|| links(&b.tool).cmp(&links(&a.tool)))
                    .then_with(|| a.is_write().cmp(&b.is_write()).reverse())
                    .then_with(|| score_of(&b.tool).total_cmp(&score_of(&a.tool)))
                    .then_with(|| a.tool.cmp(&b.tool))
            });
            n.into_iter()
        })
        .collect();
    // The anchors take turns, so one with many neighbours does not use up the room. A step before
    // is only needed while nothing on the list produces its slot yet.
    let mut any = true;
    while any && out.len() < limit {
        any = false;
        for queue in &mut queues {
            if out.len() >= limit {
                break;
            }
            let next = queue.find(|(n, via)| {
                let listed = |tool: &str| out.iter().any(|c| c.tool == tool);
                let supplied = |slot: &str| {
                    scoped
                        .iter()
                        .any(|t| listed(&t.tool) && t.produces.iter().any(|p| p.name == slot))
                };
                !listed(&n.tool) && !matches!(via, Via::Before { slot, .. } if supplied(slot))
            });
            if let Some((neighbour, via)) = next {
                any = true;
                push(
                    Candidate {
                        tool: neighbour.tool.clone(),
                        score: score_of(&neighbour.tool),
                        via,
                    },
                    &mut out,
                );
            }
        }
    }
    for h in hits.iter().filter(|h| h.score > 0.0) {
        if out.len() >= limit {
            break;
        }
        push(
            Candidate {
                tool: h.tool.tool.clone(),
                score: h.score,
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
        .filter(|h| h.score == 0.0 && !h.tool.is_write())
        .map(|h| h.tool)
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
/// what it produces.
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
        let consumers = scoped
            .iter()
            .filter(|o| o.tool != t.tool && o.consumes.iter().any(|c| c.name == slot.name));
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
    use serde_json::json;

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

    /// A manifest where three tools answer "the main page" far better than any other tool
    /// answers anything, and the first has more neighbours than the search has room for.
    fn two_things_manifest() -> Vec<ToolSpec> {
        let read = |name: &str, summary: &str, consumes: Value, produces: Value| {
            fixtures_tool(name, "read", summary, consumes, produces)
        };
        let write = |name: &str, summary: &str, consumes: Value, produces: Value| {
            fixtures_tool(name, "write", summary, consumes, produces)
        };
        let slot = json!([{"name": "slot_id", "many": false}]);
        let promotion = json!([{"name": "promotion_id", "many": false}]);
        let mut tools = vec![
            read(
                "list_slots",
                "Slots of the main page.",
                json!([]),
                slot.clone(),
            ),
            read(
                "list_popups",
                "Popups of the main page.",
                json!([]),
                json!([]),
            ),
            read(
                "list_banners",
                "Banners of the main page.",
                json!([]),
                json!([]),
            ),
            write(
                "set_sale_prices",
                "Set sale prices as one promotion.",
                json!([{"name": "category_id", "many": false}]),
                promotion.clone(),
            ),
            read(
                "list_categories",
                "The category tree.",
                json!([]),
                json!([{"name": "category_id", "many": true}]),
            ),
            read(
                "list_wishlist",
                "Saved items.",
                json!([]),
                json!([{"name": "category_id", "many": true}]),
            ),
            write("create_coupon", "Create a coupon.", promotion, json!([])),
        ];
        for name in [
            "arrange_slot",
            "book_slot",
            "clear_slot",
            "describe_slot",
            "edit_slot",
            "fill_slot",
            "group_slot",
            "hide_slot",
        ] {
            tools.push(write(
                name,
                "Change a slot during a sale.",
                slot.clone(),
                json!([]),
            ));
        }
        tools
    }

    fn fixtures_tool(
        name: &str,
        kind: &str,
        summary: &str,
        consumes: Value,
        produces: Value,
    ) -> ToolSpec {
        serde_json::from_value(crate::tools::fixtures::tool(
            name,
            &format!("promotion.x.{name}"),
            kind,
            summary,
            consumes,
            produces,
            json!({}),
            json!([]),
        ))
        .unwrap()
    }

    #[test]
    fn a_request_for_two_things_gets_a_start_on_both() {
        let p = repo();
        let promotion = p.get("promotion").unwrap();
        // Without the related words: they would lift the sale over the main page here, and this
        // test is about the anchors, not about the words a request brings in.
        let cfg = ToolSearchConfig {
            related_words: BTreeMap::new(),
            ..p.runtime.tool_search.clone()
        };
        let found = search(
            &two_things_manifest(),
            promotion,
            "A sale, and put it on the main page.",
            &[],
            &cfg,
        );
        let n = names(&found);
        // The sale ranks below all three main-page tools, and is an anchor anyway.
        let score = |t: &str| found.iter().find(|c| c.tool == t).unwrap().score;
        for t in ["list_slots", "list_popups", "list_banners"] {
            assert!(score(t) > score("set_sale_prices"), "{found:#?}");
        }
        assert!(n[..3].contains(&"set_sale_prices"), "{found:#?}");
        // The slot tools do not use up the room: the sale's next step is there too.
        let coupon = found.iter().find(|c| c.tool == "create_coupon");
        assert!(coupon.is_some(), "{found:#?}");
        assert_eq!(
            coupon.unwrap().via,
            Via::After {
                of: "set_sale_prices".into(),
                slot: "promotion_id".into()
            }
        );
    }

    #[test]
    fn a_step_before_is_skipped_when_the_list_already_produces_its_slot() {
        let p = repo();
        let promotion = p.get("promotion").unwrap();
        let found = search(
            &two_things_manifest(),
            promotion,
            "A sale, and put it on the main page.",
            &["list_wishlist"],
            &p.runtime.tool_search,
        );
        let n = names(&found);
        assert!(n.contains(&"list_wishlist"), "{found:#?}");
        assert!(
            !n[..n.len().min(6)].contains(&"list_categories"),
            "{found:#?}"
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
    fn a_write_found_by_its_records_words_brings_the_read_of_them() {
        let tool = |name: &str, address: &str, kind: &str, summary: &str, consumes, produces| {
            serde_json::from_value::<ToolSpec>(crate::tools::fixtures::tool(
                name,
                address,
                kind,
                summary,
                consumes,
                produces,
                json!({}),
                json!([]),
            ))
            .unwrap()
        };
        let item = json!([{"name": "wishlist_item_id", "many": false}]);
        let mut m = vec![
            tool(
                "add_to_wishlist",
                "customer.wishlist.add",
                "write",
                "Show a product on the member's wishlist: the wishlist keeps it.",
                json!([]),
                item.clone(),
            ),
            tool(
                "list_wishlist",
                "customer.wishlist.list",
                "read",
                "The products a member keeps for later, newest first, with today's price.",
                json!([]),
                json!([]),
            ),
        ];
        // What can be done with a wishlist item would otherwise take all the room.
        for n in 0..10 {
            m.push(tool(
                &format!("change_item_{n}"),
                "customer.item.change",
                "write",
                "Change a kept item.",
                item.clone(),
                json!([]),
            ));
        }
        let p = repo();
        let found = search(
            &m,
            p.get("shopper").unwrap(),
            "Show my wishlist",
            &[],
            &p.runtime.tool_search,
        );
        let n = names(&found);
        assert_eq!(&n[..2], ["add_to_wishlist", "list_wishlist"], "{found:#?}");
        assert_eq!(
            n.len(),
            p.runtime.tool_search.max_candidates + 1,
            "on top of the room: {found:#?}"
        );
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
