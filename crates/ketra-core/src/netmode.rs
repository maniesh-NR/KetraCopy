//! Experimental multi-stream parallel transfer for high-latency /
//! high-bandwidth links (network shares, cloud mounts).
//!
//! Technique (see `docs/ALGORITHMS.md`): the file is split into N
//! contiguous ranges, each range is copied by its own thread through
//! independent file descriptors — the same range-splitting idea used by
//! bbcp (SLAC) and GridFTP (Allcock et al., 2003). A shared token bucket
//! applies the global `--limit`; per-stream stalls feed one AIMD loop that
//! adjusts the shared rate.

use crate::aimd::Aimd;
use crate::error::{Error, Result};
use crate::options::Options;
use crate::throttle::TokenBucket;
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAX_STREAMS: u32 = 16;

/// Copy `src` → `dst` using `opts.net_streams` parallel range streams.
/// `progress` receives total transferred bytes (cumulative).
pub fn copy_parallel(
    src: &Path,
    dst: &Path,
    opts: &Options,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<crate::pipeline::CopyStats> {
    let mut stats = crate::pipeline::CopyStats::default();
    let smd = std::fs::symlink_metadata(src).map_err(|e| Error::ioctx(src, e))?;
    let size = smd.len();

    if let Some(parent) = dst.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| Error::ioctx(parent, e))?;
        }
    }

    // Fresh, sparse-capable destination of the full logical size.
    let df = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(dst)
        .map_err(|e| Error::ioctx(dst, e))?;
    df.set_len(size).map_err(|e| Error::ioctx(dst, e))?;
    drop(df);

    let streams = opts.net_streams.max(2).min(MAX_STREAMS) as usize;
    if size == 0 {
        return Ok(stats);
    }
    let range = size.div_ceil(streams as u64);

    let done = Arc::new(AtomicU64::new(0));
    let bucket = opts.speed_limit_bps.map(|r| Arc::new(Mutex::new(TokenBucket::new(r as f64))));
    let aimd = Arc::new(Mutex::new(Aimd::new(
        32.0 * 1024.0 * 1024.0,
        256.0 * 1024.0,
        2.0 * 1024.0 * 1024.0 * 1024.0,
    )));
    let chunk = opts.chunk() as usize;

    // Scoped threads: the streams may borrow `cancel` directly.
    let mut first_err: Option<Error> = None;
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(streams);
        for s in 0..streams {
            let start = s as u64 * range;
            let end = (start + range).min(size);
            if start >= end {
                break;
            }
            let src = src.to_path_buf();
            let dst = dst.to_path_buf();
            let done = Arc::clone(&done);
            let bucket = bucket.clone();
            let aimd = Arc::clone(&aimd);

            handles.push(scope.spawn(move || -> Result<u64> {
                let sf = File::open(&src).map_err(|e| Error::ioctx(&src, e))?;
                let tf = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&dst)
                    .map_err(|e| Error::ioctx(&dst, e))?;
                let mut buf = vec![0u8; chunk];
                let mut local = 0u64;
                let mut off = start;
                while off < end {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(Error::Canceled);
                    }
                    let len = ((end - off) as usize).min(chunk);
                    if let Some(b) = &bucket {
                        let rate = { aimd.lock().map(|a| a.rate()).unwrap_or(0.0) };
                        b.lock().map(|mut t| {
                            t.set_rate(rate);
                            t.acquire(len as u64)
                        }).ok();
                    }
                    let t0 = std::time::Instant::now();
                    // read/write (pread/pwrite) loop
                    let mut got = 0usize;
                    while got < len {
                        let n = crate::platform::read_at(&sf, &mut buf[got..len], off + got as u64)
                            .map_err(|e| Error::ioctx(&src, e))?;
                        if n == 0 {
                            break;
                        }
                        got += n;
                    }
                    if got == 0 {
                        break;
                    }
                    let mut put = 0usize;
                    while put < got {
                        let n = crate::platform::write_at(&tf, &buf[put..got], off + put as u64)
                            .map_err(|e| Error::ioctx(&dst, e))?;
                        if n == 0 {
                            return Err(Error::Io {
                                path: dst.display().to_string(),
                                source: std::io::Error::new(std::io::ErrorKind::WriteZero, "short write"),
                            });
                        }
                        put += n;
                    }
                    let dt = t0.elapsed();
                    local += got as u64;
                    off += got as u64;
                    done.fetch_add(got as u64, Ordering::Relaxed);

                    // AIMD on stall: share the observation across streams.
                    let target = aimd.lock().map(|a| a.rate()).unwrap_or(0.0);
                    let expected = if target > 0.0 {
                        Duration::from_secs_f64(got as f64 / target)
                    } else {
                        Duration::ZERO
                    };
                    let stalled = dt > expected * 4 && dt > Duration::from_millis(150);
                    if let Ok(mut a) = aimd.lock() {
                        if stalled {
                            a.on_congestion();
                        } else {
                            a.on_success();
                        }
                    }
                }
                Ok(local)
            }));
        }

        // Pump progress while the streams run.
        let mut last_reported = 0u64;
        loop {
            let all_done = handles.iter().all(|h| h.is_finished());
            let now = done.load(Ordering::Relaxed);
            if now > last_reported {
                progress(now - last_reported);
                last_reported = now;
            }
            if all_done {
                break;
            }
            std::thread::sleep(Duration::from_millis(40));
        }

        for h in handles {
            match h.join() {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                }
                Err(_) => {
                    if first_err.is_none() {
                        first_err = Some(Error::Unsupported("worker thread panicked".into()));
                    }
                }
            }
        }
    });

    if cancel.load(Ordering::Relaxed) {
        return Err(Error::Canceled);
    }
    match first_err {
        Some(e) if e.is_canceled() => return Err(Error::Canceled),
        Some(e) => return Err(e),
        None => {}
    }

    // Finalize: truncate to exact size (prevents torn tail), fsync, meta.
    let df = OpenOptions::new()
        .write(true)
        .open(dst)
        .map_err(|e| Error::ioctx(dst, e))?;
    df.set_len(size).map_err(|e| Error::ioctx(dst, e))?;
    df.sync_all().map_err(|e| Error::ioctx(dst, e))?;
    if opts.preserve_mtime {
        if let Ok(mtime) = smd.modified() {
            let mut times = std::fs::FileTimes::new();
            times.set_modified(mtime);
            if let Ok(atime) = smd.accessed() {
                times.set_accessed(atime);
            }
            df.set_times(times).map_err(|e| Error::ioctx(dst, e))?;
        }
    }
    if opts.preserve_perms {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = crate::platform::st_mode(&smd) & 0o7777;
            std::fs::set_permissions(dst, std::fs::Permissions::from_mode(mode))
                .map_err(|e| Error::ioctx(dst, e))?;
        }
        #[cfg(not(unix))]
        {
            let _ = (&smd, dst);
        }
    }

    stats.bytes = done.load(Ordering::Relaxed);
    Ok(stats)
}
