//! The chat stream (docs/api/chat.md §2): the events a turn sends and their SSE encoding.
//!
//! Each event is `id:` (its number in the turn), `event:` (its name), and `data:` (one line of
//! JSON). The runtime sends events through an [`Emitter`]; the service writes them out.

use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::blocks::Block;

/// What a step is doing (`agent.step.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Route,
    SearchTools,
    Plan,
    CallTool,
    Check,
}

impl StepKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Route => "route",
            Self::SearchTools => "search_tools",
            Self::Plan => "plan",
            Self::CallTool => "call_tool",
            Self::Check => "check",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Running,
    Done,
    Failed,
}

/// Turn totals for the footer ("9 tool calls · Muse Glimmer 30B · 3.8s").
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TurnUsage {
    pub model_calls: u32,
    pub tool_calls: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub ms: u64,
    /// Summed from decimal prices; `None` when a model without a USD price was used.
    pub cost_usd: Option<serde_json::Number>,
}

/// Screen commands the agent may send. A `person` button is never a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    Highlight,
    ScrollTo,
    Open,
    FocusInput,
}

/// One event of a turn.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    TurnStarted {
        turn_id: String,
        conversation_id: String,
        agent: String,
    },
    Step {
        step_id: String,
        kind: StepKind,
        summary: String,
        tool: Option<String>,
        status: StepStatus,
    },
    MessageDelta {
        text: String,
    },
    MessageDone {
        message_id: String,
        text: String,
    },
    Block(Box<Block>),
    MemorySaved {
        memory_id: String,
        text: String,
    },
    UiCommand {
        command: Command,
        target: String,
        note: Option<String>,
    },
    ChangeSetUpdated {
        change_set_id: String,
        version: i32,
        state: String,
    },
    Error {
        code: String,
        message: String,
    },
    TurnDone {
        turn_id: String,
        usage: TurnUsage,
        models: Vec<String>,
    },
}

impl Event {
    pub fn name(&self) -> &'static str {
        match self {
            Self::TurnStarted { .. } => "turn.started",
            Self::Step { .. } => "agent.step",
            Self::MessageDelta { .. } => "message.delta",
            Self::MessageDone { .. } => "message.done",
            Self::Block(_) => "ui.block",
            Self::MemorySaved { .. } => "memory.saved",
            Self::UiCommand { .. } => "ui.command",
            Self::ChangeSetUpdated { .. } => "change_set.updated",
            Self::Error { .. } => "error",
            Self::TurnDone { .. } => "turn.done",
        }
    }

    pub fn data(&self) -> Value {
        match self {
            Self::TurnStarted {
                turn_id,
                conversation_id,
                agent,
            } => json!({"turn_id": turn_id, "conversation_id": conversation_id, "agent": agent}),
            Self::Step {
                step_id,
                kind,
                summary,
                tool,
                status,
            } => {
                let mut v =
                    json!({"step_id": step_id, "kind": kind, "summary": summary, "status": status});
                if let Some(t) = tool {
                    v["tool"] = json!(t);
                }
                v
            }
            Self::MessageDelta { text } => json!({"text": text}),
            Self::MessageDone { message_id, text } => {
                json!({"message_id": message_id, "text": text})
            }
            Self::Block(b) => serde_json::to_value(b).unwrap_or(Value::Null),
            Self::MemorySaved { memory_id, text } => json!({"memory_id": memory_id, "text": text}),
            Self::UiCommand {
                command,
                target,
                note,
            } => {
                let mut v = json!({"command": command, "target": target});
                if let Some(n) = note {
                    v["note"] = json!(n);
                }
                v
            }
            Self::ChangeSetUpdated {
                change_set_id,
                version,
                state,
            } => json!({"change_set_id": change_set_id, "version": version, "state": state}),
            Self::Error { code, message } => json!({"code": code, "message": message}),
            Self::TurnDone {
                turn_id,
                usage,
                models,
            } => json!({"turn_id": turn_id, "usage": usage, "models": models}),
        }
    }
}

