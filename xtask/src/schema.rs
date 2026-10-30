//! `cargo xtask check-schema [--write]`: enforce docs/backend/database.md on `migrations/*.sql`
//! and generate the data dictionary (`docs/backend/schema.generated.md`,
//! `docs/api/generated/schema-map.json`).
//!
//! The parser understands the DDL style the migrations use (one column per line, named table
//! constraints, `COMMENT ON`, RLS statements). Syntax itself is the database's job; this
//! checks the rules that make the schema readable by people and agents.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::Serialize;

/// Name tokens that are abbreviations (docs/backend/database.md §1.6).
const BANNED: &[&str] = &[
    "qty", "amt", "cnt", "dt", "cd", "nm", "no", "yn", "flg", "seq", "desc", "addr", "tel", "num",
    "pct", "img", "info", "tmp", "val", "dtm", "ymd", "cust", "prod", "ord", "msg", "usr",
];

#[derive(Debug, Clone, Default, Serialize)]
pub struct Column {
    pub name: String,
    #[serde(rename = "type")]
    pub sql_type: String,
    pub nullable: bool,
    pub term: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub values_list: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub references: Option<String>,
    #[serde(skip)]
    pub has_default: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Table {
    pub schema: String,
    pub table: String,
    pub kind: &'static str,
    pub term: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id_prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_name: Option<String>,
    pub columns: Vec<Column>,
    #[serde(skip)]
    pub primary_key: Vec<String>,
    #[serde(skip)]
    rls_enabled: bool,
    #[serde(skip)]
    rls_forced: bool,
    #[serde(skip)]
    has_policy: bool,
    #[serde(skip)]
    id_checked: bool,
    #[serde(skip)]
    comment_seen: bool,
    #[serde(skip)]
    file: String,
}

impl Table {
    fn full_name(&self) -> String {
        format!("{}.{}", self.schema, self.table)
    }

    fn column_mut(&mut self, name: &str) -> Option<&mut Column> {
        self.columns.iter_mut().find(|c| c.name == name)
    }

    fn has_column(&self, name: &str) -> bool {
        self.columns.iter().any(|c| c.name == name)
    }
}

#[derive(Debug, Default)]
pub struct Schema {
    pub tables: BTreeMap<String, Table>,
    column_comments: Vec<(String, String, String)>,
    pub problems: Vec<String>,
}

/// Splits SQL into statements, dropping `--` and `/* */` comments and respecting quotes.
pub fn split_statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = sql.chars().collect();
    let mut i = 0;
    let mut in_quote = false;
    while i < chars.len() {
        let c = chars[i];
        if in_quote {
            cur.push(c);
            if c == '\'' {
                if chars.get(i + 1) == Some(&'\'') {
                    cur.push('\'');
                    i += 1;
                } else {
                    in_quote = false;
                }
            }
        } else if c == '\'' {
            in_quote = true;
            cur.push(c);
        } else if c == '-' && chars.get(i + 1) == Some(&'-') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            cur.push('\n');
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 1;
        } else if c == ';' {
            let s = cur.trim().to_owned();
            if !s.is_empty() {
                out.push(s);
            }
            cur.clear();
        } else {
            cur.push(c);
        }
        i += 1;
    }
    let s = cur.trim().to_owned();
    if !s.is_empty() {
        out.push(s);
    }
    out
}

/// Splits at top-level commas (outside parentheses and quotes).
fn split_top_level(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut in_quote = false;
    let mut cur = String::new();
    for c in body.chars() {
        match c {
            '\'' => {
                in_quote = !in_quote;
                cur.push(c);
            }
            '(' if !in_quote => {
                depth += 1;
                cur.push(c);
            }
            ')' if !in_quote => {
                depth -= 1;
                cur.push(c);
            }
            ',' if !in_quote && depth == 0 => {
                out.push(cur.trim().to_owned());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_owned());
    }
    out
}

fn upper(s: &str) -> String {
    s.to_ascii_uppercase()
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `'it''s'` → `it's`.
fn unquote(s: &str) -> Option<String> {
    let s = s.trim();
    let inner = s.strip_prefix('\'')?.strip_suffix('\'')?;
    Some(inner.replace("''", "'"))
}

/// Finds `<col> IN ('a', 'b')` inside a CHECK expression.
fn check_in_list(expr: &str) -> Option<(String, Vec<String>)> {
    let u = upper(expr);
    let pos = u.find(" IN (")?;
    let before = expr[..pos].trim_end();
    let col = before
        .rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .next()?
        .to_owned();
    let rest = &expr[pos + 5..];
    let end = rest.find(')')?;
    let words = rest[..end]
        .split(',')
        .filter_map(unquote)
        .collect::<Vec<_>>();
    (!words.is_empty() && !col.is_empty()).then_some((col.to_ascii_lowercase(), words))
}

fn parse_qualified(name: &str) -> Option<(String, String)> {
    let name = name.trim().trim_matches('"');
    let (s, t) = name.split_once('.')?;
    Some((
        s.to_ascii_lowercase(),
        t.trim_matches('"').to_ascii_lowercase(),
    ))
}

fn parens_inner(s: &str) -> Option<&str> {
    let start = s.find('(')?;
    let mut depth = 0;
    for (i, c) in s[start..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[start + 1..start + i]);
                }
            }
            _ => {}
        }
    }
    None
}

