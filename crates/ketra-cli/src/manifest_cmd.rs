//! `ketra manifest` and `ketra verify` — SHA-256 archive manifests.

use crate::sig;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

pub fn manifest_cmd(root: PathBuf, out: Option<PathBuf>) -> i32 {
    sig::install();
    let cancel = AtomicBool::new(false);
    match ketra_core::manifest::scan(&root, None, &cancel) {
        Ok(entries) => {
            let man = ketra_core::manifest::Manifest {
                algorithm: ketra_core::options::MANIFEST_ALGO.into(),
                root: root.display().to_string(),
                files: entries,
            };
            let path = out.unwrap_or_else(|| root.join(".ketra-manifest.json"));
            match ketra_core::manifest::write_file(&path, &man) {
                Ok(()) => {
                    println!("manifest: {} ({} entries)", path.display(), man.files.len());
                    0
                }
                Err(e) => {
                    eprintln!("ketra: {e}");
                    1
                }
            }
        }
        Err(e) => {
            eprintln!("ketra: {e}");
            1
        }
    }
}

pub fn verify_cmd(root: PathBuf, manifest: Option<PathBuf>, quiet: bool) -> i32 {
    sig::install();
    let cancel = AtomicBool::new(false);
    let mpath: PathBuf = manifest.unwrap_or_else(|| root.join(".ketra-manifest.json"));
    let man = match ketra_core::manifest::load(&mpath) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("ketra: {e}");
            return 3;
        }
    };
    match ketra_core::manifest::verify(&root, &man, &cancel) {
        Ok(statuses) => {
            let mut ok = 0u32;
            let mut bad = 0u32;
            for (path, st) in &statuses {
                match st {
                    ketra_core::manifest::VerifyStatus::Ok => ok += 1,
                    _ => {
                        bad += 1;
                        if !quiet {
                            match st {
                                ketra_core::manifest::VerifyStatus::Missing => {
                                    eprintln!("  missing: {path}")
                                }
                                ketra_core::manifest::VerifyStatus::SizeMismatch { expected, actual } => {
                                    eprintln!("  size:    {path} (expected {expected}, got {actual})")
                                }
                                ketra_core::manifest::VerifyStatus::ChecksumMismatch { .. } => {
                                    eprintln!("  corrupt: {path}")
                                }
                                ketra_core::manifest::VerifyStatus::Ok => {}
                            }
                        }
                    }
                }
            }
            if !quiet {
                println!(
                    "verify: {ok} ok, {bad} problem(s) ({} entries, algorithm {})",
                    man.files.len(),
                    man.algorithm
                );
            }
            if bad == 0 { 0 } else { 4 }
        }
        Err(e) if e.is_canceled() => 2,
        Err(e) => {
            eprintln!("ketra: {e}");
            1
        }
    }
}

/// Used by the GUI / tests: the default manifest path for a tree.
pub fn default_manifest_path(root: &Path) -> PathBuf {
    root.join(".ketra-manifest.json")
}
