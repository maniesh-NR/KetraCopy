//! `ketra bench` — a small, honest disk benchmark: write throughput, then
//! read throughput through the engine's hash path (which is how verify
//! actually reads files).

use crate::sig;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

pub fn run(path: Option<std::path::PathBuf>, size: u64) -> i32 {
    sig::install();
    let dir = path.unwrap_or_else(std::env::temp_dir);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("ketra: bench: {}: {e}", dir.display());
        return 1;
    }
    let file = dir.join(format!(".ketra-bench-{}", std::process::id()));
    let size = size.clamp(1 << 20, 8 << 30);

    // ---- write ----
    let chunk = vec![0xA5u8; 1024 * 1024];
    let t0 = Instant::now();
    let mut wrote = 0u64;
    let mut io_err = None;
    {
        use std::io::Write as _;
        match std::fs::File::create(&file) {
            Ok(mut f) => {
                while wrote < size {
                    if sig::was_interrupted() {
                        break;
                    }
                    match f.write_all(&chunk) {
                        Ok(()) => wrote += chunk.len() as u64,
                        Err(e) => {
                            io_err = Some(e.to_string());
                            break;
                        }
                    }
                }
                let _ = f.sync_all();
            }
            Err(e) => io_err = Some(e.to_string()),
        }
    }
    let wdt = t0.elapsed();
    if let Some(e) = io_err {
        eprintln!("ketra: bench write: {e}");
        let _ = std::fs::remove_file(&file);
        return 1;
    }
    let w_mibs = wrote as f64 / (1024.0 * 1024.0) / wdt.as_secs_f64().max(0.000001);

    // ---- read (through the hash path, like verify) ----
    let cancel = AtomicBool::new(false);
    let t1 = Instant::now();
    let read_result = ketra_core::options::hash_file(&file, "xxh64", 1024 * 1024, &cancel);
    let rdt = t1.elapsed();
    let _ = std::fs::remove_file(&file);
    match read_result {
        Ok(digest) => {
            let r_mibs = wrote as f64 / (1024.0 * 1024.0) / rdt.as_secs_f64().max(0.000001);
            println!(
                "wrote {} in {:.2}s -> {:.1} MiB/s",
                human(wrote),
                wdt.as_secs_f64(),
                w_mibs
            );
            println!(
                "read  {} in {:.2}s -> {:.1} MiB/s (xxh64 = {digest})",
                human(wrote),
                rdt.as_secs_f64(),
                r_mibs
            );
            0
        }
        Err(e) => {
            eprintln!("ketra: bench read: {e}");
            1
        }
    }
}

fn human(n: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = MB * 1024.0;
    let v = n as f64;
    if v >= GB {
        format!("{:.2} GiB", v / GB)
    } else {
        format!("{:.1} MiB", v / MB)
    }
}
