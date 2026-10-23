//! A run in progress (or just finished): the step list ticking off on the
//! left, the full terminal output on the right.

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::engine::{Event, Outcome, Plan, RunRecord, Status};

use super::Effect;
use super::input::{Action, Hit, HitMap};
use super::theme;
use super::widgets::{OutKind, OutputView, clamp_scroll, pane_block};

const PANE_STEPS: usize = 0;
const PANE_OUTPUT: usize = 1;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

enum Row {
    Header(String),
    Step(usize),
}

struct StepView {
    name: String,
    outcome: Option<Outcome>,
    /// Output line where the step began.
    line: Option<usize>,
}

pub struct RunScreen {
    pub dry: bool,
    steps: Vec<StepView>,
    rows: Vec<Row>,
    current: Option<usize>,
    pub output: OutputView,
    focus: usize,
    /// Cursor over `rows`; follows the running step until the user moves it.
    cursor: usize,
    cursor_follows: bool,
    offset: usize,
    pub finished: Option<Box<RunRecord>>,
    pub stopping: bool,
    started: Instant,
}

impl RunScreen {
    pub fn new(plan: &Plan, dry: bool) -> Self {
        let mut rows = Vec::new();
        let mut steps = Vec::new();
        let mut group: Option<&str> = None;
        for step in plan.visible() {
            if group != Some(step.group.as_str()) {
                rows.push(Row::Header(step.group.clone()));
                group = Some(&step.group);
            }
            rows.push(Row::Step(steps.len()));
            steps.push(StepView {
                name: step.name.clone(),
                outcome: None,
                line: None,
            });
        }
        Self {
            dry,
            steps,
            rows,
            current: None,
            output: OutputView::new(),
            focus: PANE_OUTPUT,
            cursor: 0,
            cursor_follows: true,
            offset: 0,
            finished: None,
            stopping: false,
            started: Instant::now(),
        }
    }

    pub fn empty() -> Self {
        Self::new(
            &Plan {
                steps: Vec::new(),
                tasks: Vec::new(),
            },
            true,
        )
    }

    pub fn running(&self) -> bool {
        self.finished.is_none()
    }

    pub fn apply(&mut self, event: Event) {
        match event {
            Event::StepStarted(i) => {
                self.current = Some(i);
                if let Some(s) = self.steps.get_mut(i) {
                    s.line = Some(self.output.lines.len());
                }
                if self.cursor_follows
                    && let Some(r) = self
                        .rows
                        .iter()
                        .position(|r| matches!(r, Row::Step(s) if *s == i))
                {
                    self.cursor = r;
                }
            }
            Event::StepFinished(i, outcome) => {
                if let Some(s) = self.steps.get_mut(i) {
                    s.outcome = Some(outcome);
                }
            }
            Event::Line(l) => self.output.push(OutKind::Plain, l),
            Event::Note(n) => self.output.push(OutKind::Note, format!("    {n}")),
            Event::Heading(h) => self.output.push(OutKind::Heading, format!("==> {h}")),
            Event::Status { text, status } => self.output.push(OutKind::Status(status), text),
            Event::Partial(p) => self.output.partial = Some(p),
            Event::Finished(record) => {
                self.current = None;
                for (view, rec) in self.steps.iter_mut().zip(&record.steps) {
                    if view.outcome.is_none() {
                        view.outcome = rec.outcome.clone();
                    }
                }
                self.finished = Some(record);
                self.stopping = false;
            }
        }
    }

    pub fn handle(&mut self, action: Action, hits: &HitMap) -> Vec<Effect> {
        let action = match action {
            Action::Click { x, y } => {
                match hits.find(x, y) {
                    Some(Hit::Row {
                        pane: PANE_STEPS,
                        index,
                    }) => {
                        self.focus = PANE_STEPS;
                        self.cursor = *index;
                        self.cursor_follows = false;
                        self.jump_to_cursor();
                    }
                    Some(Hit::Pane(p)) => self.focus = *p,
                    _ => {}
                }
                return Vec::new();
            }
            Action::Scroll { x, y, up } => {
                let pane = hits.pane_at(x, y).unwrap_or(self.focus);
                if pane == PANE_OUTPUT {
                    self.output.scroll_by(if up { -3 } else { 3 });
                    return Vec::new();
                }
                self.focus = pane;
                if up { Action::Up } else { Action::Down }
            }
            other => other,
        };
        match action {
            Action::Quit | Action::Back if self.running() => return vec![Effect::AskStop],
            Action::Quit | Action::Back => return vec![Effect::CloseRun],
            Action::Char('H') if !self.running() => return vec![Effect::History],
            Action::NextPane | Action::PrevPane => self.focus = 1 - self.focus,
            Action::Left => self.focus = PANE_STEPS,
            Action::Right => self.focus = PANE_OUTPUT,
            _ => {}
        }
        if self.focus == PANE_OUTPUT {
            match action {
                Action::Up => self.output.scroll_by(-1),
                Action::Down => self.output.scroll_by(1),
                Action::PageUp => self.output.page(false),
                Action::PageDown => self.output.page(true),
                Action::Home => self.output.scroll_to_start(),
                Action::End => self.output.scroll_to_end(),
                _ => {}
            }
        } else {
            let before = self.cursor;
            match action {
                Action::Up => self.move_cursor(-1),
                Action::Down => self.move_cursor(1),
                Action::PageUp => self.move_cursor(-10),
                Action::PageDown => self.move_cursor(10),
                Action::Home => self.move_cursor(-(self.rows.len() as isize)),
                Action::End => {
                    self.move_cursor(self.rows.len() as isize);
                    self.cursor_follows = true;
                }
                Action::Confirm => {
                    self.jump_to_cursor();
                    self.focus = PANE_OUTPUT;
                }
                _ => {}
            }
            if self.cursor != before {
                self.jump_to_cursor();
            }
        }
        Vec::new()
    }

