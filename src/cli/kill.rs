//! Kill a tmux session with Ratatui confirmation for worktrees.

use std::io::{stdout, IsTerminal};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::cli::tree_view::{self, SelectedAction};
use crate::config::Project;
use crate::git;
use crate::tmux;

pub fn run(session_name: Option<String>) -> Result<()> {
    // Use tree view to select session
    let action = tree_view::run_for_kill(session_name)?;

    let (project_name, branch) = match action {
        Some(SelectedAction::KillProject(name)) => (name, None),
        Some(SelectedAction::KillWorktree { project, branch }) => (project, Some(branch)),
        _ => return Ok(()), // User quit or unexpected action
    };

    let session_name = match &branch {
        Some(b) => format!("{}__{}", project_name, b),
        None => project_name.clone(),
    };

    // Check if session exists
    if !tmux::session_exists(&session_name)? {
        anyhow::bail!("Session '{}' is not running", session_name);
    }

    // Show confirmation
    let is_worktree = branch.is_some();
    let confirm_title = if let Some(ref b) = branch {
        format!("Kill worktree session '{}' ({})?", b, project_name)
    } else {
        format!("Kill session '{}'?", session_name)
    };

    if !confirm_dialog(&confirm_title, is_worktree)? {
        println!("Cancelled.");
        return Ok(());
    }

    // If it's a worktree, also offer to delete the worktree itself
    let delete_worktree = if is_worktree {
        let delete_title = format!(
            "Also delete worktree '{}'?",
            branch.as_deref().unwrap_or("")
        );
        confirm_dialog(&delete_title, true)?
    } else {
        false
    };

    // Kill the session
    tmux::kill_session(&session_name)?;
    println!("Killed session: {}", session_name);

    // Delete worktree if confirmed
    if delete_worktree {
        if let Some(ref b) = branch {
            let project = Project::load(&project_name)?;
            git::delete_worktree(&project, b)?;
            println!("Deleted worktree: {}", b);
        }
    }

    Ok(())
}

/// Run a styled confirmation dialog
fn confirm_dialog(title: &str, is_warning: bool) -> Result<bool> {
    if !stdout().is_terminal() {
        // Non-interactive: default to yes
        return Ok(true);
    }

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let result = run_confirm_loop(&mut terminal, title, is_warning);

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    result
}

fn run_confirm_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    title: &str,
    is_warning: bool,
) -> Result<bool> {
    let mut selected = false; // false = No (default), true = Yes

    loop {
        terminal.draw(|frame| {
            render_confirm_dialog(frame, title, selected, is_warning);
        })?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => return Ok(true),
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => return Ok(false),
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            return Ok(false)
                        }
                        KeyCode::Left | KeyCode::Char('h') => selected = true,
                        KeyCode::Right | KeyCode::Char('l') => selected = false,
                        KeyCode::Tab => selected = !selected,
                        KeyCode::Enter => return Ok(selected),
                        _ => {}
                    }
                }
            }
        }
    }
}

fn render_confirm_dialog(frame: &mut Frame, title: &str, selected_yes: bool, is_warning: bool) {
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
    let border_color = if is_warning {
        Color::LightYellow
    } else {
        Color::LightMagenta
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
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
