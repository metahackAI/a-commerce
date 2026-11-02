//! The reply as the model writes it (docs/api/chat.md §2): `message.delta` events carry the
//! reply's text while the model is still writing, so the person reads it as it comes.
//!
//! The reply is asked for as JSON (`{"text": "…"}`, see the runtime's reply schema), so what
//! arrives is the start of a JSON object; [`visible_text`] reads the `text` string out of it as
//! far as it has come. The reply checks run on the whole reply afterwards: when one turns it
//! down, [`ReplyStream::reset`] sends `message.reset` and the next attempt streams again, and
//! `message.done` always carries the reply that stands.

use std::sync::Mutex;

use crate::stream::{Emitter, Event};

/// The `text` of a JSON object that may still be arriving (`{"text": "Here are a fe`), or the
/// raw text when the model wrote plain text. Escapes are decoded; one cut in half is left out
/// until the rest of it arrives.
pub fn visible_text(raw: &str) -> String {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with('{') {
        // A model that ignored the schema and wrote plain text.
        return trimmed.to_owned();
    }
    let Some(key) = trimmed.find("\"text\"") else {
        return String::new();
    };
    let rest = trimmed[key + "\"text\"".len()..].trim_start();
    let Some(rest) = rest.strip_prefix(':') else {
        return String::new();
    };
    let Some(body) = rest.trim_start().strip_prefix('"') else {
        return String::new();
    };
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => {
                let Some(e) = chars.next() else { break };
                match e {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => {}
                    'b' | 'f' => {}
                    'u' => {
                        let hex: String = chars.by_ref().take(4).collect();
                        if hex.len() < 4 {
                            break;
                        }
                        // Half of a surrogate pair is left to the final text.
                        if let Some(ch) =
                            u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
                        {
                            out.push(ch);
                        }
                    }
                    other => out.push(other),
                }
            }
            c => out.push(c),
        }
    }
    out
}

#[derive(Debug, Default)]
struct State {
    raw: String,
    shown: String,
}

/// Streams one reply attempt at a time to the person.
#[derive(Debug)]
pub struct ReplyStream {
    emit: Emitter,
    state: Mutex<State>,
}

impl ReplyStream {
    pub fn new(emit: Emitter) -> Self {
        Self {
            emit,
            state: Mutex::new(State::default()),
        }
    }

    /// Takes the next piece the model wrote and sends what it adds to the visible text.
    pub fn push(&self, delta: &str) {
        let Ok(mut st) = self.state.lock() else {
            return;
        };
        st.raw.push_str(delta);
        let text = visible_text(&st.raw);
        // Only growth is sent: the visible text of a JSON object only ever grows.
        if text.len() > st.shown.len() && text.starts_with(st.shown.as_str()) {
            let added = text[st.shown.len()..].to_owned();
            st.shown = text;
            self.emit.send(Event::MessageDelta { text: added });
        }
    }

    /// What the person has seen of the current attempt.
    pub fn shown(&self) -> String {
        self.state
            .lock()
            .map(|st| st.shown.clone())
            .unwrap_or_default()
    }

    /// Drops the current attempt; tells the person when they saw any of it.
    pub fn reset(&self, reason: &str) {
        let Ok(mut st) = self.state.lock() else {
            return;
        };
        if !st.shown.is_empty() {
            self.emit.send(Event::MessageReset {
                reason: reason.to_owned(),
            });
        }
        *st = State::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_text_of_an_object_still_arriving() {
        assert_eq!(visible_text(""), "");
        assert_eq!(visible_text("{"), "");
        assert_eq!(visible_text("{\"te"), "");
        assert_eq!(visible_text("{\"text\": \""), "");
        assert_eq!(visible_text("{\"text\": \"Here are"), "Here are");
        assert_eq!(
            visible_text("{\"text\":\"Two \\\"light\\\" ones.\"}"),
            "Two \"light\" ones."
        );
        assert_eq!(visible_text("{\"text\": \"a\\nb"), "a\nb");
        // An escape cut in half waits for the rest.
        assert_eq!(visible_text("{\"text\": \"caf\\u00"), "caf");
        assert_eq!(visible_text("{\"text\": \"caf\\u00e9"), "café");
        assert_eq!(visible_text("{\"text\": \"ends\\"), "ends");
        // Plain text is shown as it is.
        assert_eq!(visible_text("Here are two."), "Here are two.");
    }

    #[test]
    fn sends_only_what_each_piece_adds_and_resets_between_attempts() {
        let (emit, mut rx) = Emitter::channel();
        let s = ReplyStream::new(emit);
        for piece in ["{\"te", "xt\": \"Here", " are", " two", ".\"}"] {
            s.push(piece);
        }
        assert_eq!(s.shown(), "Here are two.");
        s.reset("numbers not in the results");
        s.push("{\"text\": \"Again\"}");
        s.reset("unused");
        let mut got = Vec::new();
        while let Ok(e) = rx.try_recv() {
            got.push((e.name(), e.data()));
        }
        let names: Vec<&str> = got.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names,
            [
                "message.delta",
                "message.delta",
                "message.delta",
                "message.delta",
                "message.reset",
                "message.delta",
                "message.reset"
            ]
        );
        assert_eq!(got[0].1["text"], "Here");
        assert_eq!(got[3].1["text"], ".");
        assert_eq!(got[5].1["text"], "Again");
    }
}
