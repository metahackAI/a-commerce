//! Specialist agents are data (`agents/*.yml` + instructions `agents/*.md`), loaded and checked
//! here (docs/backend/agent.md §2). All of them run on the same runtime and tools; they differ
//! in tool scope, instructions, blocks, and models.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use acom_llm::ModelsFile;
use serde::Deserialize;

use crate::blocks::KNOWN_BLOCKS;
use crate::tools::{ToolKind, ToolSpec};

/// Which chat an agent works in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatSurface {
    Bo,
    Fo,
}

impl ChatSurface {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bo => "bo",
            Self::Fo => "fo",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "bo" => Some(Self::Bo),
            "fo" => Some(Self::Fo),
            _ => None,
        }
    }
}

/// The tools an agent may find: their domains and kinds.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolScope {
    pub domains: Vec<String>,
    pub kinds: Vec<ToolKind>,
}

/// Model aliases from `agents/models.yml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelChoice {
    /// Routing, arguments, short plans, replies.
    pub worker: String,
    /// Plans with many steps or domains, and take-overs after a failure.
    pub planner: String,
}

/// One layout rule for product results (`agents/shopper.yml`, docs/api/chat.md §3.4).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutRule {
    /// The block to use: `compare_table`, `look_gallery`, `product_list`.
    pub ui: String,
    /// Phrases in the shopper's message that pick this layout.
    #[serde(default)]
    pub when_said: Vec<String>,
    /// Profile facts that pick it (`compares_specs`).
    #[serde(default)]
    pub when_profile: Vec<String>,
    /// Picks it when the shopper has no history and no profile.
    #[serde(default)]
    pub first_visit: bool,
    /// The one line shown as the block's `why`.
    pub why: String,
}

/// One specialist agent.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfile {
    pub id: String,
    pub surface: ChatSurface,
    /// The agent a surface falls back to when routing is unsure.
    #[serde(default)]
    pub default: bool,
    pub title: String,
    /// What the router reads to choose.
    pub description: String,
    /// Tool search weighs these domains up.
    #[serde(default)]
    pub home_domains: Vec<String>,
    pub tool_scope: ToolScope,
    /// Blocks this agent may answer with.
    pub blocks: Vec<String>,
    pub models: ModelChoice,
    /// File name of the instructions next to the YAML.
    pub instructions: String,
    #[serde(skip)]
    pub instructions_text: String,
    /// What a person can say to fix a failed check, by check key.
    #[serde(default)]
    pub check_intents: BTreeMap<String, String>,
    #[serde(default)]
    pub layout_rules: Vec<LayoutRule>,
}

impl AgentProfile {
    /// Whether a manifest tool is inside this agent's scope.
    pub fn allows(&self, tool: &ToolSpec) -> bool {
        tool.kind != ToolKind::Person
            && self.tool_scope.kinds.contains(&tool.kind)
            && self.tool_scope.domains.iter().any(|d| d == tool.domain())
    }

    pub fn is_home(&self, domain: &str) -> bool {
        self.home_domains.iter().any(|d| d == domain)
    }
}

/// `agents/runtime.yml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    pub limits: Limits,
    pub tool_search: ToolSearchConfig,
    pub router: RouterConfig,
    pub planner: PlannerConfig,
    pub context: ContextConfig,
    pub prompts: PromptFiles,
}

