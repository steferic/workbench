mod handlers;

use crate::app::{Action, AppState};
use anyhow::Result;
use crossterm::event::{self, Event, KeyEvent, MouseButton, MouseEventKind};
use std::time::Duration;
use tokio::sync::mpsc;

/// Internal event type for terminal events
enum TerminalEvent {
    Key(KeyEvent),
    Paste(String),
    MouseDown(u16, u16),
    MouseDrag(u16, u16),
    MouseUp(u16, u16),
    MouseScrollUp,
    MouseScrollDown,
    Resize(u16, u16),
    Tick,
}

pub struct EventHandler {
    action_tx: mpsc::UnboundedSender<Action>,
    action_rx: mpsc::UnboundedReceiver<Action>,
    pty_tx: mpsc::Sender<Action>,
    pty_rx: mpsc::Receiver<Action>,
    terminal_rx: mpsc::UnboundedReceiver<TerminalEvent>,
}

impl EventHandler {
    pub fn new() -> Self {
        const PTY_QUEUE_SIZE: usize = 256;
        let (action_tx, action_rx) = mpsc::unbounded_channel();
        let (pty_tx, pty_rx) = mpsc::channel(PTY_QUEUE_SIZE);
        let (terminal_tx, terminal_rx) = mpsc::unbounded_channel();

        // Spawn dedicated thread for terminal events
        std::thread::spawn(move || {
            let poll_timeout = Duration::from_millis(50);
            loop {
                let event = if event::poll(poll_timeout).unwrap_or(false) {
                    match event::read() {
                        Ok(Event::Key(key)) => TerminalEvent::Key(key),
                        Ok(Event::Mouse(mouse)) => match mouse.kind {
                            MouseEventKind::Down(MouseButton::Left) => {
                                TerminalEvent::MouseDown(mouse.column, mouse.row)
                            }
                            MouseEventKind::Drag(MouseButton::Left) => {
                                TerminalEvent::MouseDrag(mouse.column, mouse.row)
                            }
                            MouseEventKind::Up(MouseButton::Left) => {
                                TerminalEvent::MouseUp(mouse.column, mouse.row)
                            }
                            MouseEventKind::ScrollUp => TerminalEvent::MouseScrollUp,
                            MouseEventKind::ScrollDown => TerminalEvent::MouseScrollDown,
                            _ => TerminalEvent::Tick,
                        },
                        Ok(Event::Resize(w, h)) => TerminalEvent::Resize(w, h),
                        Ok(Event::Paste(data)) => TerminalEvent::Paste(data),
                        _ => TerminalEvent::Tick,
                    }
                } else {
                    TerminalEvent::Tick
                };

                if terminal_tx.send(event).is_err() {
                    break; // Channel closed, exit thread
                }
            }
        });

        Self {
            action_tx,
            action_rx,
            pty_tx,
            pty_rx,
            terminal_rx,
        }
    }

    pub fn action_sender(&self) -> mpsc::UnboundedSender<Action> {
        self.action_tx.clone()
    }

    pub fn pty_sender(&self) -> mpsc::Sender<Action> {
        self.pty_tx.clone()
    }

    /// Try to receive a PTY action without blocking (for batch processing)
    pub fn try_recv_pty_action(&mut self) -> Result<Action, mpsc::error::TryRecvError> {
        self.pty_rx.try_recv()
    }

    pub async fn next(&mut self, state: &AppState) -> Result<Action> {
        // PRIORITY: Always check terminal events first (keyboard input should never be delayed)
        // Use try_recv to check without blocking
        if let Ok(event) = self.terminal_rx.try_recv() {
            return match event {
                TerminalEvent::Key(key) => Ok(self.handle_key_event(key, state)),
                TerminalEvent::Paste(data) => Ok(Action::Paste(data)),
                TerminalEvent::MouseDown(x, y) => Ok(Action::MouseClick(x, y)),
                TerminalEvent::MouseDrag(x, y) => Ok(Action::MouseDrag(x, y)),
                TerminalEvent::MouseUp(x, y) => Ok(Action::MouseUp(x, y)),
                TerminalEvent::MouseScrollUp => Ok(Action::ScrollOutputUp),
                TerminalEvent::MouseScrollDown => Ok(Action::ScrollOutputDown),
                TerminalEvent::Resize(w, h) => Ok(Action::Resize(w, h)),
                TerminalEvent::Tick => Ok(Action::Tick),
            };
        }
        if let Ok(action) = self.pty_rx.try_recv() {
            return Ok(action);
        }
        if let Ok(action) = self.action_rx.try_recv() {
            return Ok(action);
        }

        // Then check action channel (PTY output, etc.)
        tokio::select! {
            biased; // Prefer terminal events when both are ready

            // Terminal events (keyboard, mouse, resize)
            Some(event) = self.terminal_rx.recv() => {
                match event {
                    TerminalEvent::Key(key) => Ok(self.handle_key_event(key, state)),
                    TerminalEvent::Paste(data) => Ok(Action::Paste(data)),
                    TerminalEvent::MouseDown(x, y) => Ok(Action::MouseClick(x, y)),
                    TerminalEvent::MouseDrag(x, y) => Ok(Action::MouseDrag(x, y)),
                    TerminalEvent::MouseUp(x, y) => Ok(Action::MouseUp(x, y)),
                    TerminalEvent::MouseScrollUp => Ok(Action::ScrollOutputUp),
                    TerminalEvent::MouseScrollDown => Ok(Action::ScrollOutputDown),
                    TerminalEvent::Resize(w, h) => Ok(Action::Resize(w, h)),
                    TerminalEvent::Tick => Ok(Action::Tick),
                }
            }
            // PTY output and related actions
            Some(action) = self.pty_rx.recv() => {
                Ok(action)
            }
            // PTY output and other actions
            Some(action) = self.action_rx.recv() => {
                Ok(action)
            }
            else => Ok(Action::Tick)
        }
    }
}

impl Default for EventHandler {
    fn default() -> Self {
        Self::new()
    }
}
