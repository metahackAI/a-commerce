//! The `page_image` resource (docs/api/catalog.md#page_image): generated pictures for store
//! pages that show no single product (the top of a curated page, a banner). What its operation
//! and the serving route share: the image model connection, the id made from a request, and the
//! SQL.
//!
//! The picture is made when the change is previewed, so the person approving it sees it, and
//! kept in `catalog.page_image`; previewing the same request again finds the same picture.
//! Shoppers can load a picture only once it is approved.

pub mod generate_page_image;

use std::sync::OnceLock;
use std::time::Duration;

use acom_contract::ToolError;
use acom_store::TxCx;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{GENERATION_FAILED, db};

/// The image model on the Meta Model API (Responses API with the `image_generation` tool).
#[derive(Debug)]
pub struct ImageModel {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    pub model: String,
}

static IMAGE_MODEL: OnceLock<ImageModel> = OnceLock::new();

/// A picture takes the model 10–20 seconds.
const UPSTREAM_SECONDS: u64 = 90;

/// Connects page images to an image model when `META_API_BASE_URL`, `META_API_KEY` and
/// `IMAGE_MODEL` are set; returns the model name, or the names that are missing (pictures are
/// then placeholders that say so).
pub fn connect_image_model(env: &dyn Fn(&str) -> Option<String>) -> Result<String, String> {
    let names = ["META_API_BASE_URL", "META_API_KEY", "IMAGE_MODEL"];
    let values: Vec<Option<String>> = names
        .iter()
        .map(|n| env(n).filter(|v| !v.trim().is_empty()))
        .collect();
    let missing: Vec<&str> = names
        .iter()
        .zip(&values)
        .filter(|(_, v)| v.is_none())
        .map(|(n, _)| *n)
        .collect();
    if !missing.is_empty() {
        return Err(format!("page images: set {}", missing.join(", ")));
    }
    let [base_url, api_key, model] = [0, 1, 2].map(|i| values[i].clone().unwrap_or_default());
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(UPSTREAM_SECONDS))
        .build()
        .map_err(|e| format!("page images: {e}"))?;
    let m = IMAGE_MODEL.get_or_init(|| ImageModel {
        http,
        base_url: base_url.trim_end_matches('/').to_owned(),
        api_key,
        model,
    });
    Ok(m.model.clone())
}

/// The connected image model, if any.
pub(crate) fn image_model() -> Option<&'static ImageModel> {
    IMAGE_MODEL.get()
}

/// A picture file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Picture {
    pub encoding_format: String,
    pub content: Vec<u8>,
}

fn generation_failed(why: &str, retryable: bool) -> ToolError {
    ToolError::new(
        GENERATION_FAILED,
        format!("The image model did not make the picture: {why}."),
    )
    .detail(json!({"upstream": "image_model", "retryable": retryable}))
}

impl ImageModel {
    /// One picture for `prompt`.
    pub(crate) async fn generate(&self, prompt: &str) -> Result<Picture, ToolError> {
        let res = self
            .http
            .post(format!("{}/responses", self.base_url))
            .bearer_auth(&self.api_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(
                json!({
                    "model": self.model,
                    "input": prompt,
                    "tools": [{"type": "image_generation"}],
                })
                .to_string(),
            )
            .send()
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "page image: no answer");
                generation_failed("no answer", true)
            })?;
        let status = res.status();
        let body = res.bytes().await.unwrap_or_default();
        if !status.is_success() {
            tracing::warn!(
                status = status.as_u16(),
                body = %String::from_utf8_lossy(&body[..body.len().min(300)]),
                "page image: refused"
            );
            return Err(generation_failed(
                &format!("HTTP {}", status.as_u16()),
                status.as_u16() == 429 || status.is_server_error(),
            ));
        }
        let v: Value = serde_json::from_slice(&body)
            .map_err(|_| generation_failed("an answer that is not JSON", true))?;
        picture_of(&v).ok_or_else(|| generation_failed("an answer without a picture", true))
    }
}

/// The first picture in a Responses API answer: an `image_generation_call` whose `result` is
/// the file in base64.
pub(crate) fn picture_of(answer: &Value) -> Option<Picture> {
    let call = answer
        .get("output")?
        .as_array()?
        .iter()
        .find(|o| o.get("type").and_then(Value::as_str) == Some("image_generation_call"))?;
    let content = base64::engine::general_purpose::STANDARD
        .decode(call.get("result")?.as_str()?)
        .ok()?;
    let encoding_format = sniff(&content)?.to_owned();
    Some(Picture {
        encoding_format,
        content,
    })
}

/// The media type of a picture file from its first bytes.
pub(crate) fn sniff(b: &[u8]) -> Option<&'static str> {
    if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("image/webp")
    } else if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if b.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else {
        None
    }
}

/// The id of a request: the same store, prompt and generator give the same id, so a preview
/// run again (approval previews again) finds the picture it made.
pub(crate) fn image_id(tenant: &str, prompt: &str, generator: &str) -> String {
    let digest = Sha256::digest(format!("{tenant}\n{generator}\n{prompt}").as_bytes());
    let hex: String = digest[..6].iter().map(|b| format!("{b:02x}")).collect();
    format!("img_page_{hex}")
}

