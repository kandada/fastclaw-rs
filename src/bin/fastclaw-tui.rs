// Copyright (c) 2024-2026 xiefujin <490021684@qq.com>
// Licensed under GNU GPLv3, see LICENSE file for full license terms.

//! FastClaw TUI — an interactive terminal for development/testing.
//!
//! Run: `cargo run --bin fastclaw-tui [--workspace <path>] [--agent <id>] [--session <id>] [--workdir <path>]`
//!
//! Slash commands: /new /clear /session <id> /session_list /agent [<id>]
//!                 /skill list /workdir <path> /help /quit
//!
//! `/workdir <path>` and `--workdir <path>` persist the working directory to
//! `settings.json` (and bind it to the current session), so the project dir
//! survives restarts.

use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use fastclaw::{FastClaw, FastClawConfig};
use ratatui::{
    backend::CrosstermBackend,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line as RLine, Span, Text},
    widgets::{Block, Borders, Paragraph, Widget, Wrap},
    Terminal,
};

/// Slash commands shown by `/help`. First column is padded to align the
/// descriptions.
const COMMANDS: &[(&str, &str)] = &[
    ("/new", "start a new session"),
    ("/clear", "clear this session's messages"),
    ("/session <id>", "switch to a session"),
    ("/session_list", "list all sessions"),
    ("/agent", "list available agents"),
    ("/agent <id>", "switch to an agent"),
    ("/skill list", "list available skills"),
    ("/workdir <path>", "set + persist working directory"),
    ("/help", "show this help"),
    ("/quit", "exit fastclaw"),
];

/// One rendered line in the conversation.
#[derive(Clone, Copy)]
enum Kind {
    User,
    Assistant,
    Thinking,
    Tool,
    Info,
    Error,
}

#[derive(Clone)]
struct Line {
    kind: Kind,
    text: String,
}

/// Maximum number of options shown at once inside the interactive picker.
const PICKER_VISIBLE: usize = 12;

/// A modal, keyboard-navigable option list (e.g. the `/agent` switcher).
#[derive(Clone)]
struct Picker {
    title: String,
    items: Vec<String>,
    selected: usize,
    scroll: usize,
}

impl Picker {
    fn open(title: impl Into<String>, items: Vec<String>, current: Option<&str>) -> Self {
        let selected = current
            .and_then(|c| items.iter().position(|item| item == c))
            .unwrap_or(0);
        let mut picker = Self {
            title: title.into(),
            items,
            selected,
            scroll: 0,
        };
        picker.clamp_scroll();
        picker
    }

    /// Move the highlight one row up, keeping it inside the visible window.
    fn prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.clamp_scroll();
        }
    }

    /// Move the highlight one row down, keeping it inside the visible window.
    fn next(&mut self) {
        if self.selected + 1 < self.items.len() {
            self.selected += 1;
            self.clamp_scroll();
        }
    }

    fn page_up(&mut self) {
        self.selected = self.selected.saturating_sub(PICKER_VISIBLE);
        self.clamp_scroll();
    }

    fn page_down(&mut self) {
        self.selected = (self.selected + PICKER_VISIBLE).min(self.items.len() - 1);
        self.clamp_scroll();
    }

    fn clamp_scroll(&mut self) {
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + PICKER_VISIBLE {
            self.scroll = self.selected + 1 - PICKER_VISIBLE;
        }
    }

    fn selected(&self) -> Option<String> {
        self.items.get(self.selected).cloned()
    }
}

/// `cursor` is always a *character* index (0 = before the first char), which
/// is what navigation and rendering expect. `String` mutation methods need a
/// byte index, so every edit goes through `byte_index()` to stay safe with
/// multi-byte (CJK / emoji) input.
struct UiState {
    lines: Vec<Line>,
    input: String,
    cursor: usize,
    status: String,
    /// Interactive picker (e.g. agent switcher) currently open, if any.
    menu: Option<Picker>,
    /// Conversation scroll state. `conv_scroll` is the number of wrapped rows
    /// hidden above the view; when `conv_pinned` the view follows the newest
    /// content. `conv_max` / `conv_inner` are refreshed every frame.
    conv_scroll: usize,
    conv_pinned: bool,
    conv_max: usize,
    conv_inner: usize,
    /// Set whenever the visible UI changes so the loop only redraws on demand
    /// instead of spamming escape sequences to the terminal every 50 ms.
    dirty: bool,
    /// Advances while a request is running, driving the spinner / blinking
    /// caret animation.
    anim_tick: u64,
}