const COLUMN_STOP: &[&str] = &[
    " NOT NULL",
    " NULL",
    " DEFAULT",
    " REFERENCES",
    " CHECK",
    " PRIMARY KEY",
    " UNIQUE",
    " CONSTRAINT",
    " GENERATED",
    " COLLATE",
];

fn parse_column(def: &str) -> Column {
    let def = collapse(def);
    let (name, rest) = def.split_once(' ').unwrap_or((&def, ""));
    let rest_u = format!(" {}", upper(rest));
    let type_end = COLUMN_STOP
        .iter()
        .filter_map(|k| rest_u.find(k))
        .min()
        .unwrap_or(rest_u.len());
    let sql_type = rest_u[..type_end].trim().to_ascii_lowercase();
    let nullable = !rest_u.contains(" NOT NULL") && !rest_u.contains(" PRIMARY KEY");
    let references = rest_u.find(" REFERENCES ").map(|p| {
        let after = &rest[p + " REFERENCES ".len() - 1..];
        after
            .split(|c: char| c == '(' || c.is_whitespace())
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
    });
    let mut col = Column {
        name: name.trim_matches('"').to_ascii_lowercase(),
        sql_type,
        nullable,
        references,
        has_default: rest_u.contains(" DEFAULT "),
        ..Default::default()
    };
    if let Some(p) = rest_u.find(" CHECK") {
        let check = &rest[p..];
        if let Some(inner) = parens_inner(check)
            && let Some((c, words)) = check_in_list(inner)
            && c == col.name
        {
            col.values = Some(words);
        }
    }
    col
}

