//! Ctrl+C handling without external crates: SIGINT on unix,
//! SetConsoleCtrlHandler on Windows. The handler only flips a flag; the
//! progress loop converts it into a controller cancel (graceful: partial
//! files + journal are preserved for resume).

use std::sync::atomic::{AtomicBool, Ordering};

pub static INTERRUPTED: AtomicBool = AtomicBool::new(false);

pub fn was_interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

#[cfg(unix)]
mod imp {
    use super::INTERRUPTED;
    use std::os::raw::c_int;

    const SIGINT: c_int = 2;
    const SIGTERM: c_int = 15;

    extern "C" fn handler(_sig: c_int) {
        INTERRUPTED.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    extern "C" {
        fn signal(signum: c_int, handler: usize) -> usize;
    }

    pub fn install() {
        unsafe {
            signal(SIGINT, handler as *const () as usize);
            signal(SIGTERM, handler as *const () as usize);
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::INTERRUPTED;
    const CTRL_C_EVENT: u32 = 0;
    const CTRL_CLOSE_EVENT: u32 = 2;

    extern "system" fn handler(_t: u32) -> i32 {
        INTERRUPTED.store(true, std::sync::atomic::Ordering::SeqCst);
        1 // handled
    }

    extern "system" {
        fn SetConsoleCtrlHandler(handler: usize, add: i32) -> i32;
    }

    pub fn install() {
        unsafe {
            SetConsoleCtrlHandler(handler as usize, 1);
        }
        let _ = (CTRL_C_EVENT, CTRL_CLOSE_EVENT);
    }
}

pub use imp::install;
