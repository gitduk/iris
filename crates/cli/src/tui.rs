use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use iris_core::io::output::OutputReceiver;
use iris_core::runtime::RuntimeStatus;
use iris_core::types::SensoryEvent;

use crate::event::AppEvent;
use crate::widgets;

/// A chat message with role and content.
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// TUI application state.
pub struct App {
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub cursor: usize,
    pub scroll_offset: u16,
    pub thinking: bool,
    pub anim_frame: usize,
    pub status: RuntimeStatus,
    pub should_exit: bool,
    /// Number of stale replies to skip (from interrupted requests).
    pub skip_replies: usize,
}

impl App {
    fn new() -> Self {
        Self {
            messages: Vec::new(),
            input: String::new(),
            cursor: 0,
            scroll_offset: 0,
            thinking: false,
            anim_frame: 0,
            status: RuntimeStatus::default(),
            should_exit: false,
            skip_replies: 0,
        }
    }

    /// Submit input. If already thinking, replace the last "You" message
    /// (discard the old one) and mark the pending reply as stale.
    fn submit_input(&mut self) -> Option<String> {
        let text = self.input.trim().to_owned();
        if text.is_empty() {
            return None;
        }
        self.input.clear();
        self.cursor = 0;
        self.scroll_offset = 0;

        if self.thinking {
            // Replace: remove the last "You" message, push new one
            if let Some(pos) = self.messages.iter().rposition(|m| m.role == "You") {
                self.messages.remove(pos);
            }
            self.messages.push(ChatMessage { role: "You".into(), content: text.clone() });
            self.skip_replies += 1;
        } else {
            self.messages.push(ChatMessage { role: "You".into(), content: text.clone() });
            self.thinking = true;
        }

        Some(text)
    }

    fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    fn delete_char_before_cursor(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = self.input[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.input.drain(prev..self.cursor);
        self.cursor = prev;
    }

    fn move_cursor_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = self.input[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.cursor = prev;
    }

    fn move_cursor_right(&mut self) {
        if self.cursor >= self.input.len() {
            return;
        }
        let next = self.input[self.cursor..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| self.cursor + i)
            .unwrap_or(self.input.len());
        self.cursor = next;
    }
}

/// Run the TUI event loop. Blocks until the user exits (Ctrl+C).
pub async fn run_app(
    event_tx: mpsc::Sender<SensoryEvent>,
    mut output_rx: OutputReceiver,
    mut status_rx: watch::Receiver<RuntimeStatus>,
    token: CancellationToken,
    startup_notice: Option<String>,
) -> anyhow::Result<()> {
    // Enter raw mode + alternate screen
    terminal::enable_raw_mode()?;
    crossterm::execute!(std::io::stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let stop = Arc::new(AtomicBool::new(false));
    let mut event_rx = crate::event::spawn(stop.clone());

    let mut app = App::new();
    if let Some(content) = startup_notice {
        app.messages.push(ChatMessage {
            role: "Iris".into(),
            content,
        });
    }
    let mut anim_interval = tokio::time::interval(std::time::Duration::from_millis(80));
    anim_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Initial draw
    terminal.draw(|f| widgets::draw(f, &app))?;

    loop {
        if app.should_exit {
            break;
        }
        tokio::select! {
            _ = token.cancelled() => {
                break;
            }
            evt = event_rx.recv() => {
                let Some(evt) = evt else { break };
                match evt {
                    AppEvent::Key(key) => handle_key(&mut app, key, &event_tx).await,
                }
            }
            msg = output_rx.recv() => {
                if let Some(msg) = msg {
                    if app.skip_replies > 0 {
                        // Stale reply from an interrupted request — discard
                        app.skip_replies -= 1;
                    } else {
                        app.messages.push(ChatMessage {
                            role: "Iris".into(),
                            content: msg.content,
                        });
                        app.thinking = false;
                        app.scroll_offset = 0;
                    }
                }
            }
            Ok(()) = status_rx.changed() => {
                app.status = *status_rx.borrow_and_update();
            }
            _ = anim_interval.tick() => {
                if app.thinking {
                    app.anim_frame = app.anim_frame.wrapping_add(1);
                }
            }
        }
        terminal.draw(|f| widgets::draw(f, &app))?;
    }

    // Cleanup
    stop.store(true, Ordering::Relaxed);
    terminal::disable_raw_mode()?;
    crossterm::execute!(std::io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}

async fn handle_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    event_tx: &mpsc::Sender<SensoryEvent>,
) {
    match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
            app.should_exit = true;
        }
        (_, KeyCode::Enter) => {
            if let Some(text) = app.submit_input() {
                let _ = iris_core::io::input::submit_text(event_tx, text).await;
            }
        }
        (_, KeyCode::Backspace) => {
            app.delete_char_before_cursor();
        }
        (_, KeyCode::Left) => {
            app.move_cursor_left();
        }
        (_, KeyCode::Right) => {
            app.move_cursor_right();
        }
        (_, KeyCode::Up) => {
            app.scroll_offset = app.scroll_offset.saturating_add(1);
        }
        (_, KeyCode::Down) => {
            app.scroll_offset = app.scroll_offset.saturating_sub(1);
        }
        (_, KeyCode::Home) => {
            app.cursor = 0;
        }
        (_, KeyCode::End) => {
            app.cursor = app.input.len();
        }
        (_, KeyCode::Char(c)) => {
            app.insert_char(c);
        }
        _ => {}
    }
}