impl UiState {
    /// Byte offset of the cursor (character index -> byte index).
    fn byte_index(&self) -> usize {
        self.input
            .char_indices()
            .nth(self.cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len())
    }

    /// Total character count of the input.
    fn char_len(&self) -> usize {
        self.input.chars().count()
    }

    fn insert_char(&mut self, c: char) {
        let byte = self.byte_index();
        self.input.insert(byte, c);
        self.cursor += 1;
        self.dirty = true;
    }

    /// Delete the character before the cursor (backspace).
    fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let byte = self.byte_index();
            self.input.remove(byte);
            self.dirty = true;
        }
    }

    /// Delete the character under the cursor (delete key).
    fn delete(&mut self) {
        if self.cursor < self.char_len() {
            let byte = self.byte_index();
            self.input.remove(byte);
            self.dirty = true;
        }
    }

    fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
        self.dirty = true;
    }

    fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.char_len());
        self.dirty = true;
    }

    fn home(&mut self) {
        self.cursor = 0;
        self.dirty = true;
    }

    fn end(&mut self) {
        self.cursor = self.char_len();
        self.dirty = true;
    }

    /// Scroll the conversation up by `step` rows (towards older messages),
    /// unpinning from the bottom.
    fn conv_scroll_up(&mut self, step: usize) {
        self.conv_pinned = false;
        self.conv_scroll = self.conv_scroll.saturating_sub(step);
        self.dirty = true;
    }

    /// Scroll the conversation down by `step` rows (towards newer messages);
    /// reaching the newest content re-pins the view.
    fn conv_scroll_down(&mut self, step: usize) {
        self.conv_pinned = false;
        let target = self.conv_scroll.saturating_add(step);
        self.conv_scroll = target.min(self.conv_max);
        if self.conv_scroll >= self.conv_max {
            self.conv_pinned = true;
        }
        self.dirty = true;
    }

    /// Jump to the top of the conversation (oldest messages).
    fn conv_scroll_top(&mut self) {
        self.conv_pinned = false;
        self.conv_scroll = 0;
        self.dirty = true;
    }

    /// Re-pin to the newest content.
    fn conv_pin(&mut self) {
        self.conv_pinned = true;
        self.dirty = true;
    }
}

fn push(kind: Kind, text: String, state: &Mutex<UiState>) {
    if let Ok(mut s) = state.lock() {
        s.lines.push(Line { kind, text });
        s.dirty = true;
    }
}

fn set_status(text: String, state: &Mutex<UiState>) {
    if let Ok(mut s) = state.lock() {
        s.status = text;
        s.dirty = true;
    }
}

#[derive(Clone)]
struct Ctx {
    claw: Arc<FastClaw>,
    session_id: String,
    agent_id: String,
    workdir: PathBuf,
    state: Arc<Mutex<UiState>>,
}

