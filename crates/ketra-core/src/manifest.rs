//! Phase 3 — SHA-256 archive manifests.
//!
//! A manifest is a JSON document listing every archived file with its size,
//! mtime and checksum, e.g. written to `<dst>/.ketra-manifest.json`:
//!
//! ```json
//! {"ketra_manifest":1,"algorithm":"sha256","root":"/backup",
//!  "files":[{"path":"docs/a.txt","size":3,"mtime":1700000000,"sha256":"..."}]}
//! ```
//!
//! `ketra verify` recomputes checksums and reports any file whose bytes no
//! longer match (bit rot, truncated transfer, tampering).

use crate::error::{Error, Result};
use crate::json::{self, Json};
use crate::options::{hash_file, MANIFEST_ALGO};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// One verified entry of a manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct ManifestFile {
    /// Path relative to the manifest root, `/`-separated.
    pub path: String,
    pub size: u64,
    pub mtime: i64,
    pub sha256: String,
}

/// A parsed manifest.
#[derive(Debug, Clone)]
pub struct Manifest {
    pub algorithm: String,
    pub root: String,
    pub files: Vec<ManifestFile>,
}

/// Build a manifest for every file under `root` (or the exact list given).
/// All files are hashed by reading.
pub fn scan(
    root: &Path,
    files: Option<&[PathBuf]>,
    cancel: &AtomicBool,
) -> Result<Vec<ManifestFile>> {
    scan_with_digests(root, files, &BTreeMap::new(), cancel)
}

/// Like [`scan`], but reuses SHA-256 digests that were computed inline
/// during the copy (keyed by root-relative, `/`-separated path). Files
/// present in `digests` are not re-read — a `--verify manifest` run that
/// hashed every streamed chunk in the pipeline skips the entire second
/// read pass. Digests not listed (resumed, cloned, netmode files) fall
/// back to a full read.
pub fn scan_with_digests(
    root: &Path,
    files: Option<&[PathBuf]>,
    digests: &BTreeMap<String, String>,
    cancel: &AtomicBool,
) -> Result<Vec<ManifestFile>> {
    let mut paths: Vec<PathBuf> = match files {
        Some(f) => f.to_vec(),
        None => {
            let mut v = Vec::new();
            collect_files(root, root, &mut v, cancel)?;
            v.sort();
            v
        }
    };
    paths.sort();
    paths.dedup();
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Canceled);
        }
        let md = std::fs::metadata(&p).map_err(|e| Error::ioctx(&p, e))?;
        let rel = p
            .strip_prefix(root)
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| p.display().to_string());
        let sha256 = match digests.get(&rel) {
            Some(d) => d.clone(),
            None => hash_file(&p, MANIFEST_ALGO, 1024 * 1024, cancel)?,
        };
        out.push(ManifestFile {
            path: rel,
            size: md.len(),
            mtime: mtime_secs(&md),
            sha256,
        });
    }
    Ok(out)
}

fn collect_files(dir: &Path, root: &Path, out: &mut Vec<PathBuf>, cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::Canceled);
    }
    let entries = std::fs::read_dir(dir).map_err(|e| Error::ioctx(dir, e))?;
    for e in entries {
        let e = e.map_err(|er| Error::ioctx(dir, er))?;
        let p = e.path();
        let md = std::fs::metadata(&p).map_err(|er| Error::ioctx(&p, er))?;
        if md.is_dir() {
            collect_files(&p, root, out, cancel)?;
        } else if md.is_file() {
            out.push(p);
        }
    }
    Ok(())
}

fn mtime_secs(md: &std::fs::Metadata) -> i64 {
    use std::time::UNIX_EPOCH;
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Serialize a manifest to JSON text.
pub fn to_json(manifest: &Manifest) -> String {
    let mut s = String::new();
    s.push_str("{\n  \"ketra_manifest\": 1,\n");
    s.push_str(&format!("  \"algorithm\": {},\n", json::quote(&manifest.algorithm)));
    s.push_str(&format!("  \"root\": {},\n", json::quote(&manifest.root)));
    s.push_str("  \"files\": [\n");
    for (i, f) in manifest.files.iter().enumerate() {
        s.push_str(&format!(
            "    {{\"path\": {}, \"size\": {}, \"mtime\": {}, \"sha256\": {}}}",
            json::quote(&f.path),
            f.size,
            f.mtime,
            json::quote(&f.sha256)
        ));
        if i + 1 < manifest.files.len() {
            s.push(',');
        }
        s.push('\n');
    }
    s.push_str("  ]\n}\n");
    s
}

/// Write a manifest file (parent dirs created, fsynced).
pub fn write_file(path: &Path, manifest: &Manifest) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| Error::ioctx(parent, e))?;
        }
    }
    let mut f = std::fs::File::create(path).map_err(|e| Error::ioctx(path, e))?;
    f.write_all(to_json(manifest).as_bytes())
        .map_err(|e| Error::ioctx(path, e))?;
    f.sync_all().map_err(|e| Error::ioctx(path, e))?;
    Ok(())
}

