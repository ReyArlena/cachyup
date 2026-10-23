//! Past runs on the left, the selected run's full output on the right.

use std::path::PathBuf;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::engine::RunRecord;
use crate::engine::record::LOG_FILE;

use super::Effect;
use super::input::{Action, Hit, HitMap};
use super::theme;
use super::widgets::{Modal, OutputView, clamp_scroll, pane_block};

const PANE_RUNS: usize = 0;
const PANE_LOG: usize = 1;

pub struct HistoryScreen {
    runs: Vec<(PathBuf, RunRecord)>,
    cursor: usize,
    offset: usize,
    focus: usize,
    log: OutputView,
    /// Which run `log` was read for.
    loaded: Option<PathBuf>,
}

impl HistoryScreen {
    pub fn new() -> Self {
        Self {
            runs: Vec::new(),
            cursor: 0,
            offset: 0,
            focus: PANE_RUNS,
            log: OutputView::new(),
            loaded: None,
        }
    }

    pub fn refresh(&mut self, runs_dir: &std::path::Path) {
        self.runs = RunRecord::list(runs_dir);
        self.cursor = self.cursor.min(self.runs.len().saturating_sub(1));
        self.loaded = None;
        self.focus = PANE_RUNS;
        self.load();
    }

    fn load(&mut self) {
        let Some((dir, _)) = self.runs.get(self.cursor) else {
            self.log = OutputView::new();
            return;
        };
        if self.loaded.as_ref() == Some(dir) {
            return;
        }
        let text = std::fs::read_to_string(dir.join(LOG_FILE))
            .unwrap_or_else(|e| format!("Could not read {}: {e}", dir.join(LOG_FILE).display()));
        self.log = OutputView::from_log(&text);
        self.loaded = Some(dir.clone());
    }

    /// After `x` was confirmed.
    pub fn delete_current(&mut self, runs_dir: &std::path::Path) -> Vec<Effect> {
        let Some((dir, _)) = self.runs.get(self.cursor) else {
            return Vec::new();
        };
        let result = std::fs::remove_dir_all(dir);
        self.refresh(runs_dir);
        vec![Effect::Toast(match result {
            Ok(()) => "run deleted".into(),
            Err(e) => format!("could not delete: {e}"),
        })]
    }

    pub fn handle(&mut self, action: Action, hits: &HitMap) -> Vec<Effect> {
        let action = match action {
            Action::Click { x, y } => {
                match hits.find(x, y) {
                    Some(Hit::Row {
                        pane: PANE_RUNS,
                        index,
                    }) => {
                        self.focus = PANE_RUNS;
                        self.cursor = *index;
                        self.load();
                    }
                    Some(Hit::Pane(p)) => self.focus = *p,
                    _ => {}
                }
                return Vec::new();
            }
            Action::Scroll { x, y, up } => {
                if hits.pane_at(x, y) == Some(PANE_LOG) {
                    self.log.scroll_by(if up { -3 } else { 3 });
                    return Vec::new();
                }
                if up { Action::Up } else { Action::Down }
            }
            other => other,
        };
        match action {
            Action::Back | Action::Char('H') => {
                if self.focus == PANE_LOG && action == Action::Back {
                    self.focus = PANE_RUNS;
                    return Vec::new();
                }
                return vec![Effect::CloseHistory];
            }
            Action::NextPane | Action::PrevPane => {
                self.focus = 1 - self.focus;
                return Vec::new();
            }
            Action::Char('x') if !self.runs.is_empty() => {
                let id = self.runs[self.cursor].1.id.clone();
                return vec![Effect::Modal(Modal::confirm(
                    "Delete run",
                    &format!("Delete the log and record of the run from {id}?"),
                    "delete-run",
                ))];
            }
            _ => {}
        }
        if self.focus == PANE_RUNS {
            let len = self.runs.len();
            match action {
                Action::Up => self.cursor = self.cursor.saturating_sub(1),
                Action::Down => self.cursor = (self.cursor + 1).min(len.saturating_sub(1)),
                Action::PageUp => self.cursor = self.cursor.saturating_sub(10),
                Action::PageDown => self.cursor = (self.cursor + 10).min(len.saturating_sub(1)),
                Action::Home => self.cursor = 0,
                Action::End => self.cursor = len.saturating_sub(1),
                Action::Confirm | Action::Right => self.focus = PANE_LOG,
                _ => {}
            }
            self.load();
        } else {
            match action {
                Action::Up => self.log.scroll_by(-1),
                Action::Down => self.log.scroll_by(1),
                Action::PageUp => self.log.page(false),
                Action::PageDown => self.log.page(true),
                Action::Home => self.log.scroll_to_start(),
                Action::End => self.log.scroll_to_end(),
                Action::Left => self.focus = PANE_RUNS,
                _ => {}
            }
        }
        Vec::new()
    }

