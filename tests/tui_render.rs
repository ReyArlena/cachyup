//! Render the screens into an in-memory terminal. Catches layout panics (for
//! example on tiny terminals) and checks the main pieces show up, without a TTY.

use cachyup::app::AppState;
use cachyup::tui::App;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn sandbox() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("temp dir");
    // No network calls to GitHub from tests.
    let config = dir.path().join("config/config.toml");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, "[settings]\nupdate_check = false\n").unwrap();
    let state = AppState::sandboxed(dir.path()).expect("sandboxed state");
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
fn dotfiles_asks_for_the_folder_then_lists_packages() {
    let (dir, state) = sandbox();
    // A stow folder in the sandbox home: one package linked, one not.
    let home = dir.path().join("home");
    let dots = home.join("dots");
    for f in [
        "fish/.config/fish/config.fish",
        "niri/.config/niri/config.kdl",
    ] {
        let p = dots.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "").unwrap();
    }
    std::fs::create_dir_all(home.join(".config")).unwrap();
    std::os::unix::fs::symlink("../dots/fish/.config/fish", home.join(".config/fish")).unwrap();

    let mut app = App::new(state, false, false);
    // Stow is checked before anything else, and blocks the screen.
    app.set_stow_check_for_test(|| false);
    app.key_for_test(key(KeyCode::Char('D')));
    let text = render(&mut app, 120, 36);
    assert!(text.contains("GNU Stow is not installed"), "{text}");
    assert!(text.contains("back to main menu"), "{text}");
    assert!(!text.contains("Dotfiles folder"), "{text}");
    app.key_for_test(key(KeyCode::Esc));
    assert_eq!(app.screen_name_for_test(), "main");
    assert!(!render(&mut app, 120, 36).contains("GNU Stow"));

    app.set_stow_check_for_test(|| true);
    app.key_for_test(key(KeyCode::Char('D')));
    let text = render(&mut app, 120, 36);
    assert!(text.contains("Dotfiles folder"), "{text}");
    assert!(text.contains("~/dotfiles"), "{text}");

    // The default does not exist in the sandbox: asked again.
    app.key_for_test(key(KeyCode::Enter));
    let text = render(&mut app, 120, 36);
    assert!(text.contains("Not a folder: ~/dotfiles"), "{text}");
    assert_eq!(app.screen_name_for_test(), "main");

    for _ in 0.."~/dotfiles".len() {
        app.key_for_test(key(KeyCode::Backspace));
    }
    for c in "~/dots".chars() {
        app.key_for_test(key(KeyCode::Char(c)));
    }
    app.key_for_test(key(KeyCode::Enter));
    assert_eq!(app.screen_name_for_test(), "dotfiles");
    app.wait_dotfiles_for_test();
    let text = render(&mut app, 120, 36);
    assert!(text.contains("Dotfiles ~/dots"), "{text}");
    assert!(text.contains("Linked (1)"), "{text}");
    assert!(text.contains("folder link"), "{text}");
    assert!(text.contains("not linked"), "{text}");
    assert!(text.contains("~/.config/fish"), "{text}");
    for (w, h) in [(60, 15), (2, 2)] {
        render(&mut app, w, h);
    }

    // Remembered, and Esc goes back.
    let saved = std::fs::read_to_string(dir.path().join("config/config.toml")).unwrap();
    assert!(saved.contains("dir = \"~/dots\""), "{saved}");
    app.key_for_test(key(KeyCode::Esc));
    assert_eq!(app.screen_name_for_test(), "main");
    app.key_for_test(key(KeyCode::Char('D')));
    assert_eq!(app.screen_name_for_test(), "dotfiles");
}

/// The Dotfiles screen against this machine's real stow folder. Opt in:
///   CACHYUP_DUMP=/tmp/d cargo test --test tui_render -- --ignored dotfiles
#[test]
#[ignore]
fn dump_dotfiles_for_this_machine() {
    let state = AppState::load(cachyup::paths::Paths::from_env()).unwrap();
    let mut app = App::new(state, true, false);
    app.key_for_test(key(KeyCode::Char('D')));
    app.wait_dotfiles_for_test();
    render(&mut app, 140, 40);
    for _ in 0..30 {
        app.key_for_test(key(KeyCode::Down));
        render(&mut app, 140, 40);
    }
}
