//! Ratatui-based UI components for interactive prompts.

use std::io::{stdout, IsTerminal, Stdout, Write};
use std::time::Duration;

use anyhow::Result;
use crossterm::cursor::{MoveToColumn, MoveUp};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    self, disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use ratatui::prelude::*;
use ratatui::widgets::{
    Block, BorderType, Borders, Clear as ClearWidget, HighlightSpacing, List, ListItem, ListState,
    Paragraph,
};

use crate::config::Project;
use crate::git;
use crate::tmux;

// ============================================================================
// Picker
// ============================================================================

/// Maximum height for the inline picker
const PICKER_HEIGHT: u16 = 15;

/// A selectable item in the picker
#[derive(Debug, Clone)]
pub struct PickerItem {
    /// Display label
    pub label: String,
    /// Searchable text (if different from label)
    pub search_text: String,
    /// Optional description shown to the right
    pub description: Option<String>,
    /// Style for the label
    pub style: Style,
}

impl PickerItem {
    pub fn new(label: impl Into<String>) -> Self {
        let label = label.into();
        Self {
            search_text: label.clone(),
            label,
            description: None,
            style: Style::default(),
        }
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    pub fn with_style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn with_search_text(mut self, text: impl Into<String>) -> Self {
        self.search_text = text.into();
        self
    }
}

/// Result from picker selection
#[derive(Debug, Clone)]
pub enum PickerResult {
    /// User selected an item (returns the index)
    Selected(usize),
    /// User cancelled
    Cancelled,
}

struct PickerApp {
    items: Vec<PickerItem>,
    filtered_indices: Vec<usize>,
    list_state: ListState,
    query: String,
    placeholder: String,
    matcher: SkimMatcherV2,
    height: u16,
}

impl PickerApp {
    fn new(items: Vec<PickerItem>, placeholder: String, height: u16) -> Self {
        let filtered_indices: Vec<usize> = (0..items.len()).collect();
        let mut list_state = ListState::default();
        if !items.is_empty() {
            list_state.select(Some(0));
        }

        Self {
            items,
            filtered_indices,
            list_state,
            query: String::new(),
            placeholder,
            matcher: SkimMatcherV2::default(),
            height,
        }
    }

    fn filter_items(&mut self) {
        if self.query.is_empty() {
            self.filtered_indices = (0..self.items.len()).collect();
        } else {
            let mut scored: Vec<(usize, i64)> = self
                .items
                .iter()
                .enumerate()
                .filter_map(|(i, item)| {
                    self.matcher
                        .fuzzy_match(&item.search_text, &self.query)
                        .map(|score| (i, score))
                })
                .collect();

            // Sort by score descending
            scored.sort_by(|a, b| b.1.cmp(&a.1));
            self.filtered_indices = scored.into_iter().map(|(i, _)| i).collect();
        }

        // Reset selection to first item
        if self.filtered_indices.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
    }

    fn move_selection(&mut self, delta: i32) {
        if self.filtered_indices.is_empty() {
            return;
        }

        let current = self.list_state.selected().unwrap_or(0);
        let len = self.filtered_indices.len();
        let new = if delta > 0 {
            (current + delta as usize) % len
        } else {
            (current + len - ((-delta) as usize % len)) % len
        };
        self.list_state.select(Some(new));
    }

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<PickerResult> {
        match code {
            // Cancel
            KeyCode::Esc => return Some(PickerResult::Cancelled),
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(PickerResult::Cancelled)
            }

            // Navigation
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::Char('p') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_selection(-1)
            }
            KeyCode::Char('n') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_selection(1)
            }

            // Selection
            KeyCode::Enter => {
                if let Some(selected) = self.list_state.selected() {
                    if let Some(&original_index) = self.filtered_indices.get(selected) {
                        return Some(PickerResult::Selected(original_index));
                    }
                }
                return Some(PickerResult::Cancelled);
            }