    pub fn key_hints(&self) -> Vec<(&'static str, &'static str)> {
        let mut hints = vec![("Tab", "switch pane")];
        if self.focus == PANE_RUNS {
            hints.push(("Enter", "read log"));
        } else {
            hints.push(("G", "end"));
        }
        hints.extend([
            ("x", "delete"),
            ("Esc", "back"),
            ("?", "help"),
            ("q", "quit"),
        ]);
        hints
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, hits: &mut HitMap) {
        let [left, right] =
            Layout::horizontal([Constraint::Length(46), Constraint::Min(20)]).areas(area);

        let focused = self.focus == PANE_RUNS;
        let block = pane_block("History", focused);
        let inner = block.inner(left);
        frame.render_widget(block, left);
        hits.add(left, Hit::Pane(PANE_RUNS));
        let height = inner.height as usize;
        self.offset = clamp_scroll(self.cursor, self.runs.len(), height, self.offset);
        let mut lines = Vec::new();
        for (i, (_, run)) in self.runs.iter().enumerate().skip(self.offset).take(height) {
            let c = run.counts();
            let mut spans = vec![Span::raw(format!(
                " {} ",
                run.started.format("%Y-%m-%d %H:%M")
            ))];
            if run.dry_run {
                spans.push(Span::styled(" DRY ", theme::badge_warn()));
                spans.push(Span::raw(" "));
            }
            if run.finished.is_none() {
                spans.push(Span::styled(
                    "unfinished ",
                    theme::status(crate::engine::Status::Failed),
                ));
            } else if run.ended_early.is_some() {
                spans.push(Span::styled(
                    "ended early ",
                    theme::status(crate::engine::Status::Attention),
                ));
            }
            let done = if run.dry_run { c.would } else { c.succeeded };
            spans.push(Span::styled(format!("✓{done} "), theme::on()));
            if c.failed > 0 {
                spans.push(Span::styled(
                    format!("✗{} ", c.failed),
                    theme::status(crate::engine::Status::Failed),
                ));
            }
            if c.attention > 0 {
                spans.push(Span::styled(
                    format!("!{}", c.attention),
                    theme::status(crate::engine::Status::Attention),
                ));
            }
            let mut line = Line::from(spans);
            if i == self.cursor {
                line = line.style(if focused {
                    theme::selected()
                } else {
                    theme::selected_inactive()
                });
            }
            let y = inner.y + (i - self.offset) as u16;
            hits.add(
                Rect::new(inner.x, y, inner.width, 1),
                Hit::Row {
                    pane: PANE_RUNS,
                    index: i,
                },
            );
            lines.push(line);
        }
        if self.runs.is_empty() {
            lines.push(Line::from(Span::styled(" No runs yet", theme::dim())));
        }
        frame.render_widget(Paragraph::new(lines), inner);

        hits.add(right, Hit::Pane(PANE_LOG));
        let title = match self.runs.get(self.cursor) {
            Some((_, r)) => format!("{} output", r.id),
            None => "Output".into(),
        };
        self.log
            .draw(frame, right, pane_block(&title, self.focus == PANE_LOG));
    }
}
