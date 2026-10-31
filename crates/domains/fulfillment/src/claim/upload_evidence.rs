//! `upload_evidence`: attach photos or documents to a claim (damage, delivery, defect,
//! receipt). Evidence is only ever added (`evidence.append_only`). A claim waiting for photos
//! goes back to `requested`. File storage is outside this system: each file is recorded by its
//! upload id, name, type and size; no bytes are stored here.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{claim_not_found, visible_claim};
use crate::errors::{
    CLAIM_NOT_FOUND, EVIDENCE_LIMIT_EXCEEDED, NOT_REVOCABLE, UNSUPPORTED_MEDIA_TYPE,
};
use crate::repo::{self, ClaimEvent, ClaimRow};
use crate::{broken, rules};

/// What file storage is in this system: not ours. Said in every evidence record.
const STORAGE_NOTE: &str = "File storage is outside this system: the file is recorded by its upload id, name, type and size; no bytes are kept here.";

/// What a piece of evidence shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceType {
    /// Damage to the item or its packaging.
    DamagePhoto,
    /// Where or how the parcel was left.
    DeliveryPhoto,
    /// A fault in the product itself.
    ProductDefect,
    /// A receipt or invoice (payment details are masked).
    Receipt,
    /// Anything else.
    Other,
}

impl EvidenceType {
    fn as_str(self) -> &'static str {
        match self {
            Self::DamagePhoto => "damage_photo",
            Self::DeliveryPhoto => "delivery_photo",
            Self::ProductDefect => "product_defect",
            Self::Receipt => "receipt",
            Self::Other => "other",
        }
    }
}

/// One uploaded file (schema.org `MediaObject`).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceFileInput {
    /// The upload's id, from the chat attachment or `POST /api/uploads`.
    #[schemars(regex(pattern = r"^upl_[a-z0-9_]+$"), example = &"upl_8c1f")]
    pub upload_id: String,
    /// The file name as uploaded.
    #[schemars(length(min = 1, max = 200), example = &"parka-tear.jpg")]
    pub name: String,
    /// The media type (schema.org `encodingFormat`), for example image/jpeg.
    #[schemars(regex(pattern = r"^[a-z]+/[a-z0-9.+-]+$"), example = &"image/jpeg")]
    pub encoding_format: String,
    /// The size in bytes.
    #[schemars(range(min = 1), example = 482113)]
    pub content_size_bytes: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UploadEvidenceInput {
    /// The claim the evidence is for.
    #[schemars(regex(pattern = r"^clm_[a-z0-9_]+$"), example = &"clm_20366")]
    pub claim_id: String,
    /// The uploaded files.
    #[schemars(length(min = 1, max = 20), example = json!([{"upload_id": "upl_8c1f", "name": "parka-tear.jpg", "encoding_format": "image/jpeg", "content_size_bytes": 482113}]))]
    pub files: Vec<EvidenceFileInput>,
    /// What the files show: damage_photo, delivery_photo, product_defect, receipt or other.
    #[schemars(example = &"damage_photo")]
    pub evidence_type: EvidenceType,
    /// A note about the files.
    #[schemars(length(max = 500), example = &"Both parkas have a tear at the left sleeve.")]
    pub note: Option<String>,
}

/// The evidence to add, fixed at preview time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidencePlan {
    pub claim_id: String,
    pub from_status: String,
    pub to_status: String,
    /// `[{id, evidence_type, content_url, upload_id, name, encoding_format,
    /// content_size_bytes, description}]`, added as written.
    pub entries: Vec<Value>,
}

pub struct UploadEvidence;

impl Operation for UploadEvidence {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.upload_evidence",
        tool: "upload_evidence",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/fulfillment/claims/{claim_id}/evidence",
        summary: "Attach uploaded photos or documents (damage, delivery, defect, receipt) to a claim; a claim waiting for photos goes back to requested. Evidence is only added, never removed.",
        consumes: &[Slot::one("claim_id")],
        produces: &[Slot::many("evidence_id")],
        checks: &[
            "evidence.media_types",
            "evidence.max_files_per_claim",
            "evidence.mask_payment_details",
        ],
        errors: &[
            CLAIM_NOT_FOUND,
            UNSUPPORTED_MEDIA_TYPE,
            EVIDENCE_LIMIT_EXCEEDED,
            ErrorCode::INVALID_STATE,
        ],
        audience: Audience::Both,
        action_type: ActionType::ADD,
        person_scope: "",
    };
    type Input = UploadEvidenceInput;
    type Output = Value;
}