            // Search input
            KeyCode::Backspace => {
                self.query.pop();
                self.filter_items();
            }
            KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
                self.query.push(c);
                self.filter_items();
            }

            _ => {}
        }
        None
    }

    fn render_inline(&mut self, frame: &mut Frame) {
        let area = frame.size();

        // Use the configured height, but cap at terminal height
        let height = self.height.min(area.height);
        let render_area = Rect::new(0, 0, area.width, height);

        // Clear the entire area first
        frame.render_widget(ClearWidget, render_area);

        // Split into search input (1 line) and list
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(render_area);

        // Search input (single line, no border)
        let input_text = if self.query.is_empty() {
            Span::styled(&self.placeholder, Style::default().fg(Color::DarkGray))
        } else {
            Span::styled(&self.query, Style::default().fg(Color::White))
        };

        let input = Paragraph::new(Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::LightMagenta).bold()),
            input_text,
            Span::styled("_", Style::default().fg(Color::LightMagenta)),
        ]));
        frame.render_widget(input, chunks[0]);

        // List items (no border for inline mode)
        let list_items: Vec<ListItem> = self
            .filtered_indices
            .iter()
            .map(|&i| {
                let item = &self.items[i];
                let mut spans = vec![Span::styled(item.label.clone(), item.style)];

                if let Some(ref desc) = item.description {
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(
                        desc,
                        Style::default().fg(Color::DarkGray).italic(),
                    ));
                }

                ListItem::new(Line::from(spans))
            })
            .collect();

        let list = List::new(list_items)
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(80, 60, 120))
                    .fg(Color::White)
                    .bold(),
            )
            .highlight_symbol("\u{276f} ")
            .highlight_spacing(HighlightSpacing::Always);

        frame.render_stateful_widget(list, chunks[1], &mut self.list_state);
    }

    fn render_window(&mut self, frame: &mut Frame) {
        let area = frame.size();

        // Calculate centered popup area - larger size
        let popup_width = (area.width.saturating_sub(4)).min(80);
        let popup_height = (area.height.saturating_sub(4)).min(30);
        let popup_area = centered_rect(popup_width, popup_height, area);

        // Clear the popup area
        frame.render_widget(ClearWidget, popup_area);

        // Split into search input and list
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(1)])
            .split(popup_area);

        // Search input
        let input_text = if self.query.is_empty() {
            Span::styled(&self.placeholder, Style::default().fg(Color::DarkGray))
        } else {
            Span::styled(&self.query, Style::default().fg(Color::White))
        };

        let input = Paragraph::new(Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::LightMagenta)),
            input_text,
            Span::styled("_", Style::default().fg(Color::LightMagenta)),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::LightMagenta))
                .title(" Search ")
                .title_style(Style::default().fg(Color::LightCyan).bold()),
        );
        frame.render_widget(input, chunks[0]);

        // List items
        let list_items: Vec<ListItem> = self
            .filtered_indices
            .iter()
            .map(|&i| {
                let item = &self.items[i];
                let mut spans = vec![Span::styled(item.label.clone(), item.style)];

                if let Some(ref desc) = item.description {
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(
                        desc,
                        Style::default().fg(Color::DarkGray).italic(),
                    ));
                }

                ListItem::new(Line::from(spans))
            })
            .collect();

        let list = List::new(list_items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::LightMagenta)),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(80, 60, 120))
                    .fg(Color::White)
                    .bold(),
            )
            .highlight_symbol("\u{276f} ")
            .highlight_spacing(HighlightSpacing::Always);

        frame.render_stateful_widget(list, chunks[1], &mut self.list_state);
    }
}

/// Show an interactive picker with fuzzy search (inline mode)
pub fn picker(items: Vec<PickerItem>, placeholder: &str) -> Result<PickerResult> {
    picker_with_options(items, placeholder, false)
}

/// Show an interactive picker with fuzzy search (window mode)
#[allow(dead_code)]
pub fn picker_window(items: Vec<PickerItem>, placeholder: &str) -> Result<PickerResult> {
    picker_with_options(items, placeholder, true)
}