impl Schema {
    pub fn parse_dir(dir: &Path) -> std::io::Result<Schema> {
        let mut files: Vec<_> = fs::read_dir(dir)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "sql"))
            .collect();
        files.sort();
        let mut schema = Schema::default();
        for f in files {
            let name = f
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            schema.parse_sql(&name, &fs::read_to_string(&f)?);
        }
        schema.finish();
        Ok(schema)
    }

    pub fn parse_sql(&mut self, file: &str, sql: &str) {
        for stmt in split_statements(sql) {
            self.statement(file, &stmt);
        }
    }

    fn statement(&mut self, file: &str, stmt: &str) {
        let flat = collapse(stmt);
        let u = upper(&flat);
        if let Some(rest) = u.strip_prefix("CREATE TABLE ") {
            let rest = rest.strip_prefix("IF NOT EXISTS ").unwrap_or(rest);
            let offset = flat.len() - rest.len();
            let name = flat[offset..].split(['(', ' ']).next().unwrap_or("");
            let Some((s, t)) = parse_qualified(name) else {
                self.problems
                    .push(format!("{file}: table `{name}` is not schema-qualified"));
                return;
            };
            let body = parens_inner(stmt).unwrap_or("");
            let mut table = Table {
                schema: s,
                table: t,
                kind: "table",
                file: file.to_owned(),
                ..Default::default()
            };
            for item in split_top_level(body) {
                let iu = upper(&collapse(&item));
                if [
                    "CONSTRAINT ",
                    "PRIMARY KEY",
                    "FOREIGN KEY",
                    "UNIQUE",
                    "CHECK",
                    "EXCLUDE",
                ]
                .iter()
                .any(|k| is_keyword_start(&iu, k))
                {
                    self.table_constraint(&mut table, &item);
                } else {
                    let col = parse_column(&item);
                    if iu.contains(" PRIMARY KEY") {
                        table.primary_key = vec![col.name.clone()];
                    }
                    if col.name == "id" && iu.contains("CHECK") {
                        table.id_checked = true;
                    }
                    table.columns.push(col);
                }
            }
            self.tables.insert(table.full_name(), table);
        } else if let Some(rest) = [
            "CREATE OR REPLACE VIEW ",
            "CREATE VIEW ",
            "CREATE MATERIALIZED VIEW ",
        ]
        .iter()
        .find_map(|p| u.strip_prefix(p))
        {
            let offset = flat.len() - rest.len();
            let name = flat[offset..].split([' ', '(']).next().unwrap_or("");
            if let Some((s, t)) = parse_qualified(name) {
                let table = Table {
                    schema: s,
                    table: t,
                    kind: "view",
                    file: file.to_owned(),
                    ..Default::default()
                };
                self.tables.insert(table.full_name(), table);
            }
        } else if u.starts_with("COMMENT ON ") {
            self.comment(file, &flat);
        } else if let Some(rest) = u.strip_prefix("ALTER TABLE ") {
            let offset = flat.len() - rest.len();
            let mut parts = flat[offset..].splitn(2, ' ');
            let name = parts.next().unwrap_or("").to_ascii_lowercase();
            let action = parts.next().unwrap_or("");
            let au = upper(action);
            let Some(table) = self.tables.get_mut(&name) else {
                self.problems
                    .push(format!("{file}: ALTER TABLE on unknown table `{name}`"));
                return;
            };
            if au.starts_with("ENABLE ROW LEVEL SECURITY") {
                table.rls_enabled = true;
            } else if au.starts_with("FORCE ROW LEVEL SECURITY") {
                table.rls_forced = true;
            } else if let Some(def) = au.strip_prefix("ADD COLUMN ") {
                let def = &action[action.len() - def.len()..];
                let def = def.strip_prefix("IF NOT EXISTS ").unwrap_or(def);
                table.columns.push(parse_column(def));
            } else if au.starts_with("ADD CONSTRAINT") {
                let item = action["ADD ".len()..].to_owned();
                let mut t = std::mem::take(table);
                self.table_constraint(&mut t, &item);
                self.tables.insert(name, t);
            }
        } else if u.starts_with("CREATE POLICY ")
            && let Some(p) = u.find(" ON ")
        {
            let name = flat[p + 4..]
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if let Some(t) = self.tables.get_mut(&name) {
                t.has_policy = true;
            } else {
                self.problems
                    .push(format!("{file}: policy on unknown table `{name}`"));
            }
        }
    }

    fn table_constraint(&mut self, table: &mut Table, item: &str) {
        let flat = collapse(item);
        let u = upper(&flat);
        if let Some(p) = u.find("PRIMARY KEY") {
            if let Some(inner) = parens_inner(&flat[p..]) {
                table.primary_key = inner
                    .split(',')
                    .map(|c| c.trim().to_ascii_lowercase())
                    .collect();
            }
        } else if let Some(p) = u.find("FOREIGN KEY") {
            let cols: Vec<String> = parens_inner(&flat[p..])
                .unwrap_or("")
                .split(',')
                .map(|c| c.trim().to_ascii_lowercase())
                .collect();
            let target = u
                .find(" REFERENCES ")
                .map(|r| {
                    flat[r + 12..]
                        .split(|c: char| c == '(' || c.is_whitespace())
                        .next()
                        .unwrap_or("")
                        .to_ascii_lowercase()
                })
                .unwrap_or_default();
            for c in cols.iter().filter(|c| c.as_str() != "tenant_id") {
                if let Some(col) = table.column_mut(c) {
                    col.references = Some(target.clone());
                } else {
                    self.problems.push(format!(
                        "{}: foreign key on unknown column `{c}`",
                        table.full_name()
                    ));
                }
            }
        } else if let Some(p) = u.find("CHECK")
            && let Some(inner) = parens_inner(&flat[p..])
        {
            if inner.contains("id ~") || inner.contains("id~") {
                table.id_checked = true;
            }
            if let Some((col, words)) = check_in_list(inner)
                && let Some(c) = table.column_mut(&col)
            {
                c.values = Some(words);
            }
        }
    }

    fn comment(&mut self, file: &str, flat: &str) {
        let u = upper(flat);
        let Some(is_pos) = u.find(" IS ") else { return };
        let target = &flat["COMMENT ON ".len()..is_pos];
        let Some(text) = unquote(&flat[is_pos + 4..]) else {
            self.problems
                .push(format!("{file}: comment is not a single string: {flat}"));
            return;
        };
        let tu = upper(target);
        let name = target
            .split_whitespace()
            .last()
            .unwrap_or("")
            .to_ascii_lowercase();
        if tu.starts_with("COLUMN ") {
            let parts: Vec<&str> = name.split('.').collect();
            if parts.len() == 3 {
                self.column_comments.push((
                    format!("{}.{}", parts[0], parts[1]),
                    parts[2].to_owned(),
                    text,
                ));
            } else {
                self.problems.push(format!(
                    "{file}: column comment target `{name}` is not schema.table.column"
                ));
            }
        } else if tu.starts_with("TABLE ")
            || tu.starts_with("VIEW ")
            || tu.starts_with("MATERIALIZED VIEW ")
        {
            match self.tables.get_mut(&name) {
                Some(t) => {
                    let (term, description) = split_comment(&text);
                    t.term = term;
                    t.description = description;
                    t.comment_seen = true;
                    t.id_prefix = between(&text, "id prefix ", "_").map(str::to_owned);
                    t.api_name = between(&text, "API: ", ".").map(str::to_owned);
                }
                None => self
                    .problems
                    .push(format!("{file}: comment on unknown table `{name}`")),
            }
        }
    }

    fn finish(&mut self) {
        let comments = std::mem::take(&mut self.column_comments);
        for (table, column, text) in comments {
            let Some(t) = self.tables.get_mut(&table) else {
                self.problems.push(format!(
                    "comment on column of unknown table `{table}.{column}`"
                ));
                continue;
            };
            let is_view = t.kind == "view";
            if !t.has_column(&column) {
                if is_view {
                    t.columns.push(Column {
                        name: column.clone(),
                        nullable: true,
                        ..Default::default()
                    });
                } else {
                    self.problems
                        .push(format!("comment on unknown column `{table}.{column}`"));
                    continue;
                }
            }
            if let Some(c) = t.column_mut(&column) {
                let (term, description) = split_comment(&text);
                c.term = term;
                c.description = description;
                c.values_list = between(&text, "values: ", " ")
                    .or_else(|| text.split("values: ").nth(1))
                    .map(|s| s.trim_end_matches('.').trim().to_owned());
            }
        }
    }

    /// The rules of docs/backend/database.md that can be checked from DDL.
    pub fn check(&self, lists: &dyn Fn(&str) -> Option<&'static [&'static str]>) -> Vec<String> {
        let mut out = self.problems.clone();
        for (name, t) in &self.tables {
            let p = |msg: String| format!("{name} ({}): {msg}", t.file);
            if !t.comment_seen {
                out.push(p("no COMMENT ON TABLE".into()));
            } else if !valid_term(&t.term, true) {
                out.push(p(format!(
                    "table term must be schema:<Type>, schema:<Type>/acom:<Term>, acom:<Term>, or -, got `{}`",
                    t.term
                )));
            } else if t.description.is_empty() {
                out.push(p("table comment needs a sentence after the term".into()));
            }
            check_name(&t.table)
                .into_iter()
                .for_each(|m| out.push(p(m)));
            if t.kind == "view" {
                continue;
            }
            let scoped = name != "platform.online_store";
            if scoped {
                if !t.has_column("tenant_id") {
                    out.push(p("store-scoped table has no tenant_id".into()));
                }
                if !(t.rls_enabled && t.rls_forced && t.has_policy) {
                    out.push(p(
                        "needs ENABLE + FORCE ROW LEVEL SECURITY and a policy".into()
                    ));
                }
                if t.has_column("id")
                    && !t
                        .primary_key
                        .starts_with(&["tenant_id".to_owned(), "id".to_owned()])
                {
                    out.push(p(format!(
                        "primary key must be (tenant_id, id), got {:?}",
                        t.primary_key
                    )));
                }
            }
            if t.primary_key.is_empty() {
                out.push(p("no primary key".into()));
            }
            if t.has_column("id") && !t.id_checked {
                out.push(p("id has no format CHECK".into()));
            }
            for c in &t.columns {
                let cp = |msg: String| p(format!("column `{}`: {msg}", c.name));
                check_name(&c.name)
                    .into_iter()
                    .for_each(|m| out.push(cp(m)));
                if c.term.is_empty() && c.description.is_empty() {
                    out.push(cp("no COMMENT".into()));
                } else if !valid_term(&c.term, true) {
                    out.push(cp(format!(
                        "comment must start with schema:/acom:/-, got `{}`",
                        c.term
                    )));
                } else if c.description.is_empty() {
                    out.push(cp("comment needs ` — <sentence>` after the term".into()));
                }
                // Tier ids are store-chosen words, so a tier reference keeps its schema.org
                // property name (`valid_for_member_tier`), as database.md §3 allows.
                let tier_reference =
                    c.references.as_deref() == Some("customer.member_program_tier");
                if c.references.is_some()
                    && !c.name.ends_with("_id")
                    && c.name != "id"
                    && !tier_reference
                {
                    out.push(cp("reference columns end with _id".into()));
                }
                if c.sql_type.starts_with("timestamp")
                    && !(c.sql_type == "timestamptz" || c.sql_type.contains("with time zone"))
                {
                    out.push(cp("timestamps are timestamptz".into()));
                }
                if c.sql_type.starts_with("float")
                    || c.sql_type.starts_with("real")
                    || c.sql_type.starts_with("double")
                {
                    out.push(cp("no floating point columns".into()));
                }
                // `rating_value` is the schema.org property ratingValue, not a value/unit pair.
                let is_schema_property = c.term == format!("schema:{}", lower_camel(&c.name));
                if let Some(base) = c.name.strip_suffix("_value")
                    && !is_schema_property
                    && !t.has_column(&format!("{base}_currency"))
                    && !t.has_column(&format!("{base}_unit_text"))
                {
                    out.push(cp(format!(
                        "needs a `{base}_currency` or `{base}_unit_text` column"
                    )));
                }
                if c.name == "price" && !t.has_column("price_currency") {
                    out.push(cp("needs price_currency".into()));
                }
                if let Some(values) = &c.values {
                    match c.values_list.as_deref() {
                        None => {
                            out.push(cp("CHECK list needs `values: <list>` in the comment".into()))
                        }
                        Some(list) => match lists(list) {
                            None => out.push(cp(format!(
                                "`values: {list}` is not a list in acom-core enums"
                            ))),
                            Some(words) => {
                                let have: Vec<&str> = values.iter().map(String::as_str).collect();
                                if have != words {
                                    let a: BTreeSet<&str> = have.iter().copied().collect();
                                    let b: BTreeSet<&str> = words.iter().copied().collect();
                                    let missing: Vec<_> = b.difference(&a).collect();
                                    let extra: Vec<_> = a.difference(&b).collect();
                                    out.push(cp(format!(
                                        "CHECK differs from {list}: missing {missing:?}, extra {extra:?}{}",
                                        if missing.is_empty() && extra.is_empty() { " (order differs)" } else { "" }
                                    )));
                                }
                            }
                        },
                    }
                }
            }
        }
        out
    }

    pub fn markdown(&self) -> String {
        let mut s = String::from(
            "# 컬럼 사전 (생성됨)\n\n> `cargo xtask check-schema --write`가 `migrations/*.sql`에서 만든 파일입니다. 손으로 고치지 말고 마이그레이션의 주석을 고치세요. 규칙: [database.md](database.md).\n",
        );
        let mut current = String::new();
        for t in self.tables.values() {
            if t.schema != current {
                current = t.schema.clone();
                s.push_str(&format!("\n## `{current}`\n"));
            }
            s.push_str(&format!(
                "\n### `{}.{}`{}\n\n",
                t.schema,
                t.table,
                if t.kind == "view" { " (view)" } else { "" }
            ));
            s.push_str(&format!("`{}` — {}", t.term, t.description));
            if let Some(p) = &t.id_prefix {
                s.push_str(&format!(" · id `{p}_…`"));
            }
            s.push_str(
                "\n\n| column | type | null | term | description |\n|---|---|:--:|---|---|\n",
            );
            for c in &t.columns {
                let mut d = c.description.replace('|', "\\|");
                if let Some(v) = &c.values {
                    d.push_str(&format!(" Values: `{}`.", v.join("` · `")));
                }
                if let Some(r) = &c.references {
                    d.push_str(&format!(" → `{r}`"));
                }
                s.push_str(&format!(
                    "| `{}` | {} | {} | `{}` | {} |\n",
                    c.name,
                    c.sql_type,
                    if c.nullable { "✓" } else { "" },
                    c.term,
                    d
                ));
            }
        }
        s
    }

    pub fn json(&self) -> String {
        let tables: Vec<&Table> = self.tables.values().collect();
        serde_json::to_string_pretty(&tables).unwrap_or_default() + "\n"
    }
}

