//! Interactive tree view for projects and worktrees using Ratatui.

use std::env;
use std::io::{self, stdout, IsTerminal};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use tui_tree_widget::{Tree, TreeItem, TreeState};

use crate::config::Project;
use crate::git::{self, WorktreeInfo};
use crate::tmux::{self, SessionBuilder};

/// Current session context from environment
struct CurrentContext {
    project: Option<String>,
    worktree: Option<String>,
}

impl CurrentContext {
    fn from_env() -> Self {
        Self {
            project: env::var("TWIG_PROJECT").ok(),
            worktree: env::var("TWIG_WORKTREE").ok(),
        }
    }

    fn is_current_project(&self, name: &str) -> bool {
        self.project.as_deref() == Some(name) && self.worktree.is_none()
    }

    fn is_current_worktree(&self, project: &str, branch: &str) -> bool {
        self.project.as_deref() == Some(project) && self.worktree.as_deref() == Some(branch)
    }
}

/// Unique identifier for tree nodes
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum TreeNodeId {
    #[default]
    Root,
    Project(String),
    Worktree {
        project: String,
        branch: String,
    },
}

/// Action to perform after tree view exits
#[derive(Debug, Clone)]
pub enum SelectedAction {
    StartProject(String),
    StartWorktree { project: String, branch: String },
    KillProject(String),
    KillWorktree { project: String, branch: String },
}

/// Search candidate for fuzzy matching
struct SearchCandidate {
    /// Searchable text (e.g., "project / branch")
    label: String,
    /// Full path to this node in the tree
    node_path: Vec<TreeNodeId>,
    /// Parent project name (for opening parent when searching)
    project: String,
}

/// Data for a project and its worktrees
struct ProjectData {
    name: String,
    worktrees: Vec<WorktreeInfo>,
    session_running: bool,
}

/// Mode for the tree view
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TreeViewMode {
    /// Normal mode: show all projects/worktrees, start sessions on select
    Start,
    /// Kill mode: show only running sessions, kill on select
    Kill,
}

/// Status message to display in the tree view
#[derive(Debug, Clone)]
struct StatusMessage {
    text: String,
    is_error: bool,
    timestamp: Instant,
}

impl StatusMessage {
    fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: false,
            timestamp: Instant::now(),
        }
    }

    fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: true,
            timestamp: Instant::now(),
        }
    }

    fn is_expired(&self) -> bool {
        self.timestamp.elapsed() > Duration::from_secs(3)
    }
}

/// App state for the tree view
struct TreeViewApp<'a> {
    tree_items: Vec<TreeItem<'a, TreeNodeId>>,
    tree_state: TreeState<TreeNodeId>,
    candidates: Vec<SearchCandidate>,
    query: String,
    no_match: bool,
    search_mode: bool,
    mode: TreeViewMode,
    status_message: Option<StatusMessage>,
    /// Session to switch to after exiting (when current session was deleted)
    switch_to_session: Option<String>,
}

impl<'a> TreeViewApp<'a> {
    fn new(
        projects: Vec<ProjectData>,
        running_sessions: &[String],
        mode: TreeViewMode,
        current: &CurrentContext,
        focus_current: bool,
    ) -> Result<Self> {
        let tree_items = build_tree_items(&projects, running_sessions, current)?;
        let candidates = build_candidates(&projects);

        let mut tree_state = TreeState::default();

        // Open all projects by default and select first item
        for project in &projects {
            tree_state.open(vec![TreeNodeId::Project(project.name.clone())]);
        }
        if focus_current {
            let mut selected = None;

            if let Some(project_name) = current.project.as_deref() {
                let has_project = projects.iter().any(|project| project.name == project_name);

                if has_project {
                    if let Some(branch) = current.worktree.as_deref() {
                        let has_worktree = projects.iter().any(|project| {
                            project.name == project_name
                                && project.worktrees.iter().any(|wt| wt.branch == branch)
                        });

                        if has_worktree {
                            selected = Some(vec![
                                TreeNodeId::Project(project_name.to_string()),
                                TreeNodeId::Worktree {
                                    project: project_name.to_string(),
                                    branch: branch.to_string(),
                                },
                            ]);
                        }
                    }

                    if selected.is_none() {
                        selected = Some(vec![TreeNodeId::Project(project_name.to_string())]);
                    }
                }
            }

            if let Some(node_path) = selected {
                tree_state.select(node_path);
                tree_state.scroll_selected_into_view();
            } else if !projects.is_empty() {
                tree_state.select(vec![TreeNodeId::Project(projects[0].name.clone())]);
            }
        } else if !projects.is_empty() {
            tree_state.select(vec![TreeNodeId::Project(projects[0].name.clone())]);
        }

        Ok(Self {
            tree_items,
            tree_state,
            candidates,
            query: String::new(),
            search_mode: false,
            no_match: false,
            mode,
            status_message: None,
            switch_to_session: None,
        })
    }

