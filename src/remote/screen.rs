//! An agent's live terminal, for another Mac's Workbench to show.
//!
//! The phone reads conversations from the journals, which is speech without
//! the terminal. Another Workbench wants the terminal itself, so it can show a
//! remote agent exactly as this Mac does and type into it. The screen goes as
//! `vt100`'s own `state_formatted` bytes: the escape codes that rebuild it,
//! colors and cursor included. The other side feeds them to a parser of the
//! same size and draws that parser with the code that draws a local agent.
//!
//! ```text
//! GET /api/screen?agent=ab12   → marks ab12 watched, returns its last frame
//! tick → publish_screens       → rebuilds the frame of each watched agent
//! POST /api/keys {agent, text} → the bytes go to the agent's terminal
//! ```
//!
//! Only watched agents cost anything, and a frame that did not change goes
//! back as a 304 with no body.

use super::{session_for, Shared};
use crate::app::AppState;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A watch lapses when the other Mac stops asking for this long.
const WATCH_LASTS: Duration = Duration::from_secs(5);

/// One agent's screen as escape codes, and the size to replay them at.
#[derive(Debug, Clone, Default)]
pub struct Frame {
    pub rows: u16,
    pub cols: u16,
    pub bytes: Arc<Vec<u8>>,
    pub tag: String,
}

impl Frame {
    pub fn of(screen: &vt100::Screen) -> Frame {
        use std::hash::{Hash, Hasher};
        let bytes = screen.state_formatted();
        let (rows, cols) = screen.size();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (rows, cols, &bytes).hash(&mut hasher);
        Frame {
            rows,
            cols,
            bytes: Arc::new(bytes),
            tag: format!("\"{:x}\"", hasher.finish()),
        }
    }

    /// The screen rebuilt in a parser of its own size.
    pub fn parser(&self) -> vt100::Parser {
        let mut parser = vt100::Parser::new(self.rows.max(1), self.cols.max(1), 0);
        parser.process(&self.bytes);
        parser
    }
}

/// Keys travel as hex: a command body is trimmed, and Enter is whitespace.
pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn from_hex(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Rebuild the frame of every agent another Mac is watching. On the tick.
pub fn publish_screens(state: &AppState, shared: &Shared) {
    let watched: Vec<String> = match shared.lock() {
        Ok(mut snapshot) => {
            snapshot
                .screen_watch
                .retain(|_, at| at.elapsed() < WATCH_LASTS);
            let watched: Vec<String> = snapshot.screen_watch.keys().cloned().collect();
            snapshot.screens.retain(|id, _| watched.contains(id));
            watched
        }
        Err(_) => return,
    };
    if watched.is_empty() {
        return;
    }
    // Built outside the lock: a screen of a few thousand cells is quick, but
    // a phone reading state should never wait on it.
    let frames: Vec<(String, Option<Frame>)> = watched
        .into_iter()
        .map(|id| {
            let frame = session_for(state, &id)
                .and_then(|session| state.system.output_buffers.get(&session))
                .map(|parser| Frame::of(parser.screen()));
            (id, frame)
        })
        .collect();
    if let Ok(mut snapshot) = shared.lock() {
        for (id, frame) in frames {
            match frame {
                Some(frame) => {
                    if snapshot.screens.get(&id).is_none_or(|f| f.tag != frame.tag) {
                        snapshot.screens.insert(id, frame);
                    }
                }
                None => {
                    snapshot.screens.remove(&id);
                }
            }
        }
    }
}

/// Mark an agent watched and return its frame, if one is built yet.
pub fn watch(shared: &Shared, agent: &str) -> Result<Option<Frame>, ()> {
    let mut snapshot = shared.lock().map_err(|_| ())?;
    if !snapshot.agents.iter().any(|a| a.id == agent) {
        return Err(());
    }
    snapshot
        .screen_watch
        .insert(agent.to_string(), Instant::now());
    Ok(snapshot.screens.get(agent).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_rebuilds_the_same_screen_with_its_colors_and_cursor() {
        let mut source = vt100::Parser::new(6, 30, 0);
        source.process(b"plain \x1b[31mred\x1b[0m\r\n\x1b[1mbold\x1b[0m line\r\n> typing");
        let frame = Frame::of(source.screen());
        let copy = frame.parser();
        let (a, b) = (source.screen(), copy.screen());
        assert_eq!(b.size(), (6, 30));
        assert_eq!(a.contents(), b.contents());
        assert_eq!(a.cursor_position(), b.cursor_position());
        assert_eq!(
            b.cell(0, 6).unwrap().fgcolor(),
            vt100::Color::Idx(1),
            "red stays red"
        );
        assert!(b.cell(1, 0).unwrap().bold());

        let same = Frame::of(source.screen());
        assert_eq!(same.tag, frame.tag, "an unchanged screen keeps its tag");
        source.process(b"!");
        assert_ne!(Frame::of(source.screen()).tag, frame.tag);
    }

    #[test]
    fn keys_survive_the_trip_as_hex_whitespace_included() {
        let keys = "\r\t é\x1b[A".as_bytes();
        assert_eq!(from_hex(&to_hex(keys)).as_deref(), Some(keys));
        assert_eq!(from_hex("0"), None);
        assert_eq!(from_hex("zz"), None);
    }

    #[test]
    fn only_a_watched_agent_gets_a_frame_and_it_lapses_when_nobody_asks() {
        use crate::models::{AgentType, Session, Workspace};
        let mut state = AppState::default();
        let workspace = Workspace::new("app".into(), "/tmp".into());
        let session = Session::new(workspace.id, AgentType::Claude, false);
        let (id, short) = (session.id, session.short_id());
        state.add_workspace(workspace);
        state.add_session(session);
        let mut parser = vt100::Parser::new(10, 40, 0);
        parser.process(b"> hello from the agent");
        state.system.output_buffers.insert(id, parser);

        let shared: Shared = Default::default();
        shared.lock().unwrap().agents.push(super::super::AgentView {
            id: short.clone(),
            ..Default::default()
        });
        publish_screens(&state, &shared);
        assert!(
            shared.lock().unwrap().screens.is_empty(),
            "nobody watches yet"
        );

        assert!(watch(&shared, "nope").is_err());
        assert!(watch(&shared, &short).unwrap().is_none(), "not built yet");
        publish_screens(&state, &shared);
        let frame = watch(&shared, &short).unwrap().unwrap();
        assert!(frame
            .parser()
            .screen()
            .contents()
            .contains("hello from the agent"));

        shared
            .lock()
            .unwrap()
            .screen_watch
            .insert(short.clone(), Instant::now() - WATCH_LASTS);
        publish_screens(&state, &shared);
        assert!(
            shared.lock().unwrap().screens.is_empty(),
            "the watch lapsed"
        );
    }
}