fn picker_with_options(
    items: Vec<PickerItem>,
    placeholder: &str,
    window_mode: bool,
) -> Result<PickerResult> {
    if items.is_empty() {
        return Ok(PickerResult::Cancelled);
    }

    if !stdout().is_terminal() {
        anyhow::bail!("Interactive picker requires a terminal");
    }

    let (_, term_height) = terminal::size()?;
    let height = PICKER_HEIGHT.min(term_height.saturating_sub(2));

    let mut app = PickerApp::new(items, placeholder.to_string(), height);

    enable_raw_mode()?;

    let result = if window_mode {
        stdout().execute(EnterAlternateScreen)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        let res = run_picker_loop(&mut terminal, &mut app, true);
        disable_raw_mode()?;
        stdout().execute(LeaveAlternateScreen)?;
        res
    } else {
        // Inline mode: print newlines to make space, then render
        let mut stdout = stdout();

        // Make space for the picker
        for _ in 0..height {
            writeln!(stdout)?;
        }
        // Move cursor back up
        stdout.execute(MoveUp(height))?;
        stdout.execute(MoveToColumn(0))?;

        let mut terminal = Terminal::with_options(
            CrosstermBackend::new(stdout),
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, 0, terminal::size()?.0, height)),
            },
        )?;

        let res = run_picker_loop(&mut terminal, &mut app, false);

        // Clean up: clear the picker area and move cursor
        disable_raw_mode()?;
        let mut out = std::io::stdout();
        out.execute(MoveToColumn(0))?;
        for _ in 0..height {
            out.execute(Clear(ClearType::CurrentLine))?;
            writeln!(out)?;
        }
        out.execute(MoveUp(height))?;
        out.flush()?;

        res
    };

    result
}

fn run_picker_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut PickerApp,
    window_mode: bool,
) -> Result<PickerResult> {
    loop {
        terminal.draw(|frame| {
            if window_mode {
                app.render_window(frame);
            } else {
                app.render_inline(frame);
            }
        })?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if let Some(result) = app.handle_key(key.code, key.modifiers) {
                        return Ok(result);
                    }
                }
            }
        }
    }
}

// ============================================================================
// Confirm Dialog
// ============================================================================

/// Result from confirm dialog
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmResult {
    Yes,
    No,
}

struct ConfirmApp {
    message: String,
    selected: ConfirmResult,
}

