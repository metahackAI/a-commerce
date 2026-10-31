//! `describe_data` (`platform.schema.describe`): tables and columns with their terms.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, ReadResult,
    Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{DataSchema, describe};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DescribeDataInput {
    /// A schema.org or acom term; only tables and columns that use it (`orderStatus`,
    /// `schema:Order`, `acom:percentOff`).
    #[schemars(example = &"orderStatus")]
    pub term: Option<String>,
    /// Only this schema.
    #[schemars(example = &"sales")]
    pub schema: Option<DataSchema>,
    /// One table, as `schema.table`.
    #[schemars(regex(pattern = r"^[a-z_]+\.[a-z_]+$"), example = &"sales.sales_order")]
    pub table: Option<String>,
    /// A name the API uses, for example `variant_id`.
    #[schemars(example = &"variant_id")]
    pub api_name: Option<String>,
}

pub struct DescribeData;

impl Operation for DescribeData {
    const META: OpMeta = OpMeta {
        address: "platform.schema.describe",
        tool: "describe_data",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/schema",
        summary: "The data dictionary: every table and column with its schema.org term, a one-sentence meaning, type, allowed values and references. Use it to learn what a field means or where a number lives.",
        consumes: &[],
        produces: &[Slot::many("table_name")],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = DescribeDataInput;
    type Output = ReadResult<Value>;
}

impl ReadOp<Cx> for DescribeData {
    async fn read(&self, _: &Cx, input: DescribeDataInput) -> Result<ReadResult<Value>, ToolError> {
        let items = describe(&input);
        if items.is_empty() {
            let (entity, id) = match (&input.table, &input.term) {
                (Some(t), _) => ("table", t.clone()),
                (_, Some(t)) => ("term", t.clone()),
                _ => ("table", String::new()),
            };
            return Err(ToolError::new(
                ErrorCode::NOT_FOUND,
                format!("Nothing in the data dictionary matches `{id}`."),
            )
            .detail(json!({"entity": entity, "id": id}))
            .next("describe_data"));
        }
        Ok(ReadResult::all(items))
    }
}
