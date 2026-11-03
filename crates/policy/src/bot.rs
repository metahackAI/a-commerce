//! Ailism Bot's settings (`policies/platform.yml bot`): whether it runs and how far it goes, who
//! its drafts are made for, which model analyses, the areas it looks at, and how many proposals
//! it may have open. When it runs is `watchers.ailism_bot`. Spec: `docs/api/platform.md#bot`.

use serde::{Deserialize, Serialize};

use crate::{PolicyError, PolicySet};

/// How far the bot goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BotMode {
    /// No cycle runs.
    Off,
    /// Cycles run and write a report; no task is opened.
    Observe,
    /// Cycles open tasks whose change sets the operations agents draft for a person to decide.
    Propose,
}

/// The areas the bot looks at.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BotAreas {
    pub pricing: bool,
    pub inventory: bool,
    pub promotions: bool,
    pub merchandising: bool,
    pub catalog_content: bool,
    pub returns: bool,
    pub questions_reviews: bool,
    pub community: bool,
}

impl BotAreas {
    /// Every area with whether it is on, in a fixed order.
    pub fn all(&self) -> [(&'static str, bool); 8] {
        [
            ("pricing", self.pricing),
            ("inventory", self.inventory),
            ("promotions", self.promotions),
            ("merchandising", self.merchandising),
            ("catalog_content", self.catalog_content),
            ("returns", self.returns),
            ("questions_reviews", self.questions_reviews),
            ("community", self.community),
        ]
    }

    pub fn is_on(&self, area: &str) -> bool {
        self.all().iter().any(|(a, on)| *a == area && *on)
    }
}

/// `platform.yml bot`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Bot {
    pub mode: BotMode,
    /// The operator the drafts are made for; `None`: `task_pickup.acts_as`.
    pub acts_as: Option<String>,
    /// The analysing model's alias in `agents/models.yml`.
    pub model: String,
    pub areas: BotAreas,
    pub max_proposals_per_cycle: u32,
    pub max_open_decisions: u32,
    pub cooldown_days: u32,
    pub directives: Vec<String>,
}

impl PolicySet {
    /// `platform.yml bot`, typed.
    pub fn bot(&self) -> Result<Bot, PolicyError> {
        let v = self
            .raw("platform")
            .and_then(|p| p.get("bot"))
            .ok_or_else(|| PolicyError::Schema {
                file: "platform".to_owned(),
                message: "`bot` is missing".to_owned(),
            })?;
        serde_json::from_value(v.clone()).map_err(|e| PolicyError::Schema {
            file: "platform".to_owned(),
            message: format!("bot: {e}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::defaults;

    #[test]
    fn the_default_bot_proposes_in_every_area_every_four_hours() {
        let p = defaults();
        let b = p.bot().unwrap();
        assert_eq!(b.mode, super::BotMode::Propose);
        assert_eq!(b.model, "glimmer");
        assert!(b.areas.all().iter().all(|(_, on)| *on));
        let w = p.watchers().unwrap().schedule("ailism_bot").unwrap();
        assert_eq!((w.enabled, w.every_minutes), (true, 240));
    }

    #[test]
    fn a_store_overrides_a_few_settings_and_keeps_the_rest() {
        let p = defaults()
            .with_overrides([(
                "platform",
                "bot:\n  mode: observe\n  areas:\n    community: false\n",
            )])
            .unwrap();
        let b = p.bot().unwrap();
        assert_eq!(b.mode, super::BotMode::Observe);
        assert!(!b.areas.is_on("community"));
        assert!(b.areas.is_on("pricing"));
        assert_eq!(b.max_proposals_per_cycle, 3);
    }
}