    fn move_cursor(&mut self, delta: isize) {
        let len = self.rows.len() as isize;
        if len == 0 {
            return;
        }
        self.cursor = (self.cursor as isize + delta).clamp(0, len - 1) as usize;
        self.cursor_follows = false;
    }

    /// Scroll the output to where the step under the cursor began.
    fn jump_to_cursor(&mut self) {
        let step = match self.rows.get(self.cursor) {
            Some(Row::Step(i)) => Some(*i),
            Some(Row::Header(_)) => match self.rows.get(self.cursor + 1) {
                Some(Row::Step(i)) => Some(*i),
                _ => None,
            },
            None => None,
        };
        if let Some(line) = step.and_then(|i| self.steps[i].line) {
            // Include the heading above the first step of a group.
            self.output.show_line(line.saturating_sub(2));
        }
    }

    pub fn key_hints(&self) -> Vec<(&'static str, &'static str)> {
        let mut hints = vec![("Tab", "switch pane")];
        if self.focus == PANE_STEPS {
            hints.push(("Enter", "show output"));
        } else {
            hints.push(("G", "follow"));
        }
        if self.running() {
            hints.push(("q", "stop"));
        } else {
            hints.extend([("Esc", "back"), ("H", "history")]);
        }
        hints.push(("?", "help"));
        hints
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, hits: &mut HitMap) {
        let [left, right] =
            Layout::horizontal([Constraint::Length(46), Constraint::Min(20)]).areas(area);
        self.draw_steps(frame, left, hits);
        hits.add(right, Hit::Pane(PANE_OUTPUT));
        let title = if self.output.follow || !self.running() {
            "Output".to_string()
        } else {
            "Output (paused, G to follow)".to_string()
        };
        self.output
            .draw(frame, right, pane_block(&title, self.focus == PANE_OUTPUT));
    }

    fn draw_steps(&mut self, frame: &mut Frame, area: Rect, hits: &mut HitMap) {
        let focused = self.focus == PANE_STEPS;
        let block = pane_block(if self.dry { "Dry run" } else { "Run" }, focused);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        hits.add(area, Hit::Pane(PANE_STEPS));

        let [head, list] =
            Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(inner);
        let done = self.steps.iter().filter(|s| s.outcome.is_some()).count();
        let total = self.steps.len().max(1);
        let bar_width = (head.width as usize).saturating_sub(14).max(4);
        let filled = bar_width * done / total;
        let elapsed = self.started.elapsed().as_secs();
        let state = match (&self.finished, self.stopping) {
            (Some(r), _) if r.ended_early.is_some() => {
                Span::styled("Ended early", theme::status(Status::Failed))
            }
            (Some(r), _) if r.counts().failed > 0 => {
                Span::styled("Finished with failures", theme::status(Status::Failed))
            }
            (Some(_), _) => Span::styled("Finished", theme::status(Status::Success)),
            (None, true) => Span::styled(
                "Stopping after this step...",
                theme::status(Status::Attention),
            ),
            (None, false) => Span::styled("Running", theme::title()),
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    state,
                    Span::styled(
                        format!("  {}:{:02}", elapsed / 60, elapsed % 60),
                        theme::dim(),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("█".repeat(filled), theme::on()),
                    Span::styled("░".repeat(bar_width - filled), theme::dim()),
                    Span::raw(format!(" {done}/{}", self.steps.len())),
                ]),
            ]),
            head,
        );

        let height = list.height as usize;
        self.offset = clamp_scroll(self.cursor, self.rows.len(), height, self.offset);
        let tick = (self.started.elapsed().as_millis() / 100) as usize % SPINNER.len();
        let mut lines = Vec::new();
        for (i, row) in self.rows.iter().enumerate().skip(self.offset).take(height) {
            let mut line = match row {
                Row::Header(h) => Line::from(Span::styled(h.clone(), theme::heading())),
                Row::Step(s) => {
                    let step = &self.steps[*s];
                    let (mark, style) = match (&step.outcome, self.current == Some(*s)) {
                        (Some(o), _) => (theme::status_mark(o.status), theme::status(o.status)),
                        (None, true) => (SPINNER[tick], theme::title()),
                        (None, false) => ("·", theme::dim()),
                    };
                    let mut spans = vec![
                        Span::styled(format!(" {mark} "), style),
                        Span::styled(
                            step.name.clone(),
                            if step.outcome.is_none() && self.current != Some(*s) {
                                theme::dim()
                            } else {
                                ratatui::style::Style::default()
                            },
                        ),
                    ];
                    if let Some(reason) = step.outcome.as_ref().and_then(|o| o.reason.clone()) {
                        spans.push(Span::styled(format!("  {reason}"), theme::dim()));
                    }
                    Line::from(spans)
                }
            };
            if i == self.cursor && (focused || !self.cursor_follows) {
                line = line.style(if focused {
                    theme::selected()
                } else {
                    theme::selected_inactive()
                });
            }
            let y = list.y + (i - self.offset) as u16;
            hits.add(
                Rect::new(list.x, y, list.width, 1),
                Hit::Row {
                    pane: PANE_STEPS,
                    index: i,
                },
            );
            lines.push(line);
        }
        frame.render_widget(Paragraph::new(lines), list);
    }
}