async fn submit(ctx: Ctx, text: String) {
    let text = text.trim().to_string();
    if text.is_empty() {
        return;
    }

    push(Kind::User, text.clone(), &ctx.state);
    set_status("generating…".to_string(), &ctx.state);

    let claw = ctx.claw.clone();
    let sid = ctx.session_id.clone();
    let state = ctx.state.clone();
    let workdir = ctx.workdir.clone();

    tokio::spawn(async move {
        let _ = claw.bind_work_dir(&sid, &workdir);
        let _ = claw.chat(&sid, &text).await;

        match claw.stream_events(&sid) {
            Ok(mut stream) => {
                use futures::StreamExt;
                let mut thinking_open = false;
                while let Some(ev) = stream.next().await {
                    match ev.type_.as_str() {
                        "stream.thinking" => {
                            if let Some(d) = ev.payload.get("delta").and_then(|v| v.as_str()) {
                                if !thinking_open {
                                    thinking_open = true;
                                    push(Kind::Thinking, String::new(), &state);
                                }
                                if let Ok(mut s) = state.lock() {
                                    if let Some(last) = s.lines.last_mut() {
                                        if matches!(last.kind, Kind::Thinking) {
                                            last.text.push_str(d);
                                            s.dirty = true;
                                        }
                                    }
                                }
                            }
                        }
                        "stream.chunk" => {
                            if let Some(d) = ev.payload.get("delta").and_then(|v| v.as_str()) {
                                thinking_open = false;
                                if let Ok(mut s) = state.lock() {
                                    if let Some(last) = s.lines.last_mut() {
                                        if matches!(last.kind, Kind::Assistant) {
                                            last.text.push_str(d);
                                            s.dirty = true;
                                            continue;
                                        }
                                    }
                                }
                                push(Kind::Assistant, d.to_string(), &state);
                            }
                        }
                        "stream.tool_result" => {
                            thinking_open = false;
                            let name = ev
                                .payload
                                .get("tool_name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let result = ev
                                .payload
                                .get("result")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            push(
                                Kind::Tool,
                                format!(
                                    "[{name}] {}",
                                    result.chars().take(400).collect::<String>()
                                ),
                                &state,
                            );
                        }
                        "stream.fragment" => {
                            thinking_open = false;
                            let names: Vec<&str> = ev
                                .payload
                                .get("tool_calls")
                                .and_then(|v| v.as_array())
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|tc| {
                                            tc.get("function")
                                                .and_then(|f| f.get("name"))
                                                .and_then(|n| n.as_str())
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();
                            push(
                                Kind::Tool,
                                format!("[Executing tool: {}]", names.join(", ")),
                                &state,
                            );
                        }
                        "stream.end" => break,
                        "stream.error" => {
                            let e = ev
                                .payload
                                .get("error")
                                .and_then(|v| v.as_str())
                                .unwrap_or("error");
                            push(Kind::Error, e.to_string(), &state);
                            break;
                        }
                        _ => {}
                    }
                }
            }
            Err(e) => {
                push(
                    Kind::Error,
                    format!("Cannot open event stream: {e}"),
                    &state,
                );
            }
        }

        // Always leave the "generating…" state, even if the stream closed
        // without a terminal event.
        set_status("ready".to_string(), &state);
    });
}

fn handle_command(ctx: &mut Ctx, text: &str) -> bool {
    let text = text.trim();
    match text {
        "/quit" => return true,
        "/new" => {
            ctx.session_id = ctx.claw.new_session(Some(&ctx.agent_id));
            push(
                Kind::Info,
                format!("New session: {}", ctx.session_id),
                &ctx.state,
            );
        }
        "/clear" => {
            if let Ok(mut s) = ctx.state.lock() {
                s.lines.clear();
                s.dirty = true;
            }
            ctx.claw.clear_messages(&ctx.session_id);
        }
        "/session_list" => {
            let sessions = ctx.claw.list_sessions();
            if sessions.is_empty() {
                push(Kind::Info, "No sessions".to_string(), &ctx.state);
            }
            // One line per session (keeps rendering fast + scannable).
            for s in sessions {
                let sid = s.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
                let mark = if sid == ctx.session_id {
                    " <-- current"
                } else {
                    ""
                };
                push(Kind::Info, format!("  - {sid}{mark}"), &ctx.state);
            }
        }
        "/skill list" | "/skills" => {
            let skills = ctx.claw.list_skills();
            let info = skills
                .iter()
                .map(|s| format!("- {}: {}", s.name, s.description))
                .collect::<Vec<_>>()
                .join("\n");
            push(Kind::Info, info, &ctx.state);
        }
        "/agent" => {
            let agents = ctx.claw.list_agents();
            if agents.is_empty() {
                push(Kind::Info, "No agents".to_string(), &ctx.state);
            } else if let Ok(mut s) = ctx.state.lock() {
                s.menu = Some(Picker::open(
                    "Select agent",
                    agents,
                    Some(ctx.agent_id.as_str()),
                ));
                s.dirty = true;
            }
        }
        "/help" => {
            let mut help = String::from("Commands:");
            for (cmd, desc) in COMMANDS {
                help.push_str(&format!("\n  {cmd:<20} {desc}"));
            }
            push(Kind::Info, help, &ctx.state);
        }
        _ => {
            if let Some(id) = text.strip_prefix("/session ") {
                let id = id.trim();
                if ctx.claw.get_session(id).is_some() {
                    ctx.session_id = id.to_string();
                    push(Kind::Info, format!("Switched to {id}"), &ctx.state);
                } else {
                    push(Kind::Error, format!("Session not found: {id}"), &ctx.state);
                }
            } else if let Some(id) = text.strip_prefix("/agent ") {
                let id = id.trim();
                if ctx.claw.list_agents().iter().any(|a| a == id) {
                    ctx.agent_id = id.to_string();
                    push(
                        Kind::Info,
                        format!("Agent set to {}", ctx.agent_id),
                        &ctx.state,
                    );
                } else {
                    push(Kind::Error, format!("Agent not found: {id}"), &ctx.state);
                }
            } else if let Some(p) = text.strip_prefix("/workdir ") {
                let p = PathBuf::from(p.trim());
                ctx.workdir = p.clone();
                // Persist as the global default so the switch survives restart.
                match ctx.claw.set_default_work_dir(&p) {
                    Ok(()) => push(
                        Kind::Info,
                        format!("Working dir: {}", p.display()),
                        &ctx.state,
                    ),
                    Err(e) => push(
                        Kind::Error,
                        format!("Failed to set working dir: {e}"),
                        &ctx.state,
                    ),
                }
            }
        }
    }
    false
}

/// Apply an agent switch picked from the interactive `/agent` menu.
fn switch_agent(ctx: &mut Ctx, agent_id: &str) {
    ctx.agent_id = agent_id.to_string();
    push(Kind::Info, format!("Agent set to {agent_id}"), &ctx.state);
}

/// Best-effort terminal restore. Called on every exit path (normal, error, and
/// panic) so raw mode / the alternate screen can never leak and leave the
/// terminal unusable with Ctrl+C disabled.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        crossterm::cursor::Show,
        LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture,
        crossterm::event::DisableBracketedPaste
    );
}

