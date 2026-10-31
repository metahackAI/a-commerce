//! The `lexicon` resource (docs/api/search.md#lexicon): `get_lexicon` and `update_lexicon`, one
//! file each, and the dictionary search widens queries with: synonyms, new words, and boost or
//! exclude rules, per language and versioned.
//!
//! `v1` is the store policy's default (`search.yml lexicon.synonyms`); every `update_lexicon`
//! stores the next version in `search.lexicon`. A version takes effect when a reindex uses it
//! (`index.lexicon_change_requires_reindex`): queries widen with the dictionary of the index in
//! use (`in_use`), so a saved change waits for `reindex`.

use std::collections::{BTreeMap, BTreeSet};

use acom_contract::{ErrorClass, ErrorCode, ToolError};
use acom_policy::PolicySet;
use acom_store::TxCx;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::{stem, tokens, tsquery};
use crate::repo::db;

mod get_lexicon;
mod update_lexicon;

pub use get_lexicon::*;
pub use update_lexicon::*;

pub const LEXICON_NOT_FOUND: ErrorCode = ErrorCode::new("lexicon_not_found", ErrorClass::NotFound);
pub const CIRCULAR_SYNONYM: ErrorCode = ErrorCode::new("circular_synonym", ErrorClass::Domain);
pub const UNSUPPORTED_LANGUAGE: ErrorCode =
    ErrorCode::new("unsupported_language", ErrorClass::InvalidInput);
pub const REINDEX_PENDING: ErrorCode = ErrorCode::new("reindex_pending", ErrorClass::Domain);

fn yes() -> bool {
    true
}

/// Words that mean the same thing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Synonym {
    /// The words or phrases, at least two.
    #[schemars(length(min = 2, max = 20), example = ["windbreaker", "wind jacket"])]
    pub terms: Vec<String>,
    /// true: each term finds the others. false: the first term also finds the rest, not the
    /// other way round. Default true.
    #[serde(default = "yes")]
    #[schemars(example = true)]
    pub bidirectional: bool,
}

/// A new word and what it means in common words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Neologism {
    /// The new word as shoppers type it.
    #[schemars(length(min = 1, max = 60), example = &"packable")]
    pub surface: String,
    /// What it means; a search for the word also finds these words.
    #[schemars(length(min = 1, max = 200), example = &"packs into pocket")]
    pub normalize: String,
}

/// What a boost rule does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BoostEffect {
    /// Products with the word rank higher.
    Boost,
    /// A search with the word finds nothing.
    Exclude,
}

/// A word that raises products in the ranking or that search does not answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoostRule {
    /// The word.
    #[schemars(length(min = 1, max = 60), example = &"merino")]
    pub term: String,
    /// boost: products with the word rank higher; exclude: a search with the word finds
    /// nothing.
    #[schemars(example = &"boost")]
    pub effect: BoostEffect,
}

/// One version of a language's dictionary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lexicon {
    pub in_language: String,
    pub analyzer_version: String,
    pub synonyms: Vec<Synonym>,
    pub neologisms: Vec<Neologism>,
    pub boost_rules: Vec<BoostRule>,
    /// `lex_` id of a stored version; `None` for the policy's default.
    pub id: Option<String>,
    pub saved_at: Option<DateTime<Utc>>,
}