    /// Refresh tree data (after worktree operations)
    fn refresh(&mut self, select_project: Option<&str>) -> Result<()> {
        let running_sessions = tmux::list_sessions().unwrap_or_default();
        let current = CurrentContext::from_env();

        // Reload all project data
        let opts = LoadOptions {
            project_filter: None,
            running_only: self.mode == TreeViewMode::Kill,
            include_worktrees: true,
        };
        let projects = load_project_data(opts)?;

        self.tree_items = build_tree_items(&projects, &running_sessions, &current)?;
        self.candidates = build_candidates(&projects);

        // Re-open all projects
        for project in &projects {
            self.tree_state
                .open(vec![TreeNodeId::Project(project.name.clone())]);
        }

        // Select the specified project or first item
        if let Some(project_name) = select_project {
            self.tree_state
                .select(vec![TreeNodeId::Project(project_name.to_string())]);
        } else if !projects.is_empty() {
            self.tree_state
                .select(vec![TreeNodeId::Project(projects[0].name.clone())]);
        }

        Ok(())
    }

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<HandleResult> {
        // Search mode handling
        if self.search_mode {
            return self.handle_search_key(code, modifiers);
        }

        match code {
            // Quit
            KeyCode::Char('q') | KeyCode::Esc => {
                return Some(HandleResult::Quit);
            }
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(HandleResult::Quit);
            }

            // Enter search mode
            KeyCode::Char('/') => {
                self.search_mode = true;
                self.query.clear();
                self.no_match = false;
            }

            // Stop/Kill session
            KeyCode::Char('s') | KeyCode::Char('S') => {
                if let Some(action) = self.get_selected_action() {
                    let kill_action = match action {
                        SelectedAction::StartProject(name) | SelectedAction::KillProject(name) => {
                            SelectedAction::KillProject(name)
                        }
                        SelectedAction::StartWorktree { project, branch }
                        | SelectedAction::KillWorktree { project, branch } => {
                            SelectedAction::KillWorktree { project, branch }
                        }
                    };
                    return Some(HandleResult::Action(kill_action));
                }
            }

            // Fork worktree
            KeyCode::Char('f') | KeyCode::Char('F') => {
                if let Some(project) = self.get_selected_project() {
                    return Some(HandleResult::ForkWorktree(project));
                }
            }

            // Merge worktree (only on worktree nodes)
            KeyCode::Char('m') | KeyCode::Char('M') => {
                if let Some((project, branch)) = self.get_selected_worktree() {
                    return Some(HandleResult::MergeWorktree { project, branch });
                }
            }

            // Delete worktree (only on worktree nodes)
            KeyCode::Char('d') | KeyCode::Char('D') => {
                if let Some((project, branch)) = self.get_selected_worktree() {
                    return Some(HandleResult::DeleteWorktree { project, branch });
                }
            }

            // Navigation
            KeyCode::Up | KeyCode::Char('k') => {
                self.tree_state.key_up();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.tree_state.key_down();
            }
            KeyCode::Char('p') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.tree_state.key_up();
            }
            KeyCode::Char('n') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.tree_state.key_down();
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.tree_state.key_left();
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.tree_state.key_right();
            }

            // Selection
            KeyCode::Enter => {
                if let Some(action) = self.get_selected_action() {
                    return Some(HandleResult::Action(action));
                }
            }

            _ => {}
        }
        None
    }

    fn handle_search_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Option<HandleResult> {
        match code {
            // Exit search mode (keep cursor position)
            KeyCode::Esc => {
                self.search_mode = false;
                self.query.clear();
                self.no_match = false;
            }
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.search_mode = false;
                self.query.clear();
                self.no_match = false;
            }

            // Confirm search and trigger selection action
            KeyCode::Enter => {
                if let Some(action) = self.get_selected_action() {
                    self.search_mode = false;
                    self.query.clear();
                    self.no_match = false;
                    return Some(HandleResult::Action(action));
                }
            }

            // Search input
            KeyCode::Backspace => {
                self.query.pop();
                if self.query.is_empty() {
                    self.no_match = false;
                } else {
                    self.do_fuzzy_search();
                }
            }
            KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
                self.query.push(c);
                self.do_fuzzy_search();
            }

            // Allow navigation while searching
            KeyCode::Up => {
                self.tree_state.key_up();
            }
            KeyCode::Down => {
                self.tree_state.key_down();
            }
            KeyCode::Char('p') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.tree_state.key_up();
            }
            KeyCode::Char('n') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.tree_state.key_down();
            }

            _ => {}
        }
        None
    }

    fn do_fuzzy_search(&mut self) {
        if self.query.is_empty() {
            self.no_match = false;
            return;
        }

        let matcher = SkimMatcherV2::default();
        let mut best_match: Option<(&SearchCandidate, i64)> = None;

        for candidate in &self.candidates {
            if let Some(score) = matcher.fuzzy_match(&candidate.label, &self.query) {
                match &best_match {
                    None => best_match = Some((candidate, score)),
                    Some((_, best_score)) if score > *best_score => {
                        best_match = Some((candidate, score));
                    }
                    _ => {}
                }
            }
        }

        if let Some((candidate, _)) = best_match {
            self.no_match = false;
            // Ensure parent project is open
            self.tree_state
                .open(vec![TreeNodeId::Project(candidate.project.clone())]);
            // Select the matched node
            self.tree_state.select(candidate.node_path.clone());
            self.tree_state.scroll_selected_into_view();
        } else {
            self.no_match = true;
        }
    }

    fn get_selected_action(&self) -> Option<SelectedAction> {
        let selected = self.tree_state.selected();
        if selected.is_empty() {
            return None;
        }

        match &selected[selected.len() - 1] {
            TreeNodeId::Root => None,
            TreeNodeId::Project(name) => match self.mode {
                TreeViewMode::Start => Some(SelectedAction::StartProject(name.clone())),
                TreeViewMode::Kill => Some(SelectedAction::KillProject(name.clone())),
            },
            TreeNodeId::Worktree { project, branch } => match self.mode {
                TreeViewMode::Start => Some(SelectedAction::StartWorktree {
                    project: project.clone(),
                    branch: branch.clone(),
                }),
                TreeViewMode::Kill => Some(SelectedAction::KillWorktree {
                    project: project.clone(),
                    branch: branch.clone(),
                }),
            },
        }
    }

    /// Get the project name from the current selection (works for both project and worktree nodes)
    fn get_selected_project(&self) -> Option<String> {
        let selected = self.tree_state.selected();
        if selected.is_empty() {
            return None;
        }

        match &selected[selected.len() - 1] {
            TreeNodeId::Root => None,
            TreeNodeId::Project(name) => Some(name.clone()),
            TreeNodeId::Worktree { project, .. } => Some(project.clone()),
        }
    }

    /// Get worktree info if current selection is a worktree
    fn get_selected_worktree(&self) -> Option<(String, String)> {
        let selected = self.tree_state.selected();
        if selected.is_empty() {
            return None;
        }

        match &selected[selected.len() - 1] {
            TreeNodeId::Worktree { project, branch } => Some((project.clone(), branch.clone())),
            _ => None,
        }
    }

    /// Check if current selection is a worktree
    fn is_worktree_selected(&self) -> bool {
        self.get_selected_worktree().is_some()
    }

    fn build_default_status_line(&self) -> Line<'static> {
        let separator_color = match self.mode {
            TreeViewMode::Start => Color::LightMagenta,
            TreeViewMode::Kill => Color::LightRed,
        };
        let is_worktree = self.is_worktree_selected();

        let mut spans = vec![
            Span::styled("j/k", Style::default().fg(Color::LightCyan)),
            Span::styled(" or ", Style::default().fg(Color::Gray)),
            Span::styled("^p/^n", Style::default().fg(Color::LightCyan)),
            Span::styled(" nav ", Style::default().fg(Color::Gray)),
            Span::styled("\u{2502} ", Style::default().fg(separator_color)),
            Span::styled("/", Style::default().fg(Color::LightCyan)),
            Span::styled(" search ", Style::default().fg(Color::Gray)),
            Span::styled("\u{2502} ", Style::default().fg(separator_color)),
            Span::styled("f", Style::default().fg(Color::LightCyan)),
            Span::styled("ork ", Style::default().fg(Color::Gray)),
            Span::styled("\u{2502} ", Style::default().fg(separator_color)),
            Span::styled("s", Style::default().fg(Color::LightCyan)),
            Span::styled("top ", Style::default().fg(Color::Gray)),
        ];

        // Show worktree-specific shortcuts only when on a worktree
        if is_worktree {
            spans.extend([
                Span::styled("\u{2502} ", Style::default().fg(separator_color)),
                Span::styled("m", Style::default().fg(Color::LightCyan)),
                Span::styled("erge ", Style::default().fg(Color::Gray)),
                Span::styled("\u{2502} ", Style::default().fg(separator_color)),
                Span::styled("d", Style::default().fg(Color::LightCyan)),
                Span::styled("elete ", Style::default().fg(Color::Gray)),
            ]);
        }

        spans.extend([
            Span::styled("\u{2502} ", Style::default().fg(separator_color)),
            Span::styled("q", Style::default().fg(Color::LightCyan)),
            Span::styled("uit", Style::default().fg(Color::Gray)),
        ]);

        Line::from(spans)
    }

    fn render(&mut self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(1)])
            .split(frame.size());

        // Tree widget with glamorous styling
        let (title, border_color) = match self.mode {
            TreeViewMode::Start => (" Projects / Worktrees ", Color::LightMagenta),
            TreeViewMode::Kill => (" Kill Session ", Color::LightRed),
        };

        let tree = Tree::new(&self.tree_items)
            .expect("unique identifiers")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(border_color))
                    .title(title)
                    .title_style(Style::default().fg(Color::LightCyan).bold()),
            )
            .style(Style::default().fg(Color::White))
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(80, 60, 120)) // Soft purple background
                    .fg(Color::White)
                    .bold(),
            )
            .highlight_symbol("\u{276f} ") // Heavy right-pointing angle ❯
            .node_closed_symbol("\u{25b8} ") // Small arrow right ▸
            .node_open_symbol("\u{25be} ") // Small arrow down ▾
            .node_no_children_symbol("  ");

        frame.render_stateful_widget(tree, chunks[0], &mut self.tree_state);

        // Status bar with styling
        // Check for status message (takes priority)
        let status_line = if let Some(ref msg) = self.status_message {
            if !msg.is_expired() {
                let color = if msg.is_error {
                    Color::LightRed
                } else {
                    Color::LightGreen
                };
                Line::from(vec![Span::styled(&msg.text, Style::default().fg(color))])
            } else {
                self.build_default_status_line()
            }
        } else if self.search_mode {
            // Search mode - show search input
            let mut spans = vec![Span::styled(
                "/",
                Style::default().fg(Color::LightMagenta).bold(),
            )];
            if self.query.is_empty() {
                spans.push(Span::styled(
                    "type to search...",
                    Style::default().fg(Color::DarkGray).italic(),
                ));
            } else {
                let query_color = if self.no_match {
                    Color::LightRed
                } else {
                    Color::LightGreen
                };
                spans.push(Span::styled(
                    &self.query,
                    Style::default().fg(query_color).bold(),
                ));
            }
            spans.push(Span::styled("_", Style::default().fg(Color::LightMagenta)));
            spans.push(Span::styled(
                "  (Esc to exit)",
                Style::default().fg(Color::DarkGray),
            ));
            Line::from(spans)
        } else {
            self.build_default_status_line()
        };

        let status = Paragraph::new(status_line);
        frame.render_widget(status, chunks[1]);
    }
}

