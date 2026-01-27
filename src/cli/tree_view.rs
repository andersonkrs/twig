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
use crate::tmux;

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

/// App state for the tree view
struct TreeViewApp<'a> {
    tree_items: Vec<TreeItem<'a, TreeNodeId>>,
    tree_state: TreeState<TreeNodeId>,
    candidates: Vec<SearchCandidate>,
    query: String,
    last_typed: Instant,
    timeout: Duration,
    no_match: bool,
    mode: TreeViewMode,
}

impl<'a> TreeViewApp<'a> {
    fn new(
        projects: Vec<ProjectData>,
        running_sessions: &[String],
        mode: TreeViewMode,
        current: &CurrentContext,
    ) -> Result<Self> {
        let tree_items = build_tree_items(&projects, running_sessions, current)?;
        let candidates = build_candidates(&projects);

        let mut tree_state = TreeState::default();

        // Open all projects by default and select first item
        for project in &projects {
            tree_state.open(vec![TreeNodeId::Project(project.name.clone())]);
        }
        if !projects.is_empty() {
            tree_state.select(vec![TreeNodeId::Project(projects[0].name.clone())]);
        }

        Ok(Self {
            tree_items,
            tree_state,
            candidates,
            query: String::new(),
            last_typed: Instant::now(),
            timeout: Duration::from_millis(800),
            no_match: false,
            mode,
        })
    }

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<HandleResult> {
        match code {
            KeyCode::Char('w') if modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(HandleResult::Quit);
            }
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(HandleResult::Quit);
            }
            KeyCode::Esc => {
                if !self.query.is_empty() {
                    self.query.clear();
                    self.no_match = false;
                } else {
                    return Some(HandleResult::Quit);
                }
            }

            // Kill session with Ctrl+K
            KeyCode::Char('k') if modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(action) = self.get_selected_action() {
                    // Convert to kill action regardless of mode
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

            // Navigation
            KeyCode::Up => {
                self.tree_state.key_up();
            }
            KeyCode::Char('p') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.tree_state.key_up();
            }
            KeyCode::Down => {
                self.tree_state.key_down();
            }
            KeyCode::Char('n') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.tree_state.key_down();
            }
            KeyCode::Left if self.query.is_empty() => {
                self.tree_state.key_left();
            }
            KeyCode::Right if self.query.is_empty() => {
                self.tree_state.key_right();
            }

            // Selection
            KeyCode::Enter => {
                if let Some(action) = self.get_selected_action() {
                    return Some(HandleResult::Action(action));
                }
            }

            // Typeahead search
            KeyCode::Backspace => {
                self.query.pop();
                if self.query.is_empty() {
                    self.no_match = false;
                } else {
                    self.do_fuzzy_search();
                }
            }
            KeyCode::Char(c) => {
                // Reset query if timeout elapsed
                if self.last_typed.elapsed() > self.timeout {
                    self.query.clear();
                }
                self.query.push(c);
                self.last_typed = Instant::now();
                self.do_fuzzy_search();
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
        let status_line = if !self.query.is_empty() {
            if self.no_match {
                Line::from(vec![
                    Span::styled("Search: ", Style::default().fg(Color::LightCyan)),
                    Span::styled(&self.query, Style::default().fg(Color::LightYellow).bold()),
                    Span::styled(" (no match)", Style::default().fg(Color::LightRed).italic()),
                ])
            } else {
                Line::from(vec![
                    Span::styled("Search: ", Style::default().fg(Color::LightCyan)),
                    Span::styled(&self.query, Style::default().fg(Color::LightGreen).bold()),
                ])
            }
        } else {
            let (action_text, separator_color) = match self.mode {
                TreeViewMode::Start => (" select ", Color::LightMagenta),
                TreeViewMode::Kill => (" kill ", Color::LightRed),
            };
            Line::from(vec![
                Span::styled("Type", Style::default().fg(Color::LightCyan)),
                Span::styled(" to search ", Style::default().fg(Color::Gray)),
                Span::styled("\u{2502} ", Style::default().fg(separator_color)),
                Span::styled("\u{2191}\u{2193}", Style::default().fg(Color::LightCyan)),
                Span::styled(" navigate ", Style::default().fg(Color::Gray)),
                Span::styled("\u{2502} ", Style::default().fg(separator_color)),
                Span::styled("Enter", Style::default().fg(Color::LightCyan)),
                Span::styled(action_text, Style::default().fg(Color::Gray)),
                Span::styled("\u{2502} ", Style::default().fg(separator_color)),
                Span::styled("^K", Style::default().fg(Color::LightCyan)),
                Span::styled(" kill ", Style::default().fg(Color::Gray)),
                Span::styled("\u{2502} ", Style::default().fg(separator_color)),
                Span::styled("^W", Style::default().fg(Color::LightCyan)),
                Span::styled(" quit", Style::default().fg(Color::Gray)),
            ])
        };

        let status = Paragraph::new(status_line);
        frame.render_widget(status, chunks[1]);
    }
}

enum HandleResult {
    Quit,
    Action(SelectedAction),
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

        // Build styled project text - bright colors for visibility
        let mut spans = vec![Span::styled(
            project.name.clone(),
            Style::default().fg(Color::LightYellow).bold(),
        )];

        if is_current {
            spans.push(Span::styled(
                " \u{25c0}",
                Style::default().fg(Color::LightMagenta),
            )); // ◀ current indicator
        }

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

                // Build styled worktree text - bright colors
                let mut wt_spans = vec![Span::styled(
                    wt.branch.clone(),
                    Style::default().fg(Color::LightCyan),
                )];

                if is_current_wt {
                    wt_spans.push(Span::styled(
                        " \u{25c0}",
                        Style::default().fg(Color::LightMagenta),
                    )); // ◀ current indicator
                }

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
pub fn run(project_filter: Option<String>) -> Result<Option<SelectedAction>> {
    run_with_options(
        LoadOptions {
            project_filter,
            running_only: false,
            include_worktrees: true,
        },
        TreeViewMode::Start,
    )
}

/// Run the interactive tree view for projects only (no worktrees)
pub fn run_projects_only() -> Result<Option<SelectedAction>> {
    run_with_options(
        LoadOptions {
            project_filter: None,
            running_only: false,
            include_worktrees: false,
        },
        TreeViewMode::Start,
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
    )
}

/// Run the interactive tree view with specified options
fn run_with_options(opts: LoadOptions, mode: TreeViewMode) -> Result<Option<SelectedAction>> {
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
    let mut app = TreeViewApp::new(projects, &running_sessions, mode, &current)?;

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
        terminal.draw(|frame| app.render(frame))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if let Some(result) = app.handle_key(key.code, key.modifiers) {
                        return match result {
                            HandleResult::Quit => Ok(None),
                            HandleResult::Action(action) => Ok(Some(action)),
                        };
                    }
                }
            }
        }
    }
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