impl ConfirmApp {
    fn new(message: String) -> Self {
        Self {
            message,
            selected: ConfirmResult::Yes,
        }
    }

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<ConfirmResult> {
        match code {
            // Cancel
            KeyCode::Esc => return Some(ConfirmResult::No),
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(ConfirmResult::No)
            }

            // Quick keys
            KeyCode::Char('y') | KeyCode::Char('Y') => return Some(ConfirmResult::Yes),
            KeyCode::Char('n') | KeyCode::Char('N') => return Some(ConfirmResult::No),

            // Navigation
            KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                self.selected = match self.selected {
                    ConfirmResult::Yes => ConfirmResult::No,
                    ConfirmResult::No => ConfirmResult::Yes,
                };
            }
            KeyCode::Char('h') => self.selected = ConfirmResult::Yes,
            KeyCode::Char('l') => self.selected = ConfirmResult::No,
            KeyCode::Char('p') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.selected = ConfirmResult::Yes
            }

            // Confirm selection
            KeyCode::Enter => return Some(self.selected),

            _ => {}
        }
        None
    }

    fn render_inline(&self, frame: &mut Frame) {
        let area = frame.size();
        let render_area = Rect::new(0, 0, area.width, 1);

        // Clear the line first
        frame.render_widget(ClearWidget, render_area);

        // Single line: message + buttons
        let yes_style = if self.selected == ConfirmResult::Yes {
            Style::default()
                .bg(Color::LightGreen)
                .fg(Color::Black)
                .bold()
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let no_style = if self.selected == ConfirmResult::No {
            Style::default().bg(Color::LightRed).fg(Color::Black).bold()
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let line = Line::from(vec![
            Span::styled(&self.message, Style::default().fg(Color::White)),
            Span::raw(" "),
            Span::styled(" Yes ", yes_style),
            Span::raw(" "),
            Span::styled(" No ", no_style),
        ]);

        let paragraph = Paragraph::new(line);
        frame.render_widget(paragraph, render_area);
    }

    fn render_window(&self, frame: &mut Frame) {
        let area = frame.size();

        // Calculate centered popup area
        let popup_width = (area.width.saturating_sub(4)).min(60);
        let popup_height = 5;
        let popup_area = centered_rect(popup_width, popup_height, area);

        // Clear the popup area
        frame.render_widget(ClearWidget, popup_area);

        // Split into message and buttons
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .margin(1)
            .split(popup_area);

        // Block around everything
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::LightYellow))
            .title(" Confirm ")
            .title_style(Style::default().fg(Color::LightCyan).bold());
        frame.render_widget(block, popup_area);

        // Message
        let message = Paragraph::new(self.message.clone())
            .style(Style::default().fg(Color::White))
            .alignment(Alignment::Center);
        frame.render_widget(message, chunks[0]);

        // Buttons
        let yes_style = if self.selected == ConfirmResult::Yes {
            Style::default()
                .bg(Color::LightGreen)
                .fg(Color::Black)
                .bold()
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let no_style = if self.selected == ConfirmResult::No {
            Style::default().bg(Color::LightRed).fg(Color::Black).bold()
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let buttons = Line::from(vec![
            Span::raw("  "),
            Span::styled(" Yes ", yes_style),
            Span::raw("   "),
            Span::styled(" No ", no_style),
            Span::raw("  "),
        ]);

        let buttons_widget = Paragraph::new(buttons).alignment(Alignment::Center);
        frame.render_widget(buttons_widget, chunks[1]);
    }
}

/// Show a confirmation dialog (inline mode)
pub fn confirm(message: &str) -> Result<bool> {
    confirm_with_options(message, false)
}

/// Show a confirmation dialog (window mode)
#[allow(dead_code)]
pub fn confirm_window(message: &str) -> Result<bool> {
    confirm_with_options(message, true)
}

fn confirm_with_options(message: &str, window_mode: bool) -> Result<bool> {
    if !stdout().is_terminal() {
        anyhow::bail!("Interactive confirm requires a terminal");
    }

    let mut app = ConfirmApp::new(message.to_string());

    enable_raw_mode()?;

    let result = if window_mode {
        stdout().execute(EnterAlternateScreen)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        let res = run_confirm_loop(&mut terminal, &mut app, true);
        disable_raw_mode()?;
        stdout().execute(LeaveAlternateScreen)?;
        res
    } else {
        // Inline mode
        let mut stdout = stdout();
        writeln!(stdout)?;
        stdout.execute(MoveUp(1))?;
        stdout.execute(MoveToColumn(0))?;

        let mut terminal = Terminal::with_options(
            CrosstermBackend::new(stdout),
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, 0, terminal::size()?.0, 1)),
            },
        )?;

        let res = run_confirm_loop(&mut terminal, &mut app, false);

        disable_raw_mode()?;
        let mut out = std::io::stdout();
        out.execute(MoveToColumn(0))?;
        out.execute(Clear(ClearType::CurrentLine))?;
        out.flush()?;

        res
    };

    Ok(result? == ConfirmResult::Yes)
}

fn run_confirm_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut ConfirmApp,
    window_mode: bool,
) -> Result<ConfirmResult> {
    loop {
        terminal.draw(|frame| {
            if window_mode {
                app.render_window(frame);
            } else {
                app.render_inline(frame);
            }
        })?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if let Some(result) = app.handle_key(key.code, key.modifiers) {
                        return Ok(result);
                    }
                }
            }
        }
    }
}

// ============================================================================
// Input Dialog
// ============================================================================

struct InputApp {
    value: String,
    placeholder: String,
    title: String,
}

