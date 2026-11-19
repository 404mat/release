//! Spinner and progress output. Everything goes to stderr so that stdout stays
//! clean for `--json`.

use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use release_core::Progress;

struct Active {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

pub struct TextProgress {
    interactive: bool,
    quiet: bool,
    active: Mutex<Option<Active>>,
}

impl TextProgress {
    pub fn new(quiet: bool) -> Self {
        Self {
            interactive: std::io::stderr().is_terminal(),
            quiet,
            active: Mutex::new(None),
        }
    }

    fn finish(&self, success: bool, message: Option<&str>) {
        let active = self.active.lock().ok().and_then(|mut slot| slot.take());

        if let Some(active) = active {
            active.stop.store(true, Ordering::Relaxed);
            let _ = active.handle.join();

            if self.interactive {
                eprint!("\r\x1b[K");
            }

            if let Some(message) = message {
                if !self.quiet {
                    let marker = if success { "✓" } else { "✗" };
                    eprintln!("{marker} {message}");
                }
            }
        } else if let Some(message) = message {
            if !self.quiet {
                eprintln!("{message}");
            }
        }
    }
}

impl Progress for TextProgress {
    fn start(&self, message: &str) {
        if let Ok(mut slot) = self.active.lock() {
            if let Some(active) = slot.take() {
                active.stop.store(true, Ordering::Relaxed);
                let _ = active.handle.join();
            }
        }

        if self.quiet {
            return;
        }

        if !self.interactive {
            eprintln!("{message}");
            return;
        }

        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let label = message.to_string();

        let handle = thread::spawn(move || {
            const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let mut index = 0usize;

            while !flag.load(Ordering::Relaxed) {
                eprint!("\r\x1b[K{} {}", FRAMES[index % FRAMES.len()], label);
                index += 1;
                thread::sleep(Duration::from_millis(80));
            }
        });

        if let Ok(mut slot) = self.active.lock() {
            *slot = Some(Active { stop, handle });
        }
    }

    fn update(&self, message: &str) {
        self.start(message);
    }

    fn succeed(&self, message: Option<&str>) {
        self.finish(true, message);
    }

    fn warn(&self, message: &str) {
        self.finish(false, None);

        if !self.quiet {
            eprintln!("warning: {message}");
        }
    }
}