/// `schema:Offer/acom:Coupon — sentence` → (`schema:Offer/acom:Coupon`, `sentence`);
/// `- sentence` (technical) → (`-`, `sentence`).
fn split_comment(text: &str) -> (String, String) {
    if let Some(rest) = text.strip_prefix("- ") {
        return ("-".to_owned(), rest.trim().to_owned());
    }
    match text.split_once(" — ") {
        Some((term, d)) => (term.trim().to_owned(), d.trim().to_owned()),
        None => (text.trim().to_owned(), String::new()),
    }
}

/// A term token: `schema:<Name>`, `acom:<Name>`, `schema:<Type>/acom:<Term>`, or `-`.
fn valid_term(term: &str, allow_technical: bool) -> bool {
    fn word(s: &str, prefix: &str) -> bool {
        s.strip_prefix(prefix)
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_alphanumeric()))
    }
    if term == "-" {
        return allow_technical;
    }
    match term.split_once('/') {
        Some((a, b)) => word(a, "schema:") && word(b, "acom:"),
        None => word(term, "schema:") || word(term, "acom:"),
    }
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = text.find(start)? + start.len();
    let rest = &text[i..];
    let j = rest.find(end)?;
    Some(rest[..j].trim())
}

/// `CHECK` starts a constraint, `checks jsonb` does not.
fn is_keyword_start(item_upper: &str, keyword: &str) -> bool {
    item_upper.strip_prefix(keyword).is_some_and(|rest| {
        keyword.ends_with(' ') || rest.is_empty() || rest.starts_with(' ') || rest.starts_with('(')
    })
}