enum HandleResult {
    Quit,
    Action(SelectedAction),
    /// Fork worktree - handled internally, returns to tree view if cancelled
    ForkWorktree(String),
    /// Merge worktree - handled internally with refresh
    MergeWorktree {
        project: String,
        branch: String,
    },
    /// Delete worktree - handled internally with refresh
    DeleteWorktree {
        project: String,
        branch: String,
    },
}

/// Build tree items from project data
fn build_tree_items<'a>(
    projects: &[ProjectData],
    running_sessions: &[String],
    current: &CurrentContext,
) -> Result<Vec<TreeItem<'a, TreeNodeId>>> {
    let mut items = Vec::new();

    for project in projects {
        let is_current = current.is_current_project(&project.name);

        // Build styled project text - use magenta for current, yellow for others
        let name_style = if is_current {
            Style::default().fg(Color::LightMagenta).bold()
        } else {
            Style::default().fg(Color::LightYellow).bold()
        };

        // Current indicator before name, with spacing for alignment
        let mut spans = if is_current {
            vec![Span::styled(
                "\u{25b6} ", // ▶ current indicator
                Style::default().fg(Color::LightMagenta),
            )]
        } else {
            vec![Span::raw("  ")] // spacing for alignment
        };

        spans.push(Span::styled(project.name.clone(), name_style));

        if project.session_running {
            spans.push(Span::styled(
                " \u{25cf}",
                Style::default().fg(Color::LightGreen),
            ));
            spans.push(Span::styled(
                " running",
                Style::default().fg(Color::LightGreen).italic(),
            ));
        }

        let project_line: Line = Line::from(spans);

        let children: Vec<TreeItem<'a, TreeNodeId>> = project
            .worktrees
            .iter()
            .map(|wt| {
                let session_name = format!("{}__{}", project.name, wt.branch);
                let is_running = running_sessions.contains(&session_name);
                let is_current_wt = current.is_current_worktree(&project.name, &wt.branch);

                // Build styled worktree text - use magenta for current, cyan for others
                let branch_style = if is_current_wt {
                    Style::default().fg(Color::LightMagenta).bold()
                } else {
                    Style::default().fg(Color::LightCyan)
                };

                // Current indicator before name, with spacing for alignment
                let mut wt_spans = if is_current_wt {
                    vec![Span::styled(
                        "\u{25b6} ", // ▶ current indicator
                        Style::default().fg(Color::LightMagenta),
                    )]
                } else {
                    vec![Span::raw("  ")] // spacing for alignment
                };

                wt_spans.push(Span::styled(wt.branch.clone(), branch_style));

                if is_running {
                    wt_spans.push(Span::styled(
                        " \u{25cf}",
                        Style::default().fg(Color::LightGreen),
                    ));
                    wt_spans.push(Span::styled(
                        " running",
                        Style::default().fg(Color::LightGreen).italic(),
                    ));
                }

                let wt_line: Line = Line::from(wt_spans);

                TreeItem::new_leaf(
                    TreeNodeId::Worktree {
                        project: project.name.clone(),
                        branch: wt.branch.clone(),
                    },
                    wt_line,
                )
            })
            .collect();

        let item = if children.is_empty() {
            TreeItem::new_leaf(TreeNodeId::Project(project.name.clone()), project_line)
        } else {
            TreeItem::new(
                TreeNodeId::Project(project.name.clone()),
                project_line,
                children,
            )
            .context("Failed to create tree item")?
        };

        items.push(item);
    }

    Ok(items)
}