/// Parse a manifest file.
pub fn from_json(text: &str) -> Result<Manifest> {
    let v = json::parse(text).map_err(Error::Config)?;
    if v.get("ketra_manifest").and_then(|j| j.as_u64()) != Some(1) {
        return Err(Error::Config("not a ketra manifest (missing ketra_manifest=1)".into()));
    }
    let algorithm = v
        .get("algorithm")
        .and_then(|j| j.as_str())
        .unwrap_or(MANIFEST_ALGO)
        .to_string();
    let root = v.get("root").and_then(|j| j.as_str()).unwrap_or("").to_string();
    let mut files = Vec::new();
    if let Some(Json::Arr(items)) = v.get("files") {
        for it in items {
            let path = it
                .get("path")
                .and_then(|j| j.as_str())
                .ok_or_else(|| Error::Config("manifest entry missing path".into()))?
                .to_string();
            let size = it
                .get("size")
                .and_then(|j| j.as_u64())
                .ok_or_else(|| Error::Config("manifest entry missing size".into()))?;
            let mtime = it.get("mtime").and_then(|j| j.as_f64()).unwrap_or(0.0) as i64;
            let sha = it
                .get("sha256")
                .and_then(|j| j.as_str())
                .unwrap_or("")
                .to_string();
            files.push(ManifestFile { path, size, mtime, sha256: sha });
        }
    }
    Ok(Manifest { algorithm, root, files })
}

/// Load a manifest from disk.
pub fn load(path: &Path) -> Result<Manifest> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::ioctx(path, e))?;
    from_json(&text)
}

/// Result of verifying one file.
#[derive(Debug, Clone, PartialEq)]
pub enum VerifyStatus {
    Ok,
    Missing,
    SizeMismatch { expected: u64, actual: u64 },
    ChecksumMismatch { expected: String, actual: String },
}

/// Verify `manifest` against the tree at `root`. Returns per-file status.
pub fn verify(
    root: &Path,
    manifest: &Manifest,
    cancel: &AtomicBool,
) -> Result<BTreeMap<String, VerifyStatus>> {
    let mut out = BTreeMap::new();
    for f in &manifest.files {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Canceled);
        }
        let p = root.join(&f.path);
        let md = match std::fs::metadata(&p) {
            Ok(md) => md,
            Err(_) => {
                out.insert(f.path.clone(), VerifyStatus::Missing);
                continue;
            }
        };
        if md.len() != f.size {
            out.insert(
                f.path.clone(),
                VerifyStatus::SizeMismatch { expected: f.size, actual: md.len() },
            );
            continue;
        }
        let actual = hash_file(&p, &manifest.algorithm, 1024 * 1024, cancel)?;
        if actual != f.sha256 {
            out.insert(
                f.path.clone(),
                VerifyStatus::ChecksumMismatch { expected: f.sha256.clone(), actual },
            );
        } else {
            out.insert(f.path.clone(), VerifyStatus::Ok);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ketra-manifest-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn test_scan_write_load_verify_roundtrip() {
        let dir = tmp("rt");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/a.txt"), b"hello").unwrap();
        std::fs::write(dir.join("b.bin"), [7u8; 4096]).unwrap();
        let cancel = AtomicBool::new(false);

        let files = scan(&dir, None, &cancel).unwrap();
        assert_eq!(files.len(), 2);
        let manifest = Manifest {
            algorithm: MANIFEST_ALGO.into(),
            root: dir.display().to_string(),
            files,
        };
        let mpath = dir.join(".ketra-manifest.json");
        write_file(&mpath, &manifest).unwrap();

        let loaded = load(&mpath).unwrap();
        assert_eq!(loaded.algorithm, "sha256");
        assert_eq!(loaded.files.len(), 2);

        let statuses = verify(&dir, &loaded, &cancel).unwrap();
        assert!(statuses.values().all(|s| *s == VerifyStatus::Ok));

        // Bit rot: flip one byte.
        let p = dir.join("b.bin");
        let mut data = std::fs::read(&p).unwrap();
        data[10] ^= 0x01;
        std::fs::write(&p, &data).unwrap();
        let statuses = verify(&dir, &loaded, &cancel).unwrap();
        assert!(matches!(
            statuses.get("b.bin"),
            Some(VerifyStatus::ChecksumMismatch { .. })
        ));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_verify_reports_missing_and_size_mismatch() {
        let dir = tmp("mm");
        std::fs::write(dir.join("keep.txt"), b"abc").unwrap();
        let cancel = AtomicBool::new(false);
        let mut manifest = Manifest {
            algorithm: MANIFEST_ALGO.into(),
            root: dir.display().to_string(),
            files: vec![],
        };
        manifest.files = scan(&dir, None, &cancel).unwrap();
        manifest.files.push(ManifestFile {
            path: "gone.txt".into(),
            size: 3,
            mtime: 0,
            sha256: "00".repeat(32),
        });
        std::fs::write(dir.join("keep.txt"), b"abcd").unwrap();

        let statuses = verify(&dir, &manifest, &cancel).unwrap();
        assert_eq!(statuses.get("gone.txt"), Some(&VerifyStatus::Missing));
        assert!(matches!(
            statuses.get("keep.txt"),
            Some(VerifyStatus::SizeMismatch { expected: 3, actual: 4 })
        ));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_from_json_rejects_foreign_documents() {
        assert!(from_json("{\"hello\": 1}").is_err());
        assert!(from_json("not json").is_err());
        let m = from_json(
            "{\"ketra_manifest\":1,\"algorithm\":\"sha256\",\"root\":\"/x\",\"files\":[{\"path\":\"a\",\"size\":1,\"mtime\":2,\"sha256\":\"ab\"}]}",
        )
        .unwrap();
        assert_eq!(m.files[0].sha256, "ab");
    }
}