/// Where a browser loads the picture (through the gateway).
pub fn content_url(image_id: &str) -> String {
    format!("/api/v1/catalog/page-images/{image_id}/content")
}

/// Text placed inside SVG markup.
fn xml_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The picture while no image model is connected: a wide grey card that shows the prompt and
/// says no generator made it. The same prompt gives the same picture.
pub(crate) fn placeholder(prompt: &str) -> Picture {
    let short: String = prompt.chars().take(90).collect();
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1500\" height=\"500\" viewBox=\"0 0 1500 500\">\
<rect width=\"1500\" height=\"500\" fill=\"#e7e5e0\"/>\
<text x=\"750\" y=\"240\" font-family=\"sans-serif\" font-size=\"30\" text-anchor=\"middle\" fill=\"#3d3b37\">{}</text>\
<text x=\"750\" y=\"290\" font-family=\"sans-serif\" font-size=\"22\" text-anchor=\"middle\" fill=\"#6b6862\">Placeholder · no image generator is connected</text>\
</svg>",
        xml_text(&short)
    );
    Picture {
        encoding_format: "image/svg+xml".into(),
        content: svg.into_bytes(),
    }
}

/// A stored page image without its file.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub(crate) struct PageImageRow {
    pub id: String,
    pub caption: String,
    pub generator: String,
    pub approved_at: Option<DateTime<Utc>>,
}

pub(crate) async fn row(tx: &mut TxCx, id: &str) -> Result<Option<PageImageRow>, ToolError> {
    sqlx::query_as::<_, PageImageRow>(
        "SELECT id, caption, generator, approved_at FROM catalog.page_image
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// What a new page image stores.
pub(crate) struct NewPageImage<'a> {
    pub id: &'a str,
    pub prompt: &'a str,
    pub caption: &'a str,
    pub generator: &'a str,
    pub picture: &'a Picture,
    pub at: DateTime<Utc>,
}

pub(crate) async fn insert(tx: &mut TxCx, n: &NewPageImage<'_>) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO catalog.page_image (tenant_id, id, prompt, caption, encoding_format, content,
           generator, is_ai_generated, date_created)
         VALUES ($1, $2, $3, $4, $5, $6, $7, true, $8)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(n.id)
    .bind(n.prompt)
    .bind(n.caption)
    .bind(&n.picture.encoding_format)
    .bind(&n.picture.content)
    .bind(n.generator)
    .bind(n.at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Sets or clears the approval; returns whether the image exists.
pub(crate) async fn set_approved(
    tx: &mut TxCx,
    id: &str,
    at: Option<DateTime<Utc>>,
) -> Result<bool, ToolError> {
    Ok(sqlx::query(
        "UPDATE catalog.page_image SET approved_at = $3, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?
    .rows_affected()
        > 0)
}

/// `GET /v1/catalog/page-images/{image_id}/content`: the media type and the file. Operators see
/// every picture (they approve them); everyone else only approved ones.
pub async fn page_image_content(
    tx: &mut TxCx,
    image_id: &str,
    operator: bool,
) -> Result<Option<(String, Vec<u8>)>, ToolError> {
    let found: Option<(String, Vec<u8>, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT encoding_format, content, approved_at FROM catalog.page_image
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(image_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(found
        .filter(|(_, _, approved)| operator || approved.is_some())
        .map(|(format, content, _)| (format, content)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_follow_the_request_and_answers_give_their_picture() {
        let a = image_id("store_a", "Wool coats on grey", "muse-image-1.0");
        assert!(a.starts_with("img_page_") && a.len() == "img_page_".len() + 12);
        assert_eq!(
            a,
            image_id("store_a", "Wool coats on grey", "muse-image-1.0")
        );
        assert_ne!(
            a,
            image_id("store_b", "Wool coats on grey", "muse-image-1.0")
        );
        assert_ne!(a, image_id("store_a", "Wool coats on grey", "placeholder"));

        let webp = b"RIFF\x10\x00\x00\x00WEBPVP8 ....";
        let b64 = base64::engine::general_purpose::STANDARD.encode(webp);
        let answer = json!({"output": [
            {"type": "reasoning"},
            {"type": "image_generation_call", "status": "completed", "result": b64},
        ]});
        let p = picture_of(&answer).unwrap();
        assert_eq!(p.encoding_format, "image/webp");
        assert_eq!(p.content, webp);
        assert!(picture_of(&json!({"output": [{"type": "message"}]})).is_none());
        assert!(
            picture_of(&json!({"output": [{"type": "image_generation_call", "result": "bm90IGFuIGltYWdl"}]}))
                .is_none()
        );

        let ph = placeholder("Coats <and> boots");
        assert_eq!(ph.encoding_format, "image/svg+xml");
        let svg = String::from_utf8(ph.content.clone()).unwrap();
        assert!(svg.contains("Coats &lt;and&gt; boots") && svg.contains("no image generator"));
        assert_eq!(ph, placeholder("Coats <and> boots"));
        assert!(connect_image_model(&|_| None).is_err());
    }
}