impl InputApp {
    fn new(title: String, placeholder: String, default: Option<String>) -> Self {
        Self {
            value: default.unwrap_or_default(),
            placeholder,
            title,
        }
    }

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Option<String>> {
        match code {
            // Cancel
            KeyCode::Esc => return Some(None),
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => return Some(None),

            // Submit
            KeyCode::Enter => {
                if self.value.is_empty() {
                    return Some(None);
                }
                return Some(Some(self.value.clone()));
            }

            // Editing
            KeyCode::Backspace => {
                self.value.pop();
            }
            KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
                self.value.push(c);
            }

            _ => {}
        }
        None
    }

    fn render_inline(&self, frame: &mut Frame) {
        let area = frame.size();
        let render_area = Rect::new(0, 0, area.width, 1);

        // Clear the line first
        frame.render_widget(ClearWidget, render_area);

        // Single line: title + input
        let input_text = if self.value.is_empty() {
            Span::styled(&self.placeholder, Style::default().fg(Color::DarkGray))
        } else {
            Span::styled(&self.value, Style::default().fg(Color::White))
        };

        let line = Line::from(vec![
            Span::styled(&self.title, Style::default().fg(Color::LightCyan).bold()),
            Span::raw(": "),
            input_text,
            Span::styled("_", Style::default().fg(Color::LightMagenta)),
        ]);

        let paragraph = Paragraph::new(line);
        frame.render_widget(paragraph, render_area);
    }

    fn render_window(&self, frame: &mut Frame) {
        let area = frame.size();

        // Calculate centered popup area
        let popup_width = (area.width.saturating_sub(4)).min(70);
        let popup_height = 3;
        let popup_area = centered_rect(popup_width, popup_height, area);

        // Clear the popup area
        frame.render_widget(ClearWidget, popup_area);

        // Input text
        let input_text = if self.value.is_empty() {
            Span::styled(&self.placeholder, Style::default().fg(Color::DarkGray))
        } else {
            Span::styled(&self.value, Style::default().fg(Color::White))
        };

        let input = Paragraph::new(Line::from(vec![
            input_text,
            Span::styled("_", Style::default().fg(Color::LightMagenta)),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::LightMagenta))
                .title(format!(" {} ", self.title))
                .title_style(Style::default().fg(Color::LightCyan).bold()),
        );
        frame.render_widget(input, popup_area);
    }
}

/// Show an input dialog (inline mode)
pub fn input(title: &str, placeholder: &str, default: Option<&str>) -> Result<Option<String>> {
    input_with_options(title, placeholder, default, false)
}

/// Show an input dialog (window mode)
#[allow(dead_code)]
pub fn input_window(
    title: &str,
    placeholder: &str,
    default: Option<&str>,
) -> Result<Option<String>> {
    input_with_options(title, placeholder, default, true)
}

fn input_with_options(
    title: &str,
    placeholder: &str,
    default: Option<&str>,
    window_mode: bool,
) -> Result<Option<String>> {
    if !stdout().is_terminal() {
        anyhow::bail!("Interactive input requires a terminal");
    }

    let mut app = InputApp::new(
        title.to_string(),
        placeholder.to_string(),
        default.map(|s| s.to_string()),
    );

    enable_raw_mode()?;

    let result = if window_mode {
        stdout().execute(EnterAlternateScreen)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
        let res = run_input_loop(&mut terminal, &mut app, true);
        disable_raw_mode()?;
        stdout().execute(LeaveAlternateScreen)?;
        res
    } else {
        // Inline mode
        let mut stdout = stdout();
        writeln!(stdout)?;
        stdout.execute(MoveUp(1))?;
        stdout.execute(MoveToColumn(0))?;

        let mut terminal = Terminal::with_options(
            CrosstermBackend::new(stdout),
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, 0, terminal::size()?.0, 1)),
            },
        )?;

        let res = run_input_loop(&mut terminal, &mut app, false);

        disable_raw_mode()?;
        let mut out = std::io::stdout();
        out.execute(MoveToColumn(0))?;
        out.execute(Clear(ClearType::CurrentLine))?;
        out.flush()?;

        res
    };

    result
}

fn run_input_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut InputApp,
    window_mode: bool,
) -> Result<Option<String>> {
    loop {
        terminal.draw(|frame| {
            if window_mode {
                app.render_window(frame);
            } else {
                app.render_inline(frame);
            }
        })?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if let Some(result) = app.handle_key(key.code, key.modifiers) {
                        return Ok(result);
                    }
                }
            }
        }
    }
}

