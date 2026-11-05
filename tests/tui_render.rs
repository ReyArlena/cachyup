//! Render the screens into an in-memory terminal. Catches layout panics (for
//! example on tiny terminals) and checks the main pieces show up, without a TTY.

use cachyup::app::AppState;
use cachyup::tui::App;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

/// A sandbox as a fresh install sees it: whatever this machine has
/// installed is up for review, so the "To review" popup opens first.
fn sandbox_with_review() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("temp dir");
    // No network calls to GitHub from tests.
    let config = dir.path().join("config/config.toml");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, "[settings]\nupdate_check = false\n").unwrap();
    let state = AppState::sandboxed(dir.path()).expect("sandboxed state");
    (dir, state)
}

/// The same with everything installed already reviewed, so no popup gets
/// in the way of the keys a test sends.
fn sandbox() -> (tempfile::TempDir, AppState) {
    let (dir, mut state) = sandbox_with_review();
    state.state.reviewed = state
        .installed_candidates()
        .iter()
        .map(|c| c.key())
        .collect();
    state.save_state().unwrap();
    (dir, state)
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn render(app: &mut App, w: u16, h: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| app.draw_for_test(f)).unwrap();
    let buffer = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            text.push_str(buffer[(x, y)].symbol());
        }
        text.push('\n');
    }
    if let Ok(dir) = std::env::var("CACHYUP_DUMP") {
        let n = std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);
        let _ = std::fs::write(format!("{dir}/{n:02}.txt"), &text);
    }
    text
}

#[test]
fn main_screen_renders_at_common_and_tiny_sizes() {
    let (_dir, state) = sandbox();
    let mut app = App::new(state, false, false);
    for (w, h) in [(140, 40), (80, 24), (20, 5), (1, 1)] {
        render(&mut app, w, h);
    }
    let text = render(&mut app, 120, 36);
    assert!(text.contains("Full System Update"));
    assert!(text.contains("Full System Update options"));
    assert!(text.contains("pacman -Syu"));
    assert!(text.contains("no sudo"));
}

#[test]
fn app_list_shows_packages_tweaks_and_status() {
    let (_dir, state) = sandbox();
    let mut app = App::new(state, false, false);
    app.key_for_test(key(KeyCode::Down));
    app.key_for_test(key(KeyCode::Char('c')));
    let text = render(&mut app, 120, 40);
    assert!(text.contains("Install Apps options"), "{text}");
    assert!(text.contains("Pacman packages"));
    assert!(text.contains("adw-gtk-theme"));

    // Filter down to discord: the app and its nested tweak.
    app.key_for_test(key(KeyCode::Char('/')));
    for c in "discord".chars() {
        app.key_for_test(key(KeyCode::Char(c)));
    }
    app.key_for_test(key(KeyCode::Enter));
    let text = render(&mut app, 120, 40);
    assert!(text.contains("discord"));
    assert!(text.contains("Disable host update"));
    assert!(!text.contains("adw-gtk-theme"));

    // Info popup.
    app.key_for_test(key(KeyCode::Char('i')));
    let text = render(&mut app, 120, 40);
    assert!(text.contains("Package"), "{text}");
    app.key_for_test(key(KeyCode::Esc));
    render(&mut app, 120, 40);
}

#[test]
fn run_modal_and_dry_run_complete() {
    let (dir, mut state) = sandbox();
    // Tweaks only: quick, and the tweak files live in the sandbox home.
    state.config.tasks.update = false;
    state.config.tasks.apps = false;
    state.save_config().unwrap();
    let mut app = App::new(state, false, false);
    app.key_for_test(key(KeyCode::Char('r')));
    let text = render(&mut app, 120, 36);
    assert!(text.contains("dry run"), "{text}");
    app.key_for_test(key(KeyCode::Esc));

    app.dry_run_for_test();
    assert_eq!(app.screen_name_for_test(), "run");
    let text = render(&mut app, 120, 36);
    assert!(text.contains("Dry run finished"), "{text}");
    assert!(text.contains("Preflight"));
    app.key_for_test(key(KeyCode::Enter));
    let text = render(&mut app, 120, 36);
    assert!(text.contains("Finished"), "{text}");
    assert!(text.contains("==> Summary") || text.contains("would run"));

    // History lists the run.
    app.key_for_test(key(KeyCode::Char('H')));
    let text = render(&mut app, 120, 36);
    assert_eq!(app.screen_name_for_test(), "history");
    assert!(text.contains("DRY"), "{text}");
    for (w, h) in [(60, 15), (2, 2)] {
        render(&mut app, w, h);
    }
    assert!(dir.path().join("state/runs").is_dir());
}