/// One SSE frame. `data` is one line: JSON never contains a raw newline.
pub fn encode(id: u64, event: &Event) -> String {
    format!(
        "id: {id}\nevent: {}\ndata: {}\n\n",
        event.name(),
        event.data()
    )
}

/// Where the runtime sends events. Sending never fails the turn: when the person has gone
/// away, events are dropped and [`Emitter::is_closed`] lets the runtime stop early.
#[derive(Debug, Clone)]
pub struct Emitter {
    tx: mpsc::UnboundedSender<Event>,
}

impl Emitter {
    pub fn new(tx: mpsc::UnboundedSender<Event>) -> Self {
        Self { tx }
    }

    /// An emitter and the receiving end, for the service and for tests.
    pub fn channel() -> (Self, mpsc::UnboundedReceiver<Event>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self { tx }, rx)
    }

    pub fn send(&self, e: Event) {
        let _ = self.tx.send(e);
    }

    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }

    /// Points at an element, a block, or a page. Targets are element refs (`blk_1#check_row:x`),
    /// block ids, or paths; anything else is refused so no command can reach a button.
    pub fn command(&self, command: Command, target: &str, note: Option<String>) -> bool {
        let ok = target.starts_with("blk_") || target.starts_with('/');
        if ok {
            self.send(Event::UiCommand {
                command,
                target: target.to_owned(),
                note,
            });
        }
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_follow_the_wire_format() {
        let e = Event::TurnStarted {
            turn_id: "turn_1".into(),
            conversation_id: "conv_1".into(),
            agent: "promotion".into(),
        };
        assert_eq!(
            encode(1, &e),
            "id: 1\nevent: turn.started\ndata: {\"turn_id\":\"turn_1\",\"conversation_id\":\"conv_1\",\"agent\":\"promotion\"}\n\n"
        );
        let e = Event::MessageDelta {
            text: "line one\nline two".into(),
        };
        let frame = encode(7, &e);
        assert_eq!(frame.matches('\n').count(), 4, "{frame}");
        assert!(frame.contains("data: {\"text\":\"line one\\nline two\"}"));
    }

    #[test]
    fn every_event_has_its_documented_name_and_fields() {
        let step = Event::Step {
            step_id: "step_1".into(),
            kind: StepKind::CallTool,
            summary: "search_products".into(),
            tool: Some("search_products".into()),
            status: StepStatus::Done,
        };
        assert_eq!(step.name(), "agent.step");
        assert_eq!(step.data()["kind"], "call_tool");
        assert_eq!(step.data()["status"], "done");
        let done = Event::TurnDone {
            turn_id: "turn_1".into(),
            usage: TurnUsage {
                model_calls: 2,
                tool_calls: 9,
                ms: 3800,
                ..Default::default()
            },
            models: vec!["Muse Glimmer 30B".into()],
        };
        assert_eq!(done.name(), "turn.done");
        assert_eq!(done.data()["usage"]["tool_calls"], 9);
        assert_eq!(done.data()["usage"]["cost_usd"], Value::Null);
        let cs = Event::ChangeSetUpdated {
            change_set_id: "cs_1042".into(),
            version: 2,
            state: "waiting_for_approval".into(),
        };
        assert_eq!(cs.name(), "change_set.updated");
        assert_eq!(
            Event::Error {
                code: "x".into(),
                message: "y".into()
            }
            .name(),
            "error"
        );
    }

    #[tokio::test]
    async fn commands_only_reach_elements_blocks_and_pages() {
        let (emit, mut rx) = Emitter::channel();
        assert!(emit.command(
            Command::Highlight,
            "blk_1#check_row:price_floor",
            Some("2 products".into())
        ));
        assert!(!emit.command(Command::Highlight, "approve", None));
        assert!(emit.command(Command::Open, "/bo/change-sets/cs_1042", None));
        drop(emit);
        let mut names = Vec::new();
        while let Some(e) = rx.recv().await {
            names.push(e.data()["target"].as_str().unwrap_or_default().to_owned());
        }
        assert_eq!(
            names,
            ["blk_1#check_row:price_floor", "/bo/change-sets/cs_1042"]
        );
    }
}