/// `rating_value` → `ratingValue`.
pub fn lower_camel(snake: &str) -> String {
    let mut out = String::new();
    let mut upper_next = false;
    for ch in snake.chars() {
        if ch == '_' {
            upper_next = true;
        } else if upper_next {
            out.extend(ch.to_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

fn check_name(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let snake = !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && name.bytes().next().is_some_and(|b| b.is_ascii_lowercase());
    if !snake {
        out.push(format!("`{name}` is not snake_case"));
    }
    for token in name.split('_') {
        if BANNED.contains(&token) {
            out.push(format!("`{name}` uses the abbreviation `{token}`"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQL: &str = r#"
CREATE TABLE platform.online_store (
  id text PRIMARY KEY CHECK (id ~ '^store_[a-z0-9_]+$'),
  name text NOT NULL
);
CREATE TABLE catalog.offer (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  product_id text NOT NULL,
  price numeric(12,2) NOT NULL,
  price_currency char(3) NOT NULL,
  availability text NOT NULL DEFAULT 'in_stock',
  created_at timestamptz NOT NULL DEFAULT now(),
  CONSTRAINT offer_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT offer_id_check CHECK (id ~ '^off_[a-z0-9_]+$'),
  CONSTRAINT offer_product_fkey FOREIGN KEY (tenant_id, product_id) REFERENCES catalog.product (tenant_id, id),
  CONSTRAINT offer_availability_check CHECK (availability IN ('in_stock', 'out_of_stock', 'pre_order', 'back_order', 'discontinued'))
);
COMMENT ON TABLE platform.online_store IS 'schema:OnlineStore — a store. id prefix store_.';
COMMENT ON COLUMN platform.online_store.id IS '- the store id.';
COMMENT ON COLUMN platform.online_store.name IS 'schema:name — the store''s name; shown to shoppers.';
COMMENT ON TABLE catalog.offer IS 'schema:Offer/acom:VariantOffer — price of one variant. id prefix off_.';
COMMENT ON COLUMN catalog.offer.tenant_id IS '- store that owns this row.';
COMMENT ON COLUMN catalog.offer.id IS '- offer id.';
COMMENT ON COLUMN catalog.offer.product_id IS 'schema:itemOffered — the variant (var_ id; API variant_id).';
COMMENT ON COLUMN catalog.offer.price IS 'schema:price — list price.';
COMMENT ON COLUMN catalog.offer.price_currency IS 'schema:priceCurrency — ISO 4217 code.';
COMMENT ON COLUMN catalog.offer.availability IS 'schema:availability — can it be bought now. values: availability';
COMMENT ON COLUMN catalog.offer.created_at IS '- when the row was created.';
ALTER TABLE catalog.offer ENABLE ROW LEVEL SECURITY;
ALTER TABLE catalog.offer FORCE ROW LEVEL SECURITY;
CREATE POLICY offer_tenant_isolation ON catalog.offer USING (tenant_id = current_setting('app.tenant_id', true));
"#;

    fn parsed(sql: &str) -> Schema {
        let mut s = Schema::default();
        s.parse_sql("t.sql", sql);
        s.finish();
        s
    }

    #[test]
    fn a_good_schema_passes() {
        let s = parsed(SQL);
        let problems = s.check(&acom_core::enums::words);
        assert!(problems.is_empty(), "{problems:#?}");
        let offer = &s.tables["catalog.offer"];
        assert_eq!(offer.id_prefix.as_deref(), Some("off"));
        assert_eq!(
            offer.columns[2].references.as_deref(),
            Some("catalog.product")
        );
        assert_eq!(
            offer.columns[5].values_list.as_deref(),
            Some("availability")
        );
        assert!(s.markdown().contains("`catalog.offer`"));
        assert!(
            s.json()
                .contains("\"term\": \"schema:Offer/acom:VariantOffer\"")
        );
        assert!(valid_term("schema:Offer/acom:Coupon", false));
        assert!(!valid_term("acom:Coupon/schema:Offer", false));
        assert!(!valid_term("-", false));
    }

    #[test]
    fn reports_rule_violations() {
        let bad = SQL
            .replace("price_currency char(3) NOT NULL,", "order_qty integer NOT NULL,")
            .replace("'pre_order', ", "")
            .replace("ALTER TABLE catalog.offer FORCE ROW LEVEL SECURITY;", "")
            .replace("COMMENT ON COLUMN catalog.offer.created_at IS '- when the row was created.';", "")
            .replace("COMMENT ON COLUMN catalog.offer.price_currency IS 'schema:priceCurrency — ISO 4217 code.';", "");
        let problems = parsed(&bad).check(&acom_core::enums::words).join("\n");
        assert!(problems.contains("abbreviation `qty`"), "{problems}");
        assert!(problems.contains("needs price_currency"), "{problems}");
        assert!(problems.contains("missing [\"pre_order\"]"), "{problems}");
        assert!(problems.contains("ROW LEVEL SECURITY"), "{problems}");
        assert!(problems.contains("`created_at`: no COMMENT"), "{problems}");
    }

    #[test]
    fn splits_statements_around_quotes_and_comments() {
        let s = split_statements("COMMENT ON TABLE a.b IS 'x; y''s'; -- note; here\nSELECT 1;");
        assert_eq!(s.len(), 2);
        assert!(s[0].ends_with("'x; y''s'"));
    }
}

#[cfg(test)]
mod vocab_tests {
    use std::collections::BTreeMap;

    #[test]
    fn vocabulary_defines_every_acom_list_with_the_same_words() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../vocab/acom.jsonld");
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let graph = doc["@graph"].as_array().unwrap();
        let mut by_list: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
        for node in graph {
            if let Some(list) = node["acom:wireList"].as_str() {
                by_list.insert(
                    list.to_owned(),
                    (node["@id"].as_str().unwrap().to_owned(), Vec::new()),
                );
            }
        }
        for node in graph {
            if let (Some(t), Some(w)) = (node["@type"].as_str(), node["acom:wireWord"].as_str())
                && let Some((_, words)) = by_list.values_mut().find(|(id, _)| id == t)
            {
                words.push(w.to_owned());
            }
        }
        for (list, words) in acom_core::enums::LISTS {
            if crate::vocab::SCHEMA_ENUMERATIONS
                .iter()
                .any(|(l, _)| l == list)
            {
                assert!(
                    !by_list.contains_key(*list),
                    "{list} is a schema.org enumeration"
                );
                continue;
            }
            let (_, have) = by_list
                .get(*list)
                .unwrap_or_else(|| panic!("vocab/acom.jsonld lacks {list}"));
            assert_eq!(have, words, "{list}");
        }
    }
}