/// Build search candidates from project data
fn build_candidates(projects: &[ProjectData]) -> Vec<SearchCandidate> {
    let mut candidates = Vec::new();

    for project in projects {
        // Add project as candidate
        candidates.push(SearchCandidate {
            label: project.name.clone(),
            node_path: vec![TreeNodeId::Project(project.name.clone())],
            project: project.name.clone(),
        });

        // Add worktrees as candidates (with project name for better matching)
        for wt in &project.worktrees {
            candidates.push(SearchCandidate {
                label: format!("{} / {}", project.name, wt.branch),
                node_path: vec![
                    TreeNodeId::Project(project.name.clone()),
                    TreeNodeId::Worktree {
                        project: project.name.clone(),
                        branch: wt.branch.clone(),
                    },
                ],
                project: project.name.clone(),
            });
        }
    }

    candidates
}

/// Options for loading project data
struct LoadOptions {
    /// Filter to a specific project name
    project_filter: Option<String>,
    /// Only show running sessions
    running_only: bool,
    /// Include worktrees (false = projects only)
    include_worktrees: bool,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            project_filter: None,
            running_only: false,
            include_worktrees: true,
        }
    }
}

/// Load project data (projects + optionally their worktrees)
fn load_project_data(opts: LoadOptions) -> Result<Vec<ProjectData>> {
    let project_names = Project::list_all()?;
    let running_sessions = tmux::list_sessions().unwrap_or_default();

    let mut data = Vec::new();

    for name in project_names {
        // Apply filter if provided
        if let Some(ref filter) = opts.project_filter {
            if name != *filter {
                continue;
            }
        }

        let project = match Project::load(&name) {
            Ok(p) => p,
            Err(_) => continue, // Skip projects that fail to load
        };

        let session_running = running_sessions.contains(&name);

        // Get worktrees only if requested
        let filtered_worktrees: Vec<WorktreeInfo> = if opts.include_worktrees {
            let worktrees = git::list_worktrees(&project).unwrap_or_default();

            // Filter worktrees to only running ones if running_only
            if opts.running_only {
                worktrees
                    .into_iter()
                    .filter(|wt| {
                        let session_name = format!("{}__{}", name, wt.branch);
                        running_sessions.contains(&session_name)
                    })
                    .collect()
            } else {
                worktrees
            }
        } else {
            Vec::new()
        };

        // In running_only mode, skip projects with no running sessions
        if opts.running_only && !session_running && filtered_worktrees.is_empty() {
            continue;
        }

        data.push(ProjectData {
            name,
            worktrees: filtered_worktrees,
            session_running,
        });
    }

    Ok(data)
}

