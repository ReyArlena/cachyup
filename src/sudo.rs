//! Sudo, the way the bash version did it: ask once at launch (in the plain
//! terminal, before the TUI takes over), then keep the ticket fresh in the
//! background. Every elevated command uses `sudo -n`, so nothing can ever
//! prompt over the TUI; without a ticket it fails instead.
//!
//! Only commands that need root use sudo. yay, flatpak and installers run as
//! the user (yay calls sudo itself, also with `-n`).

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static KEEPALIVE: AtomicBool = AtomicBool::new(false);

/// Whether sudo works without a password right now.
pub fn has_ticket() -> bool {
    Command::new("sudo")
        .args(["-n", "true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Ask for the password on the terminal. Only call while no TUI is drawn.
pub fn authenticate() -> bool {
    Command::new("sudo")
        .arg("-v")
        .status()
        .is_ok_and(|s| s.success())
}

/// Refresh the ticket every 50 s for the life of the process. Safe to call
/// more than once; only one refresher runs.
pub fn keep_alive() {
    if KEEPALIVE.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(Duration::from_secs(50));
            let _ = Command::new("sudo")
                .args(["-n", "-v"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    });
}

/// At launch: use an existing ticket, or ask for one. Returns whether sudo
/// is available. Starts the refresher either way it succeeds.
pub fn ensure(prompt: bool) -> bool {
    if has_ticket() {
        keep_alive();
        return true;
    }
    if !prompt {
        return false;
    }
    eprintln!("cachyup needs sudo for system updates and package installs.");
    let ok = authenticate();
    if ok {
        keep_alive();
    }
    ok
}
