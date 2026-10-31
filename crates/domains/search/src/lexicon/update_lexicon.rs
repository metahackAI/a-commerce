//! `update_lexicon` (docs/api/search.md#lexicon): replaces a language's whole dictionary with a
//! new version. It takes effect after `reindex`; the same dictionary again saves nothing.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Change, ChangeOp, Check, Committed, ErrorClass, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{
    BoostRule, CIRCULAR_SYNONYM, Lexicon, Neologism, REINDEX_PENDING, Synonym,
    UNSUPPORTED_LANGUAGE, current, in_use, insert, languages, remove, saved, version_number,
};

const NOT_REVOCABLE: ErrorCode = ErrorCode::new("not_revocable", ErrorClass::UndoWindowClosed);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateLexiconInput {
    /// The language, as a two- or three-letter code.
    #[schemars(regex(pattern = r"^[a-z]{2,3}$"), example = &"en")]
    pub in_language: String,
    /// The whole synonym set; entries left out are removed.
    #[schemars(length(max = 500), example = json!([{"terms": ["windbreaker", "wind jacket"], "bidirectional": true}]))]
    pub synonyms: Vec<Synonym>,
    /// New words and what they mean. Default none.
    #[schemars(length(max = 200), example = json!([{"surface": "packable", "normalize": "packs into pocket"}]))]
    pub neologisms: Option<Vec<Neologism>>,
    /// Words that raise products in the ranking or that find nothing. Default none.
    #[schemars(length(max = 200), example = json!([{"term": "merino", "effect": "boost"}]))]
    pub boost_rules: Option<Vec<BoostRule>>,
}

/// The version to save, fixed at preview time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LexiconPlan {
    /// `None` when the dictionary is the same as the newest version: nothing is saved.
    pub id: Option<String>,
    /// The newest version when previewed.
    pub from_version: Option<String>,
    pub lexicon: Lexicon,
}

pub struct UpdateLexicon;