/// Run the interactive tree view for starting sessions (with worktrees)
pub fn run(project_filter: Option<String>, focus_current: bool) -> Result<Option<SelectedAction>> {
    run_with_options(
        LoadOptions {
            project_filter,
            running_only: false,
            include_worktrees: true,
        },
        TreeViewMode::Start,
        focus_current,
    )
}

/// Run the interactive tree view for killing sessions (shows only running)
pub fn run_for_kill(session_filter: Option<String>) -> Result<Option<SelectedAction>> {
    run_with_options(
        LoadOptions {
            project_filter: session_filter,
            running_only: true,
            include_worktrees: true,
        },
        TreeViewMode::Kill,
        false,
    )
}

/// Run the interactive tree view with specified options
fn run_with_options(
    opts: LoadOptions,
    mode: TreeViewMode,
    focus_current: bool,
) -> Result<Option<SelectedAction>> {
    let filter = opts.project_filter.clone();
    let running_only = opts.running_only;
    let projects = load_project_data(opts)?;

    if projects.is_empty() {
        if running_only {
            anyhow::bail!("No twig sessions running");
        } else if filter.is_some() {
            anyhow::bail!("Project '{}' not found", filter.as_deref().unwrap_or(""));
        } else {
            println!("No projects found. Create one with: twig new <name>");
            return Ok(None);
        }
    }

    // Check if running in a terminal
    if !stdout().is_terminal() {
        anyhow::bail!(
            "Interactive tree view requires a terminal. Run in a TTY or use a different command."
        );
    }

    let running_sessions = tmux::list_sessions().unwrap_or_default();
    let current = CurrentContext::from_env();
    let mut app = TreeViewApp::new(projects, &running_sessions, mode, &current, focus_current)?;

    // Setup terminal
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let result = run_event_loop(&mut terminal, &mut app);

    // Restore terminal
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    result
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut TreeViewApp,
) -> Result<Option<SelectedAction>> {
    loop {
        // Clear expired status messages
        if let Some(ref msg) = app.status_message {
            if msg.is_expired() {
                app.status_message = None;
            }
        }

        terminal.draw(|frame| app.render(frame))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if let Some(result) = app.handle_key(key.code, key.modifiers) {
                        match result {
                            HandleResult::Quit => {
                                // If we need to switch sessions, return that info
                                if let Some(session) = app.switch_to_session.take() {
                                    return Ok(Some(SelectedAction::StartProject(session)));
                                }
                                return Ok(None);
                            }
                            HandleResult::Action(action) => return Ok(Some(action)),
                            HandleResult::ForkWorktree(project) => {
                                // If fork creates a session, return the action to start it
                                if let Some(action) = handle_fork_worktree(terminal, app, &project)?
                                {
                                    return Ok(Some(action));
                                }
                            }
                            HandleResult::MergeWorktree { project, branch } => {
                                handle_merge_worktree(terminal, app, &project, &branch)?;
                            }
                            HandleResult::DeleteWorktree { project, branch } => {
                                handle_delete_worktree(terminal, app, &project, &branch)?;
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Handle fork worktree operation with input overlay
fn handle_fork_worktree(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut TreeViewApp,
    project_name: &str,
) -> Result<Option<SelectedAction>> {
    let project = match Project::load(project_name) {
        Ok(p) => p,
        Err(e) => {
            app.status_message = Some(StatusMessage::error(format!(
                "Failed to load project: {}",
                e
            )));
            return Ok(None);
        }
    };

    // Show input overlay for branch name
    let title = format!("New worktree for '{}'", project_name);
    let branch_name = match show_input_overlay(terminal, app, &title, "Enter branch name...")? {
        Some(name) if !name.is_empty() => name,
        _ => return Ok(None), // Cancelled or empty
    };

    // Show progress
    app.status_message = Some(StatusMessage::info(format!(
        "Creating '{}'...",
        branch_name
    )));
    terminal.draw(|frame| app.render(frame))?;

    // Create the git worktree
    let worktree_path = match git::create_worktree(&project, &branch_name) {
        Ok(path) => path,
        Err(e) => {
            app.status_message = Some(StatusMessage::error(format!(
                "Failed to create worktree: {}",
                e
            )));
            return Ok(None);
        }
    };

    // Create and start tmux session for the worktree
    let session_name = project.worktree_session_name(&branch_name);

    // Check if session already exists (unlikely but possible)
    if tmux::session_exists(&session_name)? {
        app.status_message = Some(StatusMessage::info(format!(
            "Session '{}' already exists",
            session_name
        )));
        return Ok(Some(SelectedAction::StartWorktree {
            project: project_name.to_string(),
            branch: branch_name,
        }));
    }

    // Create the session with setup window
    let builder = SessionBuilder::new(&project)
        .with_session_name(session_name.clone())
        .with_root(worktree_path.to_string_lossy().to_string())
        .with_worktree(branch_name.clone());

    if let Err(e) = builder.create_session() {
        app.status_message = Some(StatusMessage::error(format!(
            "Failed to create session: {}",
            e
        )));
        return Ok(None);
    }

    // If there are post-create commands, run them then setup windows
    if builder.has_post_create_commands() {
        if let Err(e) = builder.run_post_create_then("twig project setup-windows") {
            app.status_message = Some(StatusMessage::error(format!(
                "Failed to start setup: {}",
                e
            )));
            return Ok(None);
        }
    } else {
        // No post-create commands, setup windows immediately
        if let Err(e) = builder.setup_windows() {
            app.status_message = Some(StatusMessage::error(format!(
                "Failed to setup windows: {}",
                e
            )));
            return Ok(None);
        }
    }

    // Return action to start the worktree session
    Ok(Some(SelectedAction::StartWorktree {
        project: project_name.to_string(),
        branch: branch_name,
    }))
}

/// Handle merge worktree operation with confirmation
fn handle_merge_worktree(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut TreeViewApp,
    project_name: &str,
    branch_name: &str,
) -> Result<()> {
    let project = match Project::load(project_name) {
        Ok(p) => p,
        Err(e) => {
            app.status_message = Some(StatusMessage::error(format!(
                "Failed to load project: {}",
                e
            )));
            return Ok(());
        }
    };

    let default_branch = match git::get_default_branch(&project.root_expanded()) {
        Ok(b) => b,
        Err(e) => {
            app.status_message = Some(StatusMessage::error(format!(
                "Failed to get default branch: {}",
                e
            )));
            return Ok(());
        }
    };

    // Show confirmation
    let message = format!("Merge '{}' into '{}'?", branch_name, default_branch);
    if !show_confirm_overlay(terminal, app, &message)? {
        return Ok(());
    }

    // Show progress
    app.status_message = Some(StatusMessage::info(format!("Merging '{}'...", branch_name)));
    terminal.draw(|frame| app.render(frame))?;

    // Perform the merge
    if let Err(e) = git::merge_branch_to_default(&project.root_expanded(), branch_name) {
        app.status_message = Some(StatusMessage::error(format!("Merge failed: {}", e)));
        return Ok(());
    }

    // Ask if user wants to delete the worktree
    let delete_msg = format!("Delete worktree '{}' and its session?", branch_name);
    if show_confirm_overlay(terminal, app, &delete_msg)? {
        delete_worktree_internal(terminal, app, &project, branch_name)?;
    } else {
        app.status_message = Some(StatusMessage::info(format!(
            "Merged '{}' into '{}'",
            branch_name, default_branch
        )));
        app.refresh(Some(project_name))?;
    }

    Ok(())
}

/// Handle delete worktree operation with confirmation
fn handle_delete_worktree(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut TreeViewApp,
    project_name: &str,
    branch_name: &str,
) -> Result<()> {
    let project = match Project::load(project_name) {
        Ok(p) => p,
        Err(e) => {
            app.status_message = Some(StatusMessage::error(format!(
                "Failed to load project: {}",
                e
            )));
            return Ok(());
        }
    };

    // Show confirmation
    let message = format!(
        "Delete worktree '{}' for project '{}'?",
        branch_name, project_name
    );
    if !show_confirm_overlay(terminal, app, &message)? {
        return Ok(());
    }

    delete_worktree_internal(terminal, app, &project, branch_name)
}

/// Internal helper to delete a worktree with progress feedback
fn delete_worktree_internal(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut TreeViewApp,
    project: &Project,
    branch_name: &str,
) -> Result<()> {
    let session_name = project.worktree_session_name(branch_name);
    let current = CurrentContext::from_env();

    // Check if we're deleting the current session
    let is_current = current.is_current_worktree(&project.name, branch_name);

    // Show progress
    app.status_message = Some(StatusMessage::info(format!(
        "Deleting '{}'...",
        branch_name
    )));
    terminal.draw(|frame| app.render(frame))?;

    // Kill the tmux session if running
    if tmux::session_exists(&session_name).unwrap_or(false) {
        if let Err(e) = tmux::safe_kill_session(&session_name) {
            app.status_message = Some(StatusMessage::error(format!(
                "Failed to kill session: {}",
                e
            )));
            return Ok(());
        }
    }

    // Delete the worktree
    if let Err(e) = git::delete_worktree(project, branch_name) {
        app.status_message = Some(StatusMessage::error(format!(
            "Failed to delete worktree: {}",
            e
        )));
        return Ok(());
    }

    // If we deleted the current session, switch to the project session on exit
    if is_current {
        app.switch_to_session = Some(project.name.clone());
        app.status_message = Some(StatusMessage::info(format!(
            "Deleted '{}'. Will switch to '{}' on exit.",
            branch_name, project.name
        )));
    } else {
        app.status_message = Some(StatusMessage::info(format!(
            "Deleted worktree '{}'",
            branch_name
        )));
    }

    // Refresh the tree view
    app.refresh(Some(&project.name))?;

    Ok(())
}

/// Show an input overlay and return the entered text (None if cancelled)
fn show_input_overlay(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut TreeViewApp,
    title: &str,
    placeholder: &str,
) -> Result<Option<String>> {
    let mut value = String::new();

    loop {
        terminal.draw(|frame| {
            // Render the tree view in the background
            app.render(frame);
            // Render input dialog on top
            render_input_dialog(frame, title, placeholder, &value);
        })?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Esc => return Ok(None),
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            return Ok(None)
                        }
                        KeyCode::Enter => return Ok(Some(value)),
                        KeyCode::Backspace => {
                            value.pop();
                        }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            value.push(c);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

/// Render a centered input dialog
fn render_input_dialog(frame: &mut Frame, title: &str, placeholder: &str, value: &str) {
    use ratatui::widgets::Clear;

    let area = frame.size();

    // Center the dialog
    let dialog_width = 50.min(area.width - 4);
    let dialog_height = 5;
    let dialog_x = (area.width.saturating_sub(dialog_width)) / 2;
    let dialog_y = (area.height.saturating_sub(dialog_height)) / 2;

    let dialog_area = Rect::new(dialog_x, dialog_y, dialog_width, dialog_height);

    // Clear background
    frame.render_widget(Clear, dialog_area);

    // Dialog box
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::LightMagenta))
        .title(format!(" {} ", title))
        .title_style(Style::default().fg(Color::LightCyan).bold());

    let inner = block.inner(dialog_area);
    frame.render_widget(block, dialog_area);

    // Input text
    let input_area = Rect::new(inner.x + 1, inner.y + 1, inner.width - 2, 1);
    let input_text = if value.is_empty() {
        Line::from(vec![
            Span::styled(placeholder, Style::default().fg(Color::DarkGray).italic()),
            Span::styled("_", Style::default().fg(Color::LightMagenta)),
        ])
    } else {
        Line::from(vec![
            Span::styled(value, Style::default().fg(Color::White)),
            Span::styled("_", Style::default().fg(Color::LightMagenta)),
        ])
    };
    let input_widget = Paragraph::new(input_text);
    frame.render_widget(input_widget, input_area);

    // Help text
    let help_area = Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1);
    let help = Paragraph::new("Enter to confirm, Esc to cancel")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(help, help_area);
}

/// Show a confirmation overlay and return true if user confirmed
fn show_confirm_overlay(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut TreeViewApp,
    message: &str,
) -> Result<bool> {
    let mut selected = false; // false = No (default), true = Yes

    loop {
        terminal.draw(|frame| {
            // Render the tree view in the background
            app.render(frame);
            // Render confirmation dialog on top
            render_confirm_dialog(frame, message, selected);
        })?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => return Ok(true),
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => return Ok(false),
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            return Ok(false)
                        }
                        KeyCode::Left => selected = true,
                        KeyCode::Right => selected = false,
                        KeyCode::Tab => selected = !selected,
                        KeyCode::Enter => return Ok(selected),
                        _ => {}
                    }
                }
            }
        }
    }
}

