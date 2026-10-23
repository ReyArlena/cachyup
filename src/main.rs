use cachyup::cli::{Cli, run_cli};
use clap::Parser;

fn main() {
    reset_sigpipe();
    let cli = Cli::parse();
    match run_cli(cli) {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("cachyup: {err:#}");
            std::process::exit(2);
        }
    }
}

/// Rust ignores SIGPIPE, so `cachyup --run | head` would panic on a closed
/// pipe. Restore the default so the process just exits quietly.
fn reset_sigpipe() {
    use nix::sys::signal::{SigHandler, Signal, signal};
    // SAFETY: setting a signal disposition before any other thread exists.
    unsafe {
        let _ = signal(Signal::SIGPIPE, SigHandler::SigDfl);
    }
}