/// Render the app list against the real system and home directory. Opt in:
///   CACHYUP_DUMP=/tmp/d cargo test --test tui_render -- --ignored
#[test]
#[ignore]
fn dump_app_list_for_this_machine() {
    let state = AppState::load(cachyup::paths::Paths::from_env()).unwrap();
    let mut app = App::new(state, true, false);
    app.key_for_test(key(KeyCode::Down));
    app.key_for_test(key(KeyCode::Char('c')));
    app.key_for_test(key(KeyCode::Char('/')));
    for c in "d".chars() {
        app.key_for_test(key(KeyCode::Char(c)));
    }
    app.key_for_test(key(KeyCode::Enter));
    app.key_for_test(key(KeyCode::Char('I')));
    render(&mut app, 120, 30);
    app.key_for_test(key(KeyCode::Char('i')));
    render(&mut app, 120, 30);
}

#[test]
fn descriptions_show_under_each_entry() {
    let (_dir, state) = sandbox();
    let mut app = App::new(state, false, false);
    app.key_for_test(key(KeyCode::Down));
    app.key_for_test(key(KeyCode::Char('c')));
    app.key_for_test(key(KeyCode::Char('/')));
    for c in "discord".chars() {
        app.key_for_test(key(KeyCode::Char(c)));
    }
    app.key_for_test(key(KeyCode::Enter));
    app.key_for_test(key(KeyCode::Char('I')));
    let text = render(&mut app, 120, 30);
    // The tweak description starts in line with the tweak name above it.
    let lines: Vec<&str> = text.lines().collect();
    let name = lines
        .iter()
        .position(|l| l.contains("Disable host update"))
        .unwrap();
    let below = lines[name + 1];
    let col = |l: &str, s: &str| {
        l.chars()
            .collect::<String>()
            .find(s)
            .map(|b| l[..b].chars().count())
    };
    assert_eq!(
        col(lines[name], "Disable host update"),
        col(below, "Stop Discord"),
        "{text}"
    );
    // Remembered for the next launch.
    let saved = std::fs::read_to_string(_dir.path().join("config/config.toml")).unwrap();
    assert!(saved.contains("show_descriptions = true"), "{saved}");

    // Toggling back hides them.
    app.key_for_test(key(KeyCode::Char('I')));
    assert!(!render(&mut app, 120, 30).contains("Stop Discord"));
}

#[test]
fn help_names_the_apps_file_and_the_edit_key() {
    let (_dir, state) = sandbox();
    let mut app = App::new(state, false, false);
    app.key_for_test(key(KeyCode::Char('?')));
    let text = render(&mut app, 120, 40);
    assert!(text.contains("Your files"), "{text}");
    assert!(text.contains("apps.toml"), "{text}");
    assert!(text.contains("edit the app list"), "{text}");
}

#[test]
fn settings_pane_and_review_screen_render() {
    let (_dir, state) = sandbox_with_review();
    let mut app = App::new(state, false, false);
    let text = render(&mut app, 120, 40);
    if text.contains("To review") {
        app.key_for_test(key(KeyCode::Esc));
    }
    // Down to Settings and into it.
    for _ in 0..3 {
        app.key_for_test(key(KeyCode::Down));
    }
    app.key_for_test(key(KeyCode::Char('c')));
    let text = render(&mut app, 120, 40);
    assert!(text.contains("Edit the app list in your editor"), "{text}");
    assert!(
        text.contains("Review apps installed on this machine"),
        "{text}"
    );

    // Third row: the installed review. Whatever this machine has, the
    // screen must open and close cleanly.
    app.key_for_test(key(KeyCode::Down));
    app.key_for_test(key(KeyCode::Down));
    app.key_for_test(key(KeyCode::Enter));
    assert_eq!(app.screen_name_for_test(), "review");
    let text = render(&mut app, 120, 40);
    assert!(text.contains("Review: installed on this machine"), "{text}");
    for (w, h) in [(60, 15), (2, 2)] {
        render(&mut app, w, h);
    }
    app.key_for_test(key(KeyCode::Char('s')));
    render(&mut app, 120, 40);
    app.key_for_test(key(KeyCode::Esc));
    assert_eq!(app.screen_name_for_test(), "main");

    // Tick the first candidate and save: apps.toml grows, the old copy is
    // kept, the entry is switched on. Skipped on a machine with nothing to
    // review.
    let apps_file = _dir.path().join("config/apps.toml");
    let before = std::fs::read_to_string(&apps_file).unwrap();
    app.key_for_test(key(KeyCode::Enter));
    app.key_for_test(key(KeyCode::Char('s')));
    let text = render(&mut app, 120, 40);
    if text.contains(" · ") {
        app.key_for_test(key(KeyCode::Char(' ')));
        app.key_for_test(key(KeyCode::Char('w')));
        assert_eq!(app.screen_name_for_test(), "main");
        let after = std::fs::read_to_string(&apps_file).unwrap();
        assert!(after.len() > before.len(), "{after}");
        assert!(_dir.path().join("config/apps.toml.bak").exists());
        let config = std::fs::read_to_string(_dir.path().join("config/config.toml")).unwrap();
        assert!(config.contains("= true"), "{config}");
    } else {
        app.key_for_test(key(KeyCode::Esc));
    }
}