/// The languages the store's search analyzes (`search.yml lexicon.languages`).
pub fn languages(policy: &PolicySet) -> Vec<String> {
    policy
        .lookup("search.lexicon.languages")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The language of the store's index: the first the policy lists.
pub fn index_language(policy: &PolicySet) -> String {
    languages(policy)
        .into_iter()
        .next()
        .unwrap_or_else(|| "en".to_owned())
}

/// `v12` → 12.
pub fn version_number(v: &str) -> Option<u32> {
    v.strip_prefix('v')?.parse().ok()
}

/// The policy's dictionary of a language as version `v1`, when it has one.
pub fn policy_default(policy: &PolicySet, language: &str) -> Option<Lexicon> {
    let groups = policy
        .lookup(&format!("search.lexicon.synonyms.{language}"))?
        .as_array()?
        .iter()
        .filter_map(Value::as_array)
        .map(|g| Synonym {
            terms: g
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            bidirectional: true,
        })
        .collect();
    Some(Lexicon {
        in_language: language.to_owned(),
        analyzer_version: "v1".to_owned(),
        synonyms: groups,
        neologisms: Vec::new(),
        boost_rules: Vec::new(),
        id: None,
        saved_at: None,
    })
}

fn normal(term: &str) -> String {
    term.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

impl Lexicon {
    /// Terms trimmed and lowercased, empty ones dropped.
    pub fn normalized(mut self) -> Self {
        for s in &mut self.synonyms {
            s.terms = s
                .terms
                .iter()
                .map(|t| normal(t))
                .filter(|t| !t.is_empty())
                .collect();
        }
        for n in &mut self.neologisms {
            n.surface = normal(&n.surface);
            n.normalize = normal(&n.normalize);
        }
        for b in &mut self.boost_rules {
            b.term = normal(&b.term);
        }
        self
    }

    /// Whether two versions hold the same dictionary.
    pub fn same_words(&self, other: &Lexicon) -> bool {
        self.synonyms == other.synonyms
            && self.neologisms == other.neologisms
            && self.boost_rules == other.boost_rules
    }

    /// Entries that point at themselves: a synonym entry with fewer than two different terms, or
    /// a new word that means itself (`lexicon.no_self_reference`).
    pub fn self_references(&self) -> Vec<String> {
        let mut out = Vec::new();
        for s in &self.synonyms {
            let distinct: BTreeSet<&String> = s.terms.iter().collect();
            if distinct.len() < 2 || distinct.len() < s.terms.len() {
                out.push(s.terms.join(", "));
            }
        }
        for n in &self.neologisms {
            if n.surface == n.normalize {
                out.push(n.surface.clone());
            }
        }
        out
    }

    /// A chain of one-way entries that leads back to where it started
    /// (`lexicon.no_circular_synonym`), as the words along it.
    pub fn cycle(&self) -> Option<Vec<String>> {
        let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for s in self.synonyms.iter().filter(|s| !s.bidirectional) {
            if let Some((head, rest)) = s.terms.split_first() {
                for t in rest {
                    edges.entry(head.clone()).or_default().insert(t.clone());
                }
            }
        }
        for n in &self.neologisms {
            edges
                .entry(n.surface.clone())
                .or_default()
                .insert(n.normalize.clone());
        }
        fn walk(
            at: &str,
            edges: &BTreeMap<String, BTreeSet<String>>,
            path: &mut Vec<String>,
            done: &mut BTreeSet<String>,
        ) -> Option<Vec<String>> {
            if let Some(i) = path.iter().position(|p| p == at) {
                let mut c = path[i..].to_vec();
                c.push(at.to_owned());
                return Some(c);
            }
            if done.contains(at) {
                return None;
            }
            path.push(at.to_owned());
            for next in edges.get(at).into_iter().flatten() {
                if let Some(c) = walk(next, edges, path, done) {
                    return Some(c);
                }
            }
            path.pop();
            done.insert(at.to_owned());
            None
        }
        let mut done = BTreeSet::new();
        for start in edges.keys() {
            if let Some(c) = walk(start, &edges, &mut Vec::new(), &mut done) {
                return Some(c);
            }
        }
        None
    }

    /// The synonym groups that widen this query: every two-way entry, a one-way entry when the
    /// query has its first term, and a new word with its meaning when the query has the word.
    pub fn groups(&self, query: &str) -> Vec<Vec<String>> {
        let words: Vec<String> = tokens(query);
        let has = |term: &str| {
            let t = tokens(term);
            !t.is_empty() && t.iter().all(|w| words.iter().any(|q| stem(q) == stem(w)))
        };
        let flat =
            |terms: &[&str]| -> Vec<String> { terms.iter().flat_map(|t| tokens(t)).collect() };
        let mut out = Vec::new();
        for s in &self.synonyms {
            let terms: Vec<&str> = s.terms.iter().map(String::as_str).collect();
            if s.bidirectional || s.terms.first().is_some_and(|h| has(h)) {
                out.push(flat(&terms));
            }
        }
        for n in &self.neologisms {
            if has(&n.surface) {
                out.push(flat(&[&n.surface, &n.normalize]));
            }
        }
        out.retain(|g| !g.is_empty());
        out
    }

    /// The excluded word the query has, if any: such a search finds nothing.
    pub fn excluded_in(&self, query: &str) -> Option<String> {
        let words = tokens(query);
        self.boost_rules
            .iter()
            .filter(|b| b.effect == BoostEffect::Exclude)
            .find(|b| {
                let t = tokens(&b.term);
                !t.is_empty() && t.iter().all(|w| words.iter().any(|q| stem(q) == stem(w)))
            })
            .map(|b| b.term.clone())
    }

    /// One `to_tsquery` text per boosted word; a product matching one counts it as one more
    /// concept covered.
    pub fn boosts(&self) -> Vec<String> {
        self.boost_rules
            .iter()
            .filter(|b| b.effect == BoostEffect::Boost)
            .filter_map(|b| tsquery(&tokens(&b.term)).map(|q| q.replace(" | ", " & ")))
            .collect()
    }

    /// As `get_lexicon` shows it.
    pub fn json(&self) -> Value {
        json!({
            "in_language": self.in_language,
            "analyzer_version": self.analyzer_version,
            "synonyms": self.synonyms,
            "neologisms": self.neologisms,
            "boost_rules": self.boost_rules,
            "source": if self.id.is_some() { "saved" } else { "policy_default" },
        })
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct LexiconRow {
    id: String,
    in_language: String,
    analyzer_version: String,
    synonyms: Value,
    neologisms: Value,
    boost_rules: Value,
    created_at: DateTime<Utc>,
}

impl From<LexiconRow> for Lexicon {
    fn from(r: LexiconRow) -> Self {
        Self {
            in_language: r.in_language,
            analyzer_version: r.analyzer_version,
            synonyms: serde_json::from_value(r.synonyms).unwrap_or_default(),
            neologisms: serde_json::from_value(r.neologisms).unwrap_or_default(),
            boost_rules: serde_json::from_value(r.boost_rules).unwrap_or_default(),
            id: Some(r.id),
            saved_at: Some(r.created_at),
        }
    }
}

const LEXICON_SELECT: &str = "SELECT id, in_language, analyzer_version, synonyms, neologisms,
    boost_rules, created_at FROM search.lexicon WHERE tenant_id = $1 AND in_language = $2";

/// A saved version (`Some(v)`), or the newest saved one (`None`).
async fn saved(
    tx: &mut TxCx,
    language: &str,
    version: Option<&str>,
) -> Result<Option<Lexicon>, ToolError> {
    let row = match version {
        Some(v) => {
            sqlx::query_as::<_, LexiconRow>(sqlx::AssertSqlSafe(format!(
                "{LEXICON_SELECT} AND analyzer_version = $3"
            )))
            .bind(tx.tenant.as_str())
            .bind(language)
            .bind(v)
            .fetch_optional(&mut *tx.tx)
            .await
        }
        None => {
            sqlx::query_as::<_, LexiconRow>(sqlx::AssertSqlSafe(format!(
                "{LEXICON_SELECT} ORDER BY created_at DESC, id DESC LIMIT 1"
            )))
            .bind(tx.tenant.as_str())
            .bind(language)
            .fetch_optional(&mut *tx.tx)
            .await
        }
    }
    .map_err(db)?;
    Ok(row.map(Lexicon::from))
}

/// The newest version of a language: the last saved one, else the policy's default.
pub async fn current(
    tx: &mut TxCx,
    policy: &PolicySet,
    language: &str,
) -> Result<Option<Lexicon>, ToolError> {
    Ok(match saved(tx, language, None).await? {
        Some(l) => Some(l),
        None => policy_default(policy, language),
    })
}

/// One version of a language, saved or the policy's `v1`.
pub async fn version(
    tx: &mut TxCx,
    policy: &PolicySet,
    language: &str,
    v: &str,
) -> Result<Option<Lexicon>, ToolError> {
    if let Some(l) = saved(tx, language, Some(v)).await? {
        return Ok(Some(l));
    }
    Ok(policy_default(policy, language).filter(|d| d.analyzer_version == v))
}

/// The version the index in use was built with (the policy's `v1` before any reindex).
pub async fn in_use(
    tx: &mut TxCx,
    policy: &PolicySet,
    language: &str,
) -> Result<Option<Lexicon>, ToolError> {
    match crate::index::active(tx, language).await? {
        Some(job) => match version(tx, policy, language, &job.analyzer_version).await? {
            Some(l) => Ok(Some(l)),
            None => Ok(policy_default(policy, language)),
        },
        None => Ok(policy_default(policy, language)),
    }
}

/// Saves a version (saving the same id again changes nothing).
async fn insert(tx: &mut TxCx, l: &Lexicon, id: &str) -> Result<(), ToolError> {
    let agent = tx.principal.subject.clone();
    sqlx::query(
        "INSERT INTO search.lexicon (tenant_id, id, in_language, analyzer_version, synonyms,
           neologisms, boost_rules, agent_id, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(&l.in_language)
    .bind(&l.analyzer_version)
    .bind(json!(l.synonyms))
    .bind(json!(l.neologisms))
    .bind(json!(l.boost_rules))
    .bind(&agent)
    .bind(tx.clock.now())
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Removes a saved version no index has used (undo of `update_lexicon`).
async fn remove(tx: &mut TxCx, id: &str) -> Result<(), ToolError> {
    sqlx::query("DELETE FROM search.lexicon WHERE tenant_id = $1 AND id = $2")
        .bind(tx.tenant.as_str())
        .bind(id)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::policies;

    fn syn(terms: &[&str], bidirectional: bool) -> Synonym {
        Synonym {
            terms: terms.iter().map(|t| (*t).to_owned()).collect(),
            bidirectional,
        }
    }

    fn lexicon(synonyms: Vec<Synonym>) -> Lexicon {
        Lexicon {
            in_language: "en".into(),
            analyzer_version: "v2".into(),
            synonyms,
            neologisms: vec![Neologism {
                surface: "Packable ".into(),
                normalize: "packs into  pocket".into(),
            }],
            boost_rules: vec![
                BoostRule {
                    term: "merino".into(),
                    effect: BoostEffect::Boost,
                },
                BoostRule {
                    term: "fur".into(),
                    effect: BoostEffect::Exclude,
                },
            ],
            id: None,
            saved_at: None,
        }
        .normalized()
    }

    #[test]
    fn the_policy_dictionary_is_version_one() {
        let p = policies();
        let d = policy_default(&p, "en").unwrap();
        assert_eq!(d.analyzer_version, "v1");
        assert!(
            d.synonyms
                .iter()
                .any(|s| s.terms.contains(&"windbreaker".into()))
        );
        assert!(policy_default(&p, "ko").is_none());
        assert_eq!(languages(&p), ["en"]);
        assert_eq!(index_language(&p), "en");
        assert_eq!(version_number("v13"), Some(13));
        assert_eq!(version_number("13"), None);
    }

    #[test]
    fn one_way_entries_and_new_words_widen_only_from_their_word() {
        let l = lexicon(vec![
            syn(&["jacket", "coat"], true),
            syn(&["parka", "rain jacket"], false),
        ]);
        assert_eq!(l.neologisms[0].normalize, "packs into pocket");
        let g = l.groups("packable parkas");
        assert!(g.contains(&vec!["jacket".to_owned(), "coat".to_owned()]));
        assert!(g.contains(&vec![
            "parka".to_owned(),
            "rain".to_owned(),
            "jacket".to_owned()
        ]));
        assert!(g.iter().any(|g| g.contains(&"pocket".to_owned())));
        let g = l.groups("rain jacket");
        assert_eq!(g.len(), 1);
        assert_eq!(l.excluded_in("Fur hood"), Some("fur".into()));
        assert_eq!(l.excluded_in("further"), None);
        assert_eq!(l.boosts(), ["merino"]);
    }

    #[test]
    fn circles_and_self_references_are_found() {
        let ok = lexicon(vec![
            syn(&["a jacket", "coat"], false),
            syn(&["tee", "shirt"], true),
        ]);
        assert_eq!(ok.cycle(), None);
        assert!(ok.self_references().is_empty());
        let circle = lexicon(vec![
            syn(&["anorak", "parka"], false),
            syn(&["parka", "shell"], false),
            syn(&["shell", "anorak"], false),
        ]);
        assert_eq!(
            circle.cycle().unwrap(),
            ["anorak", "parka", "shell", "anorak"]
        );
        let selfish = lexicon(vec![syn(&["coat", "Coat"], true)]);
        assert_eq!(selfish.self_references(), ["coat, coat"]);
        assert!(selfish.same_words(&selfish.clone()));
    }
}
