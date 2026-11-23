//! Let the Unix line discipline deliver Ctrl-Z even when the UI cannot read keys.

#[cfg(unix)]
mod platform {
    use std::fs::{File, OpenOptions};
    use std::io::{self, Write};
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::thread::JoinHandle;

    use nix::sys::signal::{Signal, kill};
    use nix::sys::termios::{
        LocalFlags, SetArg, SpecialCharacterIndices, Termios, tcgetattr, tcsetattr,
    };
    use nix::unistd::{PathconfVar, fpathconf, getpgrp, getpid, tcgetpgrp};
    use signal_hook::consts::{SIGCONT, SIGTSTP};
    use signal_hook::iterator::{Handle, Signals};

    const RUNNING: u8 = 0;
    const SUSPENDED: u8 = 1;
    const RESUMED: u8 = 2;

    // The UI could be stuck writing to stdout. Use an independently opened,
    // nonblocking tty descriptor for best-effort cleanup before stopping.
    const LEAVE_UI: &[u8] =
        b"\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?2004l\x1b[?1049l\x1b[23;0t\x1b[?25h";

    pub(in crate::app) struct TerminalSettings {
        tty: File,
        shell: Termios,
    }

    impl TerminalSettings {
        pub(in crate::app) fn capture() -> Option<Self> {
            let tty = OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(nix::libc::O_NONBLOCK | nix::libc::O_NOCTTY)
                .open("/dev/tty")
                .ok()?;
            if !is_foreground(&tty) {
                return None;
            }
            let shell = tcgetattr(&tty).ok()?;
            Some(Self { tty, shell })
        }

        pub(in crate::app) fn start(self) -> anyhow::Result<JobControl> {
            let mut app = tcgetattr(&self.tty)?;
            app.local_flags.insert(LocalFlags::ISIG);
            // Keep the existing treatment of Ctrl-C and Ctrl-backslash. The
            // terminal driver should consume only Ctrl-Z as a job-control key.
            let disabled = fpathconf(&self.tty, PathconfVar::_POSIX_VDISABLE)?
                .and_then(|value| u8::try_from(value).ok())
                .unwrap_or(0);
            app.control_chars[SpecialCharacterIndices::VINTR as usize] = disabled;
            app.control_chars[SpecialCharacterIndices::VQUIT as usize] = disabled;
            app.control_chars[SpecialCharacterIndices::VSUSP as usize] = 0x1a;
            let mut signals = Signals::new([SIGTSTP, SIGCONT])?;
            let signal_handle = signals.handle();
            let phase = Arc::new(AtomicU8::new(RUNNING));
            let thread_phase = phase.clone();
            let tty = self.tty.try_clone()?;
            let resumed_mode = app.clone();
            let worker = std::thread::Builder::new()
                .name("carbide-job-control".into())
                .spawn(move || self.watch(&mut signals, &resumed_mode, &thread_phase))?;
            let job = JobControl {
                phase,
                signal_handle,
                worker: Some(worker),
            };
            tcsetattr(&tty, SetArg::TCSANOW, &app)?;
            Ok(job)
        }

        fn watch(mut self, signals: &mut Signals, app: &Termios, phase: &AtomicU8) {
            for signal in signals.forever() {
                match signal {
                    SIGTSTP => {
                        let previous = phase.swap(SUSPENDED, Ordering::AcqRel);
                        if is_foreground(&self.tty) {
                            let _ = tcsetattr(&self.tty, SetArg::TCSANOW, &self.shell);
                            if previous == RUNNING {
                                let _ = self.tty.write(LEAVE_UI);
                            }
                        }
                        let _ = kill(getpid(), Signal::SIGSTOP);
                    }
                    SIGCONT if phase.load(Ordering::Acquire) == SUSPENDED => {
                        if !is_foreground(&self.tty) {
                            // A foreground-only TUI stays stopped after "bg"
                            // instead of drawing on the resumed shell.
                            let _ = kill(getpid(), Signal::SIGSTOP);
                            continue;
                        }
                        let _ = tcsetattr(&self.tty, SetArg::TCSANOW, app);
                        phase.store(RESUMED, Ordering::Release);
                    }
                    _ => {}
                }
            }
        }
    }

    fn is_foreground(tty: &File) -> bool {
        tcgetpgrp(tty).is_ok_and(|group| group == getpgrp())
    }

    pub(in crate::app) struct JobControl {
        phase: Arc<AtomicU8>,
        signal_handle: Handle,
        worker: Option<JoinHandle<()>>,
    }

    impl JobControl {
        pub(in crate::app) fn suspended(&self) -> bool {
            self.phase.load(Ordering::Acquire) == SUSPENDED
        }

        pub(in crate::app) fn take_resume(&self) -> bool {
            self.phase
                .compare_exchange(RESUMED, RUNNING, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        }

        pub(in crate::app) fn suspend(&self) -> io::Result<()> {
            kill(getpid(), Signal::SIGTSTP).map_err(io::Error::from)
        }
    }

    impl Drop for JobControl {
        fn drop(&mut self) {
            self.signal_handle.close();
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }
}

#[cfg(not(unix))]
mod platform {
    pub(in crate::app) struct TerminalSettings;
    pub(in crate::app) struct JobControl;

    impl TerminalSettings {
        pub(in crate::app) fn capture() -> Option<Self> {
            None
        }
        pub(in crate::app) fn start(self) -> anyhow::Result<JobControl> {
            Ok(JobControl)
        }
    }

    impl JobControl {
        pub(in crate::app) fn suspended(&self) -> bool {
            false
        }
        pub(in crate::app) fn take_resume(&self) -> bool {
            false
        }
        pub(in crate::app) fn suspend(&self) -> std::io::Result<()> {
            Ok(())
        }
    }
}

pub(super) use platform::{JobControl, TerminalSettings};