// ============================================================================
// High-level Project/Worktree Pickers
// ============================================================================

/// Select a project from the list
pub fn select_project(placeholder: &str) -> Result<Option<String>> {
    let projects = Project::list_all()?;

    if projects.is_empty() {
        anyhow::bail!("No projects found. Create one with: twig new <name>");
    }

    if projects.len() == 1 {
        return Ok(Some(projects.into_iter().next().unwrap()));
    }

    let running_sessions = tmux::list_sessions().unwrap_or_default();

    let items: Vec<PickerItem> = projects
        .iter()
        .map(|name| {
            let is_running = running_sessions.contains(name);
            let mut item =
                PickerItem::new(name.clone()).with_style(Style::default().fg(Color::LightYellow));

            if is_running {
                item = item.with_description("\u{25cf} running");
            }

            item
        })
        .collect();

    match picker(items, placeholder)? {
        PickerResult::Selected(i) => Ok(Some(projects[i].clone())),
        PickerResult::Cancelled => Ok(None),
    }
}

/// Select a worktree from a project
pub fn select_worktree(project: &Project, placeholder: &str) -> Result<Option<String>> {
    let worktrees = git::list_worktrees(project)?;

    if worktrees.is_empty() {
        anyhow::bail!("No worktrees found for project '{}'", project.name);
    }

    let running_sessions = tmux::list_sessions().unwrap_or_default();

    let items: Vec<PickerItem> = worktrees
        .iter()
        .map(|wt| {
            let session_name = format!("{}__{}", project.name, wt.branch);
            let is_running = running_sessions.contains(&session_name);

            let mut item = PickerItem::new(wt.branch.clone())
                .with_style(Style::default().fg(Color::LightCyan))
                .with_search_text(format!("{} {}", project.name, wt.branch));

            if is_running {
                item = item.with_description("\u{25cf} running");
            }

            item
        })
        .collect();

    match picker(items, placeholder)? {
        PickerResult::Selected(i) => Ok(Some(worktrees[i].branch.clone())),
        PickerResult::Cancelled => Ok(None),
    }
}

/// Select a project and optionally a worktree
/// Returns (project_name, optional_branch)
#[allow(dead_code)]
pub fn select_project_or_worktree(placeholder: &str) -> Result<Option<(String, Option<String>)>> {
    let projects = Project::list_all()?;

    if projects.is_empty() {
        anyhow::bail!("No projects found. Create one with: twig new <name>");
    }

    let running_sessions = tmux::list_sessions().unwrap_or_default();

    // Build combined list: projects and their worktrees
    let mut items: Vec<PickerItem> = Vec::new();
    let mut item_map: Vec<(String, Option<String>)> = Vec::new(); // (project, branch)

    for project_name in &projects {
        // Add project
        let is_running = running_sessions.contains(project_name);
        let mut item = PickerItem::new(project_name.clone())
            .with_style(Style::default().fg(Color::LightYellow).bold());

        if is_running {
            item = item.with_description("\u{25cf} running");
        }

        items.push(item);
        item_map.push((project_name.clone(), None));

        // Add worktrees for this project
        if let Ok(project) = Project::load(project_name) {
            if let Ok(worktrees) = git::list_worktrees(&project) {
                for wt in worktrees {
                    let session_name = format!("{}__{}", project_name, wt.branch);
                    let is_wt_running = running_sessions.contains(&session_name);

                    let label = format!("  {} / {}", project_name, wt.branch);
                    let mut wt_item = PickerItem::new(label)
                        .with_style(Style::default().fg(Color::LightCyan))
                        .with_search_text(format!("{} {}", project_name, wt.branch));

                    if is_wt_running {
                        wt_item = wt_item.with_description("\u{25cf} running");
                    }

                    items.push(wt_item);
                    item_map.push((project_name.clone(), Some(wt.branch)));
                }
            }
        }
    }

    match picker(items, placeholder)? {
        PickerResult::Selected(i) => Ok(Some(item_map[i].clone())),
        PickerResult::Cancelled => Ok(None),
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Create a centered rect
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}
