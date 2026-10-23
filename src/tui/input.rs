//! Translate raw terminal events into app actions, and map mouse clicks back
//! onto whatever was drawn.
//!
//! Arrows and j/k only ever move. `h` opens the history, so h/l are not
//! vim-style left/right here.

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    /// Tab: the other pane.
    NextPane,
    PrevPane,
    Toggle,
    Confirm,
    Back,
    Quit,
    Help,
    /// A letter not covered above.
    Char(char),
    Click {
        x: u16,
        y: u16,
    },
    Scroll {
        x: u16,
        y: u16,
        up: bool,
    },
    Resize,
}

pub fn map_event(event: Event) -> Option<Action> {
    match event {
        Event::Key(key) => map_key(key),
        Event::Mouse(mouse) => map_mouse(mouse),
        Event::Resize(_, _) => Some(Action::Resize),
        _ => None,
    }
}

pub fn map_key(key: KeyEvent) -> Option<Action> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(Action::Quit);
    }
    Some(match key.code {
        KeyCode::Up | KeyCode::Char('k') => Action::Up,
        KeyCode::Down | KeyCode::Char('j') => Action::Down,
        KeyCode::Left => Action::Left,
        KeyCode::Right => Action::Right,
        KeyCode::PageUp => Action::PageUp,
        KeyCode::PageDown => Action::PageDown,
        KeyCode::Home | KeyCode::Char('g') => Action::Home,
        KeyCode::End | KeyCode::Char('G') => Action::End,
        KeyCode::Tab => Action::NextPane,
        KeyCode::BackTab => Action::PrevPane,
        KeyCode::Char(' ') => Action::Toggle,
        KeyCode::Enter => Action::Confirm,
        KeyCode::Esc => Action::Back,
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('?') => Action::Help,
        KeyCode::Char(c) => Action::Char(c),
        _ => return None,
    })
}

fn map_mouse(mouse: MouseEvent) -> Option<Action> {
    let (x, y) = (mouse.column, mouse.row);
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => Some(Action::Click { x, y }),
        MouseEventKind::ScrollUp => Some(Action::Scroll { x, y, up: true }),
        MouseEventKind::ScrollDown => Some(Action::Scroll { x, y, up: false }),
        _ => None,
    }
}

/// Clickable regions registered while drawing. Rebuilt every frame.
#[derive(Debug, Default, Clone)]
pub struct HitMap {
    regions: Vec<(Rect, Hit)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Hit {
    /// Row `index` of pane `pane`.
    Row { pane: usize, index: usize },
    /// A key hint: clicking it is the same as pressing the key.
    Key(char),
    /// Anywhere in pane `pane`.
    Pane(usize),
}

impl HitMap {
    pub fn clear(&mut self) {
        self.regions.clear();
    }

    pub fn add(&mut self, area: Rect, hit: Hit) {
        self.regions.push((area, hit));
    }

    /// The smallest region under a cell.
    pub fn find(&self, x: u16, y: u16) -> Option<&Hit> {
        self.regions
            .iter()
            .filter(|(r, _)| contains(r, x, y))
            .min_by_key(|(r, _)| r.width as u32 * r.height as u32)
            .map(|(_, h)| h)
    }

    pub fn pane_at(&self, x: u16, y: u16) -> Option<usize> {
        self.regions
            .iter()
            .filter(|(r, h)| matches!(h, Hit::Pane(_)) && contains(r, x, y))
            .min_by_key(|(r, _)| r.width as u32 * r.height as u32)
            .and_then(|(_, h)| match h {
                Hit::Pane(p) => Some(*p),
                _ => None,
            })
    }
}

fn contains(r: &Rect, x: u16, y: u16) -> bool {
    x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h_is_a_letter_not_navigation() {
        let key = |c| map_key(KeyEvent::new(c, KeyModifiers::NONE)).unwrap();
        assert_eq!(key(KeyCode::Char('h')), Action::Char('h'));
        assert_eq!(key(KeyCode::Char('j')), Action::Down);
        assert_eq!(key(KeyCode::Char(' ')), Action::Toggle);
    }

    #[test]
    fn smallest_region_wins() {
        let mut map = HitMap::default();
        map.add(Rect::new(0, 0, 40, 20), Hit::Pane(0));
        map.add(Rect::new(2, 3, 30, 1), Hit::Row { pane: 0, index: 2 });
        assert_eq!(map.find(5, 3), Some(&Hit::Row { pane: 0, index: 2 }));
        assert_eq!(map.find(5, 10), Some(&Hit::Pane(0)));
        assert_eq!(map.pane_at(5, 3), Some(0));
    }
}