/// Render a centered confirmation dialog
fn render_confirm_dialog(frame: &mut Frame, title: &str, selected_yes: bool) {
    use ratatui::widgets::Clear;

    let area = frame.size();

    // Center the dialog
    let dialog_width = (title.len() as u16 + 8).max(30).min(area.width - 4);
    let dialog_height = 7;
    let dialog_x = (area.width.saturating_sub(dialog_width)) / 2;
    let dialog_y = (area.height.saturating_sub(dialog_height)) / 2;

    let dialog_area = Rect::new(dialog_x, dialog_y, dialog_width, dialog_height);

    // Clear background
    frame.render_widget(Clear, dialog_area);

    // Dialog box
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::LightYellow))
        .title(" Confirm ")
        .title_style(Style::default().fg(Color::LightCyan).bold());

    let inner = block.inner(dialog_area);
    frame.render_widget(block, dialog_area);

    // Title text
    let title_area = Rect::new(inner.x, inner.y + 1, inner.width, 1);
    let title_widget = Paragraph::new(title)
        .style(Style::default().fg(Color::White))
        .alignment(Alignment::Center);
    frame.render_widget(title_widget, title_area);

    // Buttons
    let buttons_area = Rect::new(inner.x, inner.y + 3, inner.width, 1);

    let yes_style = if selected_yes {
        Style::default()
            .fg(Color::Black)
            .bg(Color::LightGreen)
            .bold()
    } else {
        Style::default().fg(Color::LightGreen)
    };

    let no_style = if !selected_yes {
        Style::default().fg(Color::Black).bg(Color::LightRed).bold()
    } else {
        Style::default().fg(Color::LightRed)
    };

    let buttons = Line::from(vec![
        Span::raw("        "),
        Span::styled(" Yes ", yes_style),
        Span::raw("   "),
        Span::styled(" No ", no_style),
        Span::raw("        "),
    ]);

    let buttons_widget = Paragraph::new(buttons).alignment(Alignment::Center);
    frame.render_widget(buttons_widget, buttons_area);

    // Help text
    let help_area = Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1);
    let help = Paragraph::new("y/n or Enter to confirm")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(help, help_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_candidates() {
        let projects = vec![
            ProjectData {
                name: "proj-a".to_string(),
                worktrees: vec![
                    WorktreeInfo {
                        path: "/tmp/a/main".into(),
                        branch: "main".to_string(),
                    },
                    WorktreeInfo {
                        path: "/tmp/a/feat".into(),
                        branch: "feature-x".to_string(),
                    },
                ],
                session_running: false,
            },
            ProjectData {
                name: "proj-b".to_string(),
                worktrees: vec![],
                session_running: true,
            },
        ];

        let candidates = build_candidates(&projects);

        // 2 projects + 2 worktrees = 4 candidates
        assert_eq!(candidates.len(), 4);

        // Check project candidate
        assert_eq!(candidates[0].label, "proj-a");
        assert_eq!(
            candidates[0].node_path,
            vec![TreeNodeId::Project("proj-a".to_string())]
        );

        // Check worktree candidate includes project name
        assert_eq!(candidates[1].label, "proj-a / main");
        assert_eq!(candidates[1].project, "proj-a");
    }

    #[test]
    fn test_tree_node_id_equality() {
        let a = TreeNodeId::Project("test".to_string());
        let b = TreeNodeId::Project("test".to_string());
        let c = TreeNodeId::Project("other".to_string());

        assert_eq!(a, b);
        assert_ne!(a, c);

        let wt1 = TreeNodeId::Worktree {
            project: "proj".to_string(),
            branch: "main".to_string(),
        };
        let wt2 = TreeNodeId::Worktree {
            project: "proj".to_string(),
            branch: "main".to_string(),
        };
        let wt3 = TreeNodeId::Worktree {
            project: "proj".to_string(),
            branch: "dev".to_string(),
        };

        assert_eq!(wt1, wt2);
        assert_ne!(wt1, wt3);
    }
}