impl Operation for UpdateLexicon {
    const META: OpMeta = OpMeta {
        address: "search.lexicon.update",
        tool: "update_lexicon",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Put,
        path: "/v1/search/lexicon",
        summary: "Replace a language's whole search dictionary (synonyms, new words, boost and exclude rules) with a new analyzer version. It takes effect after reindex.",
        consumes: &[],
        produces: &[Slot::one("analyzer_version")],
        checks: &[
            "lexicon.no_circular_synonym",
            "lexicon.no_self_reference",
            "index.lexicon_change_requires_reindex",
        ],
        errors: &[CIRCULAR_SYNONYM, UNSUPPORTED_LANGUAGE, REINDEX_PENDING],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = UpdateLexiconInput;
    type Output = Value;
}

fn rule_on(cx: &Cx, key: &str) -> bool {
    cx.policy
        .lookup(&format!("search.lexicon.{key}"))
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

fn sets(l: &Lexicon) -> Vec<String> {
    l.synonyms.iter().map(|s| s.terms.join(" = ")).collect()
}

impl WriteOp<Cx> for UpdateLexicon {
    type Plan = LexiconPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UpdateLexiconInput,
    ) -> Result<Planned<LexiconPlan>, ToolError> {
        let lang = input.in_language.clone();
        let supported = languages(&cx.policy);
        if !supported.contains(&lang) {
            return Err(ToolError::new(
                UNSUPPORTED_LANGUAGE,
                format!(
                    "The store's search analyzes {} only, not `{lang}`.",
                    supported.join(", ")
                ),
            )
            .detail(json!({"field": "in_language", "reason": "enum", "allowed": supported}))
            .next("get_lexicon"));
        }
        let wanted = Lexicon {
            in_language: lang.clone(),
            analyzer_version: String::new(),
            synonyms: input.synonyms,
            neologisms: input.neologisms.unwrap_or_default(),
            boost_rules: input.boost_rules.unwrap_or_default(),
            id: None,
            saved_at: None,
        }
        .normalized();
        let selfish = wanted.self_references();
        if rule_on(cx, "no_self_reference") && !selfish.is_empty() {
            return Err(ToolError::invalid_input(format!(
                "These entries point at themselves: {}.",
                selfish.join("; ")
            ))
            .detail(json!({"rule": "lexicon.no_self_reference", "items": selfish}))
            .next("update_lexicon"));
        }
        if rule_on(cx, "no_circular_synonym")
            && let Some(circle) = wanted.cycle()
        {
            return Err(ToolError::new(
                CIRCULAR_SYNONYM,
                format!(
                    "These entries lead back to themselves: {}.",
                    circle.join(" → ")
                ),
            )
            .detail(json!({"rule": "lexicon.no_circular_synonym", "items": circle}))
            .next("update_lexicon"));
        }

        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let newest = current(&mut tx, &cx.policy, &lang).await?;
        let used = in_use(&mut tx, &cx.policy, &lang).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let newest_version = newest.as_ref().map(|l| l.analyzer_version.clone());
        let used_version = used.map(|l| l.analyzer_version);
        let same = newest.as_ref().is_some_and(|n| n.same_words(&wanted));
        if !same
            && let (Some(n), Some(u)) = (&newest_version, &used_version)
            && n != u
        {
            return Err(ToolError::new(
                REINDEX_PENDING,
                format!(
                    "{n} is saved but the index still uses {u}; reindex before changing it again."
                ),
            )
            .detail(json!({"analyzer_version": n, "in_use_version": u}))
            .next("get_index_status"));
        }
        let next = newest_version
            .as_deref()
            .and_then(version_number)
            .map_or(1, |n| n + 1);
        let mut lexicon = wanted;
        let (id, change) = if same {
            let v = newest_version.clone().unwrap_or_default();
            lexicon.analyzer_version = v.clone();
            (
                None,
                Change {
                    op: ChangeOp::Update,
                    entity: "lexicon".into(),
                    id: lang.clone(),
                    from: json!({"analyzer_version": v}),
                    to: json!({"analyzer_version": v}),
                    note: Some(format!("Same dictionary as {v}; nothing is saved")),
                },
            )
        } else {
            lexicon.analyzer_version = format!("v{next}");
            let before = newest.as_ref().map(sets).unwrap_or_default();
            let after = sets(&lexicon);
            let added: Vec<&String> = after.iter().filter(|s| !before.contains(s)).collect();
            let removed: Vec<&String> = before.iter().filter(|s| !after.contains(s)).collect();
            (
                Some(format!("lex_{lang}_v{next}")),
                Change {
                    op: ChangeOp::Update,
                    entity: "lexicon".into(),
                    id: lang.clone(),
                    from: json!({"analyzer_version": newest_version,
                                 "synonyms": newest.as_ref().map_or(0, |l| l.synonyms.len()),
                                 "removed": removed}),
                    to: json!({"analyzer_version": lexicon.analyzer_version,
                               "synonyms": lexicon.synonyms.len(), "added": added,
                               "neologisms": lexicon.neologisms.len(),
                               "boost_rules": lexicon.boost_rules.len()}),
                    note: None,
                },
            )
        };
        let mut checks = vec![
            Check {
                rule: "lexicon.no_circular_synonym".into(),
                passed: true,
                detail: Some("No entry leads back to itself".into()),
                fix_tool: None,
            },
            Check {
                rule: "lexicon.no_self_reference".into(),
                passed: true,
                detail: Some("No entry points at itself".into()),
                fix_tool: None,
            },
        ];
        if !same
            && cx
                .policy
                .lookup("search.index.lexicon_change_requires_reindex")
                .and_then(Value::as_bool)
                .unwrap_or(true)
        {
            checks.push(Check {
                rule: "index.lexicon_change_requires_reindex".into(),
                passed: true,
                detail: Some("Takes effect after reindex".into()),
                fix_tool: Some("reindex".into()),
            });
        }
        let approval = cx.policy.approval_for(Self::META.tool, &json!({}));
        Ok(Planned {
            plan: LexiconPlan {
                id,
                from_version: newest_version,
                lexicon,
            },
            changes: vec![change],
            checks,
            basis: Vec::new(),
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: approval
                        .reason
                        .unwrap_or_else(|| "Changes the search dictionary".into()),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &LexiconPlan) -> Result<Committed, ToolError> {
        let l = &plan.lexicon;
        let done = Committed {
            id: l.analyzer_version.clone(),
            produced: Map::from_iter([("analyzer_version".to_owned(), json!(l.analyzer_version))]),
        };
        let Some(id) = &plan.id else {
            return Ok(done);
        };
        let policy = tx.policy.clone();
        let newest = current(tx, &policy, &l.in_language).await?;
        let newest_version = newest.as_ref().map(|n| n.analyzer_version.clone());
        if newest_version.as_deref() == Some(&l.analyzer_version)
            && newest.as_ref().and_then(|n| n.id.as_deref()) == Some(id.as_str())
        {
            // The same preview ran already.
            return Ok(done);
        }
        if newest_version != plan.from_version {
            return Err(ToolError::new(
                ErrorCode::ESTIMATE_CHANGED,
                "The dictionary changed since this preview. Preview the change again.",
            )
            .next("update_lexicon"));
        }
        insert(tx, l, id).await?;
        Ok(done)
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &LexiconPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let l = &plan.lexicon;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let stored = saved(&mut tx, &l.in_language, Some(&l.analyzer_version)).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match (&plan.id, stored) {
            (None, _) => Vec::new(),
            (Some(_), None) => vec![format!(
                "{} {} is missing",
                l.in_language, l.analyzer_version
            )],
            (Some(_), Some(s)) if !s.same_words(l) => {
                vec![format!("{} differs from the plan", l.analyzer_version)]
            }
            (Some(_), Some(_)) => Vec::new(),
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    /// Removes the saved version while no index uses it.
    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &LexiconPlan,
        _: &Committed,
    ) -> Result<(), ToolError> {
        let Some(id) = &plan.id else {
            return Ok(());
        };
        let l = &plan.lexicon;
        let policy = tx.policy.clone();
        if in_use(tx, &policy, &l.in_language)
            .await?
            .is_some_and(|u| u.analyzer_version == l.analyzer_version)
        {
            return Err(ToolError::new(
                NOT_REVOCABLE,
                format!(
                    "The index already uses {}; save the dictionary again to change it.",
                    l.analyzer_version
                ),
            )
            .detail(json!({"entity": "lexicon", "id": id})));
        }
        remove(tx, id).await
    }
}