/// `2 JPEG photos`, `1 PDF document`.
fn files_text(formats: &[String]) -> String {
    let mut groups: Vec<(String, usize)> = Vec::new();
    for f in formats {
        match groups.iter_mut().find(|(g, _)| g == f) {
            Some((_, n)) => *n += 1,
            None => groups.push((f.clone(), 1)),
        }
    }
    groups
        .iter()
        .map(|(f, n)| {
            let (kind, sub) = f.split_once('/').unwrap_or((f.as_str(), ""));
            let noun = if kind == "image" { "photo" } else { "document" };
            let plural = if *n == 1 { "" } else { "s" };
            format!("{n} {} {noun}{plural}", sub.to_uppercase())
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn has_entry(c: &ClaimRow, id: &str) -> bool {
    c.evidence.as_array().is_some_and(|l| {
        l.iter()
            .any(|e| e.get("id").and_then(Value::as_str) == Some(id))
    })
}

impl WriteOp<Cx> for UploadEvidence {
    type Plan = EvidencePlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UploadEvidenceInput,
    ) -> Result<Planned<EvidencePlan>, ToolError> {
        let rules = rules(&cx.policy)?;
        let allowed = rules.list("evidence.media_types");
        let formats: Vec<String> = input
            .files
            .iter()
            .map(|f| f.encoding_format.to_ascii_lowercase())
            .collect();
        if let Some(bad) = formats.iter().find(|f| !allowed.contains(f)) {
            return Err(ToolError::new(
                UNSUPPORTED_MEDIA_TYPE,
                format!(
                    "{bad} files cannot be attached; use {}.",
                    allowed.join(", ")
                ),
            )
            .detail(
                json!({"field": "files", "reason": "enum", "allowed": allowed,
                           "rule": "evidence.media_types"}),
            )
            .next("upload_evidence"));
        }
        let max = rules
            .lookup("evidence.max_files_per_claim")
            .and_then(Value::as_i64)
            .ok_or_else(|| {
                broken("The store's policy has no usable `evidence.max_files_per_claim`.")
            })?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let c = visible_claim(&mut tx, &input.claim_id, "search_claims").await?;
        tx.commit().await.map_err(ToolError::from)?;
        if !c.is_open() {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "Claim {} is {}; evidence can no longer be added.",
                    c.id,
                    c.claim_status.replace('_', " ")
                ),
            )
            .detail(json!({"entity": "claim", "from": c.claim_status, "allowed_to": []}))
            .next("get_claim"));
        }
        let existing: Vec<Value> = c.evidence.as_array().cloned().unwrap_or_default();
        let known = |upload: &str| {
            existing
                .iter()
                .any(|e| e.get("upload_id").and_then(Value::as_str) == Some(upload))
        };
        let mut changes = Vec::new();
        let mut entries = Vec::new();
        let mut kept_formats = Vec::new();
        let note = input
            .note
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty());
        for f in &input.files {
            let dup = known(&f.upload_id)
                || entries
                    .iter()
                    .any(|e: &Value| e["upload_id"] == json!(f.upload_id));
            if dup {
                changes.push(Change {
                    op: ChangeOp::Exclude,
                    entity: "evidence".into(),
                    id: String::new(),
                    from: Value::Null,
                    to: json!({"upload_id": f.upload_id, "name": f.name}),
                    note: Some("Already attached to this claim".into()),
                });
                continue;
            }
            kept_formats.push(f.encoding_format.to_ascii_lowercase());
            entries.push(json!({
                "id": format!("evd_{}", acom_core::id::new_suffix()),
                "evidence_type": input.evidence_type.as_str(),
                "content_url": null,
                "upload_id": f.upload_id,
                "name": f.name.trim(),
                "encoding_format": f.encoding_format.to_ascii_lowercase(),
                "content_size_bytes": f.content_size_bytes,
                "description": note,
            }));
        }
        if entries.is_empty() {
            return Err(
                ToolError::invalid_input("These files are already attached to the claim.")
                    .detail(json!({"field": "files", "reason": "format"})),
            );
        }
        let total = existing.len() as i64 + entries.len() as i64;
        if total > max {
            return Err(ToolError::new(
                EVIDENCE_LIMIT_EXCEEDED,
                format!(
                    "A claim holds at most {max} files; this one has {} already.",
                    existing.len()
                ),
            )
            .detail(json!({"limit": max, "actual": total,
                           "policy_key": "evidence.max_files_per_claim"})));
        }
        let to_status = if c.claim_status == "photos_requested" {
            "requested"
        } else {
            c.claim_status.as_str()
        };
        changes.insert(
            0,
            Change {
                op: ChangeOp::Create,
                entity: "evidence".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"evidence_type": input.evidence_type.as_str(), "files": entries.len()}),
                note: Some(STORAGE_NOTE.into()),
            },
        );
        if to_status != c.claim_status {
            changes.insert(
                1,
                Change {
                    op: ChangeOp::Update,
                    entity: "claim".into(),
                    id: c.id.clone(),
                    from: json!(c.claim_status),
                    to: json!(to_status),
                    note: None,
                },
            );
        }
        let mut checks = vec![
            Check {
                rule: "evidence.media_types".into(),
                passed: true,
                detail: Some(files_text(&kept_formats)),
                fix_tool: None,
            },
            Check {
                rule: "evidence.max_files_per_claim".into(),
                passed: true,
                detail: Some(format!("{total} of {max} files on this claim")),
                fix_tool: None,
            },
        ];
        if input.evidence_type == EvidenceType::Receipt
            && rules
                .lookup("evidence.mask_payment_details")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        {
            checks.push(Check {
                rule: "evidence.mask_payment_details".into(),
                passed: true,
                detail: Some(
                    "Payment details on a receipt are masked where the file is kept".into(),
                ),
                fix_tool: None,
            });
        }
        let approval = cx.policy.approval_for(Self::META.tool, &json!({}));
        Ok(Planned {
            plan: EvidencePlan {
                claim_id: c.id.clone(),
                from_status: c.claim_status.clone(),
                to_status: to_status.to_owned(),
                entries,
            },
            changes,
            checks,
            basis: vec![Basis {
                entity: "fulfillment.claim".into(),
                id: c.id.clone(),
                version: i64::from(c.version),
            }],
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: approval.reason.unwrap_or_else(|| "Adds evidence".into()),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &EvidencePlan) -> Result<Committed, ToolError> {
        let ids: Vec<Value> = plan.entries.iter().map(|e| e["id"].clone()).collect();
        let first = ids
            .first()
            .and_then(Value::as_str)
            .ok_or_else(|| broken("upload_evidence plan has no files"))?
            .to_owned();
        let done = Committed {
            id: first.clone(),
            produced: Map::from_iter([
                ("evidence_id".to_owned(), json!(ids)),
                ("claim_id".to_owned(), json!(plan.claim_id)),
            ]),
        };
        let c = repo::claim_for_update(tx, &plan.claim_id)
            .await?
            .ok_or_else(|| claim_not_found(&plan.claim_id, "search_claims"))?;
        if has_entry(&c, &first) {
            return Ok(done);
        }
        if !c.is_open() {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!("Claim {} is {}.", c.id, c.claim_status.replace('_', " ")),
            )
            .next("get_claim"));
        }
        let agent = tx.principal.subject.clone();
        let at = tx.time.format(tx.clock.now());
        let entries: Vec<Value> = plan
            .entries
            .iter()
            .map(|e| {
                let mut e = e.clone();
                e["added_by"] = json!(agent);
                e["added_at"] = json!(at);
                e
            })
            .collect();
        repo::append_evidence(tx, &c.id, &Value::Array(entries)).await?;
        let moved = plan.from_status != plan.to_status
            && repo::move_claim(tx, &c.id, &plan.from_status, &plan.to_status).await?;
        repo::append_event(
            tx,
            &c.id,
            &ClaimEvent {
                action_type: "AddAction",
                action_status: "completed",
                instrument: Self::META.tool,
                from_state: moved.then_some(plan.from_status.as_str()),
                to_state: moved.then_some(plan.to_status.as_str()),
                result: Some(json!({"evidence_ids": ids, "storage": STORAGE_NOTE})),
            },
        )
        .await?;
        Ok(done)
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &EvidencePlan,
        done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let c = repo::claim(&mut tx, &plan.claim_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match c {
            Some(c) if has_entry(&c, &done.id) => Vec::new(),
            Some(c) => vec![format!("claim {} has no evidence {}", c.id, done.id)],
            None => vec![format!("claim {} is missing", plan.claim_id)],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        _: &mut TxCx,
        plan: &EvidencePlan,
        _: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            NOT_REVOCABLE,
            format!(
                "Evidence on {} is never removed (evidence.append_only); a person can mark it void.",
                plan.claim_id
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{input, offline};

    #[tokio::test]
    async fn refuses_files_the_policy_does_not_take() {
        let e = UploadEvidence
            .preview(
                &offline("cus_riley"),
                input(json!({"claim_id": "clm_20366", "evidence_type": "damage_photo",
                             "files": [{"upload_id": "upl_1", "name": "tear.gif",
                                        "encoding_format": "image/gif", "content_size_bytes": 900}]})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "unsupported_media_type");
        assert_eq!(e.next_tool.as_deref(), Some("upload_evidence"));
    }

    #[test]
    fn says_what_the_files_are() {
        assert_eq!(
            files_text(&["image/jpeg".into(), "image/jpeg".into()]),
            "2 JPEG photos"
        );
        assert_eq!(
            files_text(&["image/png".into(), "application/pdf".into()]),
            "1 PNG photo, 1 PDF document"
        );
    }
}