fn main() -> fastclaw::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut workspace: Option<PathBuf> = None;
    let mut agent: Option<String> = None;
    let mut session: Option<String> = None;
    let mut workdir_flag: Option<PathBuf> = None;
    let mut i = 1;
    while i < args.len() {
        let flag = args[i].as_str();
        let val = args.get(i + 1).cloned();
        match (flag, val) {
            ("--workspace", Some(v)) => {
                workspace = Some(PathBuf::from(v));
                i += 1;
            }
            ("--agent", Some(v)) => {
                agent = Some(v);
                i += 1;
            }
            ("--session", Some(v)) => {
                session = Some(v);
                i += 1;
            }
            ("--workdir", Some(v)) => {
                workdir_flag = Some(PathBuf::from(v));
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    let claw = Arc::new(
        FastClaw::builder(FastClawConfig {
            workspace_root: workspace,
            default_agent_id: agent.clone(),
            ..FastClawConfig::default()
        })
        .build(),
    );

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let _guard = rt.enter();

    // If anything below panics after raw mode / the alternate screen are
    // enabled, the terminal must still be restored — otherwise the user is
    // left with a broken terminal where Ctrl+C does nothing.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));

    let result = rt.block_on(async {
        claw.start().await?;
        let default_agent = claw.get_settings().default_agent_id.clone();
        let agent_id = agent.unwrap_or(default_agent);
        let session_id = session.unwrap_or_else(|| claw.new_session(Some(&agent_id)));
        // `--workdir` overrides the persisted default (and is itself persisted
        // via `set_default_work_dir`, so the project dir survives restart).
        let mut workdir = claw.default_work_dir();
        if let Some(w) = &workdir_flag {
            let _ = claw.set_default_work_dir(w);
            workdir = claw.default_work_dir();
        }

        let state = Arc::new(Mutex::new(UiState {
            lines: vec![Line {
                kind: Kind::Info,
                text:
                    "FastClaw — /new /clear /session_list /agent /skill list /workdir /help /quit"
                        .to_string(),
            }],
            input: String::new(),
            cursor: 0,
            status: "ready".to_string(),
            menu: None,
            conv_scroll: 0,
            conv_pinned: true,
            conv_max: 0,
            conv_inner: 0,
            dirty: true,
            anim_tick: 0,
        }));

        let mut ctx = Ctx {
            claw,
            session_id,
            agent_id,
            workdir,
            state,
        };

        // Everything from raw mode on is wrapped so that the terminal is
        // always restored, even if the event loop or setup errors out.
        let outcome = async {
            enable_raw_mode()?;
            let mut stdout = io::stdout();
            execute!(
                stdout,
                EnterAlternateScreen,
                crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
                crossterm::cursor::MoveTo(0, 0),
                crossterm::event::EnableBracketedPaste,
                crossterm::event::EnableMouseCapture
            )?;
            let backend = CrosstermBackend::new(stdout);
            let mut terminal = Terminal::new(backend)?;
            event_loop(&mut terminal, &mut ctx).await
        }
        .await;

        restore_terminal();
        // Don't let a stuck in-flight request block quitting; give the
        // shutdown a hard deadline and exit regardless.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(3), ctx.claw.stop()).await;
        outcome
    });

    result
}

/// Main TUI loop. Errors here are handled by the caller, which always restores
/// the terminal before returning.
async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ctx: &mut Ctx,
) -> fastclaw::Result<()> {
    let mut run = true;
    let mut last_tick = std::time::Instant::now();
    while run {
        // Redraw only when something actually changed; when idle the terminal
        // receives no output at all, instead of a stream of escape codes.
        let should_draw = match ctx.state.lock() {
            Ok(mut s) => {
                if s.dirty {
                    s.dirty = false;
                    true
                } else {
                    false
                }
            }
            Err(_) => false,
        };
        if should_draw {
            terminal.draw(|f| render(f, ctx))?;
        }

        // Drive the spinner / blinking caret while a request is running.
        let animating = ctx
            .state
            .lock()
            .map(|s| s.status != "ready")
            .unwrap_or(false);
        if animating && last_tick.elapsed() >= std::time::Duration::from_millis(100) {
            last_tick = std::time::Instant::now();
            if let Ok(mut s) = ctx.state.lock() {
                s.anim_tick = s.anim_tick.wrapping_add(1);
                s.dirty = true;
            }
        }

        if event::poll(std::time::Duration::from_millis(50))? {
            match event::read()? {
                Event::Resize(_, _) => {
                    if let Ok(mut s) = ctx.state.lock() {
                        s.dirty = true;
                    }
                }
                // Committed IME text (e.g. Chinese) and clipboard pastes often
                // arrive as a single paste event rather than per-key events.
                Event::Paste(text) => {
                    let mut s = match ctx.state.lock() {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    for c in text.chars().filter(|c| !c.is_control()) {
                        s.insert_char(c);
                    }
                }
                // Mouse wheel scrolls the conversation (or the picker while one
                // is open).
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        if let Ok(mut s) = ctx.state.lock() {
                            if let Some(m) = s.menu.as_mut() {
                                m.prev();
                                s.dirty = true;
                            } else {
                                s.conv_scroll_up(3);
                            }
                        }
                    }
                    MouseEventKind::ScrollDown => {
                        if let Ok(mut s) = ctx.state.lock() {
                            if let Some(m) = s.menu.as_mut() {
                                m.next();
                                s.dirty = true;
                            } else {
                                s.conv_scroll_down(3);
                            }
                        }
                    }
                    _ => {}
                },
                Event::Key(key) => {
                    if key.kind == KeyEventKind::Release {
                        continue;
                    }
                    // While the interactive picker is open it is modal: keys
                    // navigate it instead of editing the input box.
                    let menu_open = ctx.state.lock().map(|s| s.menu.is_some()).unwrap_or(false);
                    if menu_open {
                        match key.code {
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                run = false;
                            }
                            KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                terminal.clear()?;
                                if let Ok(mut s) = ctx.state.lock() {
                                    s.dirty = true;
                                }
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                if let Ok(mut s) = ctx.state.lock() {
                                    if let Some(m) = s.menu.as_mut() {
                                        m.prev();
                                        s.dirty = true;
                                    }
                                }
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                if let Ok(mut s) = ctx.state.lock() {
                                    if let Some(m) = s.menu.as_mut() {
                                        m.next();
                                        s.dirty = true;
                                    }
                                }
                            }
                            KeyCode::PageUp => {
                                if let Ok(mut s) = ctx.state.lock() {
                                    if let Some(m) = s.menu.as_mut() {
                                        m.page_up();
                                        s.dirty = true;
                                    }
                                }
                            }
                            KeyCode::PageDown => {
                                if let Ok(mut s) = ctx.state.lock() {
                                    if let Some(m) = s.menu.as_mut() {
                                        m.page_down();
                                        s.dirty = true;
                                    }
                                }
                            }
                            KeyCode::Enter => {
                                let picked = ctx.state.lock().ok().and_then(|mut s| {
                                    s.dirty = true;
                                    s.menu.take().and_then(|m| m.selected())
                                });
                                if let Some(agent) = picked {
                                    switch_agent(ctx, &agent);
                                }
                            }
                            KeyCode::Esc | KeyCode::Char('q') => {
                                if let Ok(mut s) = ctx.state.lock() {
                                    s.menu = None;
                                    s.dirty = true;
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }
                    match key.code {
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            run = false;
                        }
                        // Force a full repaint if the display ever gets visually
                        // corrupted.
                        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            terminal.clear()?;
                            if let Ok(mut s) = ctx.state.lock() {
                                s.dirty = true;
                            }
                        }
                        KeyCode::Esc => run = false,
                        KeyCode::Char(c) => {
                            if let Ok(mut s) = ctx.state.lock() {
                                s.insert_char(c);
                            }
                        }
                        KeyCode::Backspace => {
                            if let Ok(mut s) = ctx.state.lock() {
                                s.backspace();
                            }
                        }
                        KeyCode::Delete => {
                            if let Ok(mut s) = ctx.state.lock() {
                                s.delete();
                            }
                        }
                        KeyCode::Left => {
                            if let Ok(mut s) = ctx.state.lock() {
                                s.move_left();
                            }
                        }
                        KeyCode::Right => {
                            if let Ok(mut s) = ctx.state.lock() {
                                s.move_right();
                            }
                        }
                        KeyCode::Home => {
                            if let Ok(mut s) = ctx.state.lock() {
                                if s.input.is_empty() {
                                    s.conv_scroll_top();
                                } else {
                                    s.home();
                                }
                            }
                        }
                        KeyCode::End => {
                            if let Ok(mut s) = ctx.state.lock() {
                                if s.input.is_empty() {
                                    s.conv_pin();
                                } else {
                                    s.end();
                                }
                            }
                        }
                        KeyCode::PageUp => {
                            if let Ok(mut s) = ctx.state.lock() {
                                let page = s.conv_inner.max(1);
                                s.conv_scroll_up(page);
                            }
                        }
                        KeyCode::PageDown => {
                            if let Ok(mut s) = ctx.state.lock() {
                                let page = s.conv_inner.max(1);
                                s.conv_scroll_down(page);
                            }
                        }
                        KeyCode::Enter => {
                            let (text, busy) = {
                                let mut s = match ctx.state.lock() {
                                    Ok(s) => s,
                                    Err(_) => continue,
                                };
                                let busy = s.status != "ready";
                                let t = s.input.clone();
                                s.input.clear();
                                s.cursor = 0;
                                s.dirty = true;
                                (t, busy)
                            };
                            // One request at a time: concurrent streaming would
                            // interleave into the same conversation and corrupt
                            // the display. Keep the input (don't clear it) and
                            // let the user know why it wasn't sent.
                            if busy && !text.trim().is_empty() && !text.starts_with('/') {
                                if let Ok(mut s) = ctx.state.lock() {
                                    s.input = text.clone();
                                    s.cursor = s.char_len();
                                    s.dirty = true;
                                }
                                push(
                                    Kind::Info,
                                    "Still generating… wait for it to finish".to_string(),
                                    &ctx.state,
                                );
                                continue;
                            }
                            if handle_command(ctx, &text) {
                                run = false;
                            } else if !text.trim().is_empty() && !text.starts_with('/') {
                                if let Ok(mut s) = ctx.state.lock() {
                                    s.conv_pin();
                                }
                                submit(ctx.clone(), text).await;
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// Spinner frames shown while a request is running.
const SPINNER: [char; 4] = ['|', '/', '-', '\\'];

fn render(f: &mut ratatui::Frame, ctx: &Ctx) {
    let (lines, input, cursor, status, anim_tick, menu, conv_scroll, conv_pinned) =
        match ctx.state.lock() {
            Ok(s) => (
                s.lines.clone(),
                s.input.clone(),
                s.cursor,
                s.status.clone(),
                s.anim_tick,
                s.menu.clone(),
                s.conv_scroll,
                s.conv_pinned,
            ),
            // Never panic from a poisoned lock: degrade to an empty frame
            // instead of breaking the terminal mid-render.
            Err(_) => (
                Vec::new(),
                String::new(),
                0,
                "⚠ internal state error".into(),
                0,
                None,
                0,
                true,
            ),
        };
    let generating = status != "ready";

    // Layout, top to bottom:
    //   conversation — fills all remaining space, scrolls to the bottom
    //   input bar    — a FIXED-height box pinned to the very bottom. Its height
    //                  never changes, so the conversation above is stable while
    //                  typing; the input content scrolls inside the box to keep
    //                  the caret visible instead of growing the box upward.
    const INPUT_HEIGHT_LINES: u16 = 3;
    let input_width = f.area().width.saturating_sub(2).max(1);

    // Render the caret as a reversed block inside the text so it stays at the
    // correct position however the input wraps.
    let chars: Vec<char> = input.chars().collect();
    let caret_idx = cursor.min(chars.len());
    let before: String = chars[..caret_idx].iter().collect();
    let caret_ch = chars.get(caret_idx).copied().unwrap_or(' ');
    let after: String = chars
        .get(caret_idx + 1..)
        .map(|s| s.iter().collect())
        .unwrap_or_default();
    let input_text = Text::from(vec![RLine::from(vec![
        Span::raw(before.clone()),
        Span::styled(
            caret_ch.to_string(),
            Style::default().add_modifier(Modifier::REVERSED),
        ),
        Span::raw(after),
    ])]);

    // The caret's wrapped line, measured from the text *through* the caret
    // block, so an exact line-fill (common with CJK) can never push the caret
    // just below the visible area and make the typed text appear to vanish.
    let caret_line = if before.is_empty() {
        0
    } else {
        let caret_text = format!("{before}{caret_ch}");
        Paragraph::new(Text::from(vec![RLine::from(caret_text)]))
            .wrap(Wrap { trim: false })
            .line_count(input_width)
            .saturating_sub(1)
    };
    // Keep the caret on the last visible line: scroll only once the caret
    // leaves the bottom of the fixed-height box, and never any earlier.
    let input_scroll = caret_line.saturating_sub(INPUT_HEIGHT_LINES as usize - 1) as u16;
    // Box = fixed content rows + 2 border rows, never larger than the screen
    // (the conversation keeps at least one row).
    let input_height = (INPUT_HEIGHT_LINES + 2).min(f.area().height.saturating_sub(1));

    let [conversation_area, input_area] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(input_height)])
        .areas(f.area());

    // Use the workdir basename in the title to keep it compact; the full path
    // is still visible via `/workdir`.
    let workdir_name = ctx
        .workdir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| ctx.workdir.display().to_string());

    // Conversation: a scroll-anchored chat log. A Paragraph clips cleanly to
    // its area, so long agent replies can never spill over the borders. While
    // generating, a small blinking block rides on the last assistant line.
    //
    // Assistant output is rendered as markdown (headings, code, lists, …);
    // every other kind is plain text. Either way the text is split on `\n` so
    // embedded line breaks are preserved instead of being collapsed.
    let mut conv_lines: Vec<RLine> = Vec::with_capacity(lines.len());
    for (i, l) in lines.iter().enumerate() {
        let style = match l.kind {
            Kind::User => Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            Kind::Assistant => Style::default(),
            Kind::Thinking => Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::ITALIC),
            Kind::Tool => Style::default().fg(Color::Magenta),
            Kind::Info => Style::default().fg(Color::DarkGray),
            Kind::Error => Style::default().fg(Color::Red),
        };
        let mut rendered: Vec<RLine> = if matches!(l.kind, Kind::Assistant) {
            fastclaw::markdown::render_markdown(&l.text)
        } else {
            l.text
                .split('\n')
                .map(|seg| RLine::from(seg.to_string()).style(style))
                .collect()
        };
        let is_streaming_line =
            generating && i == lines.len() - 1 && matches!(l.kind, Kind::Assistant);
        if is_streaming_line {
            let blink = if anim_tick % 2 == 0 {
                Color::White
            } else {
                Color::DarkGray
            };
            match rendered.last_mut() {
                Some(last) => last.spans.push(Span::styled(
                    " ▌",
                    style.add_modifier(Modifier::BOLD).fg(blink),
                )),
                None => rendered.push(RLine::from(Span::styled(
                    " ▌",
                    style.add_modifier(Modifier::BOLD).fg(blink),
                ))),
            }
        }
        conv_lines.extend(rendered);
    }
    if conv_lines.is_empty() {
        conv_lines.push(RLine::from(""));
    }
    let conv_width = conversation_area.width.saturating_sub(2).max(1);
    let conv_wrapped = Paragraph::new(Text::from(conv_lines.clone()))
        .wrap(Wrap { trim: false })
        .line_count(conv_width);
    let conv_inner = conversation_area.height.saturating_sub(2) as usize;
    // Max scroll = rows hidden above the view when pinned to the newest line.
    let conv_max = conv_wrapped.saturating_sub(conv_inner);
    // When pinned the view follows the bottom; otherwise it keeps the user's
    // manual offset (clamped to the content). Persist the derived metrics so
    // the event loop can page-scroll and detect "reached bottom".
    let effective_scroll = if conv_pinned {
        conv_max
    } else {
        conv_scroll.min(conv_max)
    };
    if let Ok(mut s) = ctx.state.lock() {
        s.conv_max = conv_max;
        s.conv_inner = conv_inner;
        s.conv_scroll = effective_scroll;
        if !s.conv_pinned && effective_scroll >= conv_max {
            s.conv_pinned = true;
        }
    }
    let conv_title = format!(
        " FastClaw — session {} [{}] {} ",
        ctx.session_id, ctx.agent_id, workdir_name
    );
    let conv = Paragraph::new(Text::from(conv_lines))
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title(conv_title))
        .scroll((effective_scroll as u16, 0));
    f.render_widget(conv, conversation_area);

    // Input bar: fixed-height box pinned to the bottom; long input scrolls
    // inside it instead of growing the box. The status / spinner live in the
    // box title so there is no separate row that could shift the box.
    let status_text = if generating {
        format!(
            " {} {}",
            SPINNER[(anim_tick as usize) % SPINNER.len()],
            status
        )
    } else {
        status
    };
    let input_title = format!(" You {} ", status_text);
    let input_par = Paragraph::new(input_text.clone())
        .style(Style::default().fg(Color::Green))
        .wrap(Wrap { trim: false })
        .scroll((input_scroll, 0))
        .block(Block::default().borders(Borders::ALL).title(input_title));
    f.render_widget(input_par, input_area);

    // Interactive picker (e.g. `/agent`): a modal overlay drawn on top of the
    // conversation. The input caret isn't placed while it's open.
    if let Some(picker) = &menu {
        let items = &picker.items;
        let count = items.len();
        let visible = PICKER_VISIBLE.min(count);
        let height = (visible as u16 + 2).min(conversation_area.height);
        let max_item_width = items
            .iter()
            .map(|i| RLine::from(i.as_str()).width())
            .max()
            .unwrap_or(0);
        let title = format!(" {} ({}/{}) ", picker.title, picker.selected + 1, count);
        let title_width = title.chars().count() as u16 + 2;
        let width = (max_item_width as u16 + 4)
            .max(title_width)
            .min(f.area().width.saturating_sub(2).max(1));
        let x = f.area().width.saturating_sub(width) / 2;
        let y = f.area().height.saturating_sub(height) / 2;
        let area = Rect::new(x, y, width, height);

        let mut list: Vec<RLine> = Vec::with_capacity(visible);
        for (idx, item) in items[picker.scroll..picker.scroll + visible]
            .iter()
            .enumerate()
        {
            let is_selected = idx + picker.scroll == picker.selected;
            let style = if is_selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            list.push(RLine::from(Span::styled(item.clone(), style)));
        }
        let menu_par = Paragraph::new(Text::from(list))
            .block(Block::default().borders(Borders::ALL).title(title));
        f.render_widget(menu_par, area);
    } else {
        // Place the real terminal cursor at the input caret. Without this the
        // terminal hides the cursor and an IME composition window anchors at a
        // stale position (often the bottom border row), which makes the screen
        // appear to scroll/jump while typing. The caret's (row, col) is located
        // by rendering the input text into a scratch buffer with the exact same
        // wrap + scroll, so CJK width and word-boundary wrapping can't throw it
        // off.
        let caret_area = Rect::new(0, 0, input_width, INPUT_HEIGHT_LINES);
        let mut caret_buf = Buffer::empty(caret_area);
        Paragraph::new(input_text)
            .style(Style::default().fg(Color::Green))
            .wrap(Wrap { trim: false })
            .scroll((input_scroll, 0))
            .render(caret_area, &mut caret_buf);
        let caret_cell = (0..caret_area.height)
            .flat_map(|y| (0..caret_area.width).map(move |x| (x, y)))
            .find(|&(x, y)| {
                caret_buf
                    .cell((x, y))
                    .is_some_and(|c| c.modifier.contains(Modifier::REVERSED))
            });
        if let Some((x, y)) = caret_cell {
            f.set_cursor_position((input_area.x + 1 + x, input_area.y + 1 + y));
        }
    }
}