/// Per-turn ceilings (docs/backend/agent.md §1).
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub model_calls: u32,
    pub tool_calls: u32,
    pub plan_revisions: u32,
    pub seconds: u64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSearchConfig {
    pub min_candidates: usize,
    pub max_candidates: usize,
    pub home_domain_boost: f64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouterConfig {
    pub min_confidence: f64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannerConfig {
    pub worker_max_steps: usize,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextConfig {
    pub recent_turns: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptFiles {
    pub router: String,
    pub planner: String,
    pub arguments: String,
    pub reply: String,
}

/// The shared prompts, read from the files `runtime.yml` names.
#[derive(Debug, Clone, Default)]
pub struct Prompts {
    pub router: String,
    pub planner: String,
    pub arguments: String,
    pub reply: String,
}

/// Everything in `agents/`.
#[derive(Debug, Clone)]
pub struct Profiles {
    pub runtime: RuntimeConfig,
    pub prompts: Prompts,
    pub models: ModelsFile,
    pub agents: Vec<AgentProfile>,
}

const SPECIAL: &[&str] = &["models.yml", "runtime.yml"];

impl Profiles {
    /// Loads `dir` (`agents/`). Every problem is reported with the file it is in.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let read = |name: &str| {
            std::fs::read_to_string(dir.join(name))
                .map_err(|e| format!("{}: {e}", dir.join(name).display()))
        };
        let mut files = BTreeMap::new();
        let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".yml") {
                files.insert(name.clone(), read(&name)?);
            }
        }
        Self::from_files(&files, &read)
    }

    /// Builds from file contents: YAML files by name, other files read on demand.
    pub fn from_files(
        yaml: &BTreeMap<String, String>,
        read: &dyn Fn(&str) -> Result<String, String>,
    ) -> Result<Self, String> {
        let get = |name: &str| {
            yaml.get(name)
                .ok_or_else(|| format!("agents/{name} is missing"))
        };
        let runtime: RuntimeConfig = serde_norway::from_str(get("runtime.yml")?)
            .map_err(|e| format!("agents/runtime.yml: {e}"))?;
        let models = ModelsFile::parse(get("models.yml")?)?;
        let prompts = Prompts {
            router: read(&runtime.prompts.router)?,
            planner: read(&runtime.prompts.planner)?,
            arguments: read(&runtime.prompts.arguments)?,
            reply: read(&runtime.prompts.reply)?,
        };
        let mut agents = Vec::new();
        for (name, text) in yaml {
            if SPECIAL.contains(&name.as_str()) {
                continue;
            }
            let mut a: AgentProfile =
                serde_norway::from_str(text).map_err(|e| format!("agents/{name}: {e}"))?;
            a.instructions_text = read(&a.instructions)?;
            agents.push(a);
        }
        let p = Self {
            runtime,
            prompts,
            models,
            agents,
        };
        p.validate()?;
        Ok(p)
    }

    fn validate(&self) -> Result<(), String> {
        let mut problems = Vec::new();
        let mut ids = BTreeSet::new();
        for a in &self.agents {
            if !ids.insert(a.id.as_str()) {
                problems.push(format!("agent `{}` is defined twice", a.id));
            }
            for alias in [&a.models.worker, &a.models.planner] {
                if !self.models.models.contains_key(alias) {
                    problems.push(format!("{}: model `{alias}` is not in models.yml", a.id));
                }
            }
            for b in &a.blocks {
                if !KNOWN_BLOCKS.contains(&b.as_str()) {
                    problems.push(format!("{}: unknown block `{b}`", a.id));
                }
            }
            if a.tool_scope.kinds.contains(&ToolKind::Person) {
                problems.push(format!(
                    "{}: person-only endpoints can never be in a tool scope",
                    a.id
                ));
            }
            for r in &a.layout_rules {
                if !a.blocks.contains(&r.ui) {
                    problems.push(format!(
                        "{}: layout rule uses `{}`, not in its blocks",
                        a.id, r.ui
                    ));
                }
            }
        }
        for s in [ChatSurface::Bo, ChatSurface::Fo] {
            let defaults = self
                .agents
                .iter()
                .filter(|a| a.surface == s && a.default)
                .count();
            if defaults != 1 {
                problems.push(format!(
                    "surface {} needs exactly one default agent; it has {defaults}",
                    s.as_str()
                ));
            }
        }
        let l = &self.runtime.tool_search;
        if l.min_candidates == 0 || l.min_candidates > l.max_candidates {
            problems.push("tool_search: need 0 < min_candidates <= max_candidates".into());
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems.join("; "))
        }
    }

    pub fn get(&self, id: &str) -> Option<&AgentProfile> {
        self.agents.iter().find(|a| a.id == id)
    }

    pub fn for_surface(&self, s: ChatSurface) -> Vec<&AgentProfile> {
        self.agents.iter().filter(|a| a.surface == s).collect()
    }

    /// The surface's default agent (validation guarantees there is one).
    pub fn default_for(&self, s: ChatSurface) -> Option<&AgentProfile> {
        self.agents.iter().find(|a| a.surface == s && a.default)
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use std::path::PathBuf;

    use super::Profiles;

    /// The repository's own `agents/` folder.
    pub fn repo() -> Profiles {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../agents");
        Profiles::load(&dir).unwrap_or_else(|e| panic!("agents/: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::fixtures::{get, manifest};

    #[test]
    fn the_repository_agents_load_and_validate() {
        let p = fixtures::repo();
        assert_eq!(p.default_for(ChatSurface::Bo).unwrap().id, "promotion");
        assert_eq!(p.default_for(ChatSurface::Fo).unwrap().id, "shopper");
        assert!(p.for_surface(ChatSurface::Bo).len() >= 2);
        assert!(!p.prompts.planner.is_empty());
        assert!(p.runtime.limits.model_calls > 0);
        let shopper = p.get("shopper").unwrap();
        assert!(!shopper.layout_rules.is_empty());
        assert!(!shopper.instructions_text.is_empty());
    }

    #[test]
    fn scope_filters_domains_and_kinds() {
        let p = fixtures::repo();
        let m = manifest();
        let shopper = p.get("shopper").unwrap();
        assert!(shopper.allows(get(&m, "add_to_cart")));
        assert!(!shopper.allows(get(&m, "set_sale_prices")));
        let promotion = p.get("promotion").unwrap();
        assert!(promotion.allows(get(&m, "set_sale_prices")));
        assert!(!promotion.allows(get(&m, "add_to_cart")));
    }

    #[test]
    fn bad_definitions_are_refused_with_the_reason() {
        let mut files = BTreeMap::new();
        files.insert(
            "runtime.yml".to_owned(),
            include_str!("../../../agents/runtime.yml").to_owned(),
        );
        files.insert(
            "models.yml".to_owned(),
            include_str!("../../../agents/models.yml").to_owned(),
        );
        files.insert(
            "x.yml".to_owned(),
            "id: x\nsurface: bo\ndefault: true\ntitle: X\ndescription: X\n\
             tool_scope: {domains: [sales], kinds: [read, person]}\nblocks: [fancy]\n\
             models: {worker: glimmer, planner: nope}\ninstructions: x.md\n"
                .to_owned(),
        );
        let read = |_: &str| Ok("text".to_owned());
        let e = Profiles::from_files(&files, &read).unwrap_err();
        assert!(e.contains("model `nope`"), "{e}");
        assert!(e.contains("unknown block `fancy`"), "{e}");
        assert!(e.contains("person-only"), "{e}");
        assert!(e.contains("surface fo needs exactly one default"), "{e}");
    }
}
