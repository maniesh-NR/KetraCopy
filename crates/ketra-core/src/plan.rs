//! Phase 0 — build the transfer plan: walk sources, apply filters, resolve
//! destination paths.
//!
//! Path mapping rules:
//! - A regular-file source `S` maps to `dst/<file_name(S)>`.
//! - A directory source `S` mirrors its *contents* into `dst`
//!   (`S/a/b.txt` → `dst/a/b.txt`); files at the root of `S` therefore land
//!   in the destination root.
//! - `flatten` collapses every file into `dst` regardless of depth.
//! - Filters match the `/`-separated path *relative to the source root*;
//!   an unmatched directory is still descended into because files below it
//!   may match (only files are pruned).

use crate::error::{Error, Result};
use crate::glob;
use crate::options::Options;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// One file to transfer.
#[derive(Debug, Clone)]
pub struct FileTask {
    /// Absolute-ish source path as given.
    pub src: PathBuf,
    /// Destination path.
    pub dst: PathBuf,
    /// Source-root-relative path with `/` separators.
    pub rel: String,
    /// Source file size in bytes.
    pub size: u64,
    /// Source modification time (seconds since epoch).
    pub mtime: i64,
    /// Device id of the source volume.
    pub dev: u64,
    /// Unix mode bits of the source file.
    pub mode: u32,
}

/// The outcome of the planning phase.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub tasks: Vec<FileTask>,
    /// Directories that must be created under `dst` (relative, sorted).
    pub dirs: Vec<String>,
    pub total_bytes: u64,
}

/// Build the plan for the given sources and destination root.
pub fn build(sources: &[PathBuf], dst: &Path, opts: &Options) -> Result<Plan> {
    let mut plan = Plan::default();
    let mut seen_rels: BTreeSet<String> = BTreeSet::new();
    let mut seen_files: BTreeSet<PathBuf> = BTreeSet::new();

    for src in sources {
        let src = src.as_path();
        let md = std::fs::symlink_metadata(src).map_err(|e| Error::ioctx(src, e))?;
        if md.is_dir() {
            // Root-level files of a directory source go straight to dst root.
            walk_dir(src, src, dst, opts, &mut plan, &mut seen_rels, &mut seen_files)?;
        } else if md.is_file() {
            let name = src
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "unnamed".to_string());
            if !allowed(&name, &name, opts) {
                continue;
            }
            if seen_files.insert(src.to_path_buf()) {
                push_task(src, src, dst, &name, md, opts, &mut plan, &mut seen_rels)?;
            }
        } else {
            return Err(Error::Unsupported(format!(
                "source '{}' is neither file nor directory",
                src.display()
            )));
        }
    }
    plan.dirs.sort();
    Ok(plan)
}

fn allowed(rel: &str, file_name: &str, opts: &Options) -> bool {
    // Exclude wins. Include (when non-empty) must match.
    if glob::matches_any(&opts.excludes, rel) || glob::matches_any(&opts.excludes, file_name) {
        return false;
    }
    if opts.includes.is_empty() {
        return true;
    }
    glob::matches_any(&opts.includes, rel) || glob::matches_any(&opts.includes, file_name)
}

fn rel_display(rel: &str, opts: &Options) -> String {
    if opts.flatten {
        rel.rsplit('/').next().unwrap_or(rel).to_string()
    } else {
        rel.to_string()
    }
}

#[allow(clippy::too_many_arguments)]
fn push_task(
    src: &Path,
    root: &Path,
    dst: &Path,
    rel: &str,
    md: std::fs::Metadata,
    opts: &Options,
    plan: &mut Plan,
    seen_rels: &mut BTreeSet<String>,
) -> Result<()> {
    if md.is_dir() {
        let shown = rel_display(rel, opts);
        if !seen_rels.insert(shown.clone()) {
            return Ok(());
        }
        if !opts.flatten {
            plan.dirs.push(shown);
        }
        return Ok(());
    }
    if !md.is_file() {
        return Ok(()); // symlinks & special files are skipped (documented)
    }
    if !allowed(rel, &rel.rsplit('/').next().unwrap_or(rel), opts) {
        return Ok(());
    }
    let shown = rel_display(rel, opts);
    if !seen_rels.insert(shown.clone()) {
        return Ok(());
    }
    let dst_path = dst.join(&shown);
    plan.tasks.push(FileTask {
        src: src.to_path_buf(),
        dst: dst_path,
        rel: shown,
        size: md.len(),
        mtime: mtime_of(&md),
        dev: crate::platform::st_dev(&md),
        mode: perms_of(&md),
    });
    plan.total_bytes += md.len();
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn walk_dir(
    dir: &Path,
    root: &Path,
    dst: &Path,
    opts: &Options,
    plan: &mut Plan,
    seen_rels: &mut BTreeSet<String>,
    seen_files: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| Error::ioctx(dir, e))?;
    let mut items: Vec<_> = entries
        .filter_map(|e| e.ok())
        .map(|e| (e.path(), e.file_name()))
        .collect();
    items.sort();
    for (path, name) in items {
        let name = name.to_string_lossy().to_string();
        // NOTE: directories are always descended; only files are filtered.
        // A directory that itself does not match `--include` may still
        // contain matching files, so pruning it here would be wrong.
        let md = std::fs::metadata(&path).map_err(|e| Error::ioctx(&path, e))?;
        let rel = if dir == root {
            name.clone()
        } else {
            let parent = dir.strip_prefix(root).map_err(|e| Error::Config(e.to_string()))?;
            let p = parent.to_string_lossy().replace('\\', "/");
            if p.is_empty() {
                name.clone()
            } else {
                format!("{p}/{name}")
            }
        };
        if md.is_dir() {
            let shown = rel_display(&rel, opts);
            if !opts.flatten && !seen_rels.insert(shown.clone()) {
                continue;
            }
            if !opts.flatten {
                if allowed(&rel, &name, opts) || opts.includes.is_empty() {
                    plan.dirs.push(shown);
                }
            }
            walk_dir(&path, root, dst, opts, plan, seen_rels, seen_files)?;
        } else if md.is_file() {
            if seen_files.insert(path.clone()) && allowed(&rel, &name, opts) {
                push_task(&path, root, dst, &rel, md, opts, plan, seen_rels)?;
            }
        }
    }
    Ok(())
}

fn mtime_of(md: &std::fs::Metadata) -> i64 {
    use std::time::UNIX_EPOCH;
    md.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn perms_of(md: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        md.permissions().mode()
    }
    #[cfg(not(unix))]
    {
        0o644
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::OnConflict;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ketra-plan-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn opts() -> Options {
        Options { conflict: OnConflict::Overwrite, ..Default::default() }
    }

    #[test]
    fn test_root_files_land_in_destination_root() {
        let src = tmp("rootfiles");
        let dst = tmp("rootfiles-dst");
        std::fs::write(src.join("top.txt"), b"top").unwrap();
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("sub/nested.txt"), b"n").unwrap();

        let plan = build(&[src.clone()], &dst, &opts()).unwrap();
        let rels: Vec<&str> = plan.tasks.iter().map(|t| t.rel.as_str()).collect();
        assert_eq!(rels, vec!["sub/nested.txt", "top.txt"]);
        let top = plan.tasks.iter().find(|t| t.rel == "top.txt").unwrap();
        assert_eq!(top.dst, dst.join("top.txt"));
        assert_eq!(plan.total_bytes, 4); // b"top" + b"n"

        std::fs::remove_dir_all(&src).ok();
        std::fs::remove_dir_all(&dst).ok();
    }

    #[test]
    fn test_flatten_collapses_hierarchy() {
        let src = tmp("flat");
        let dst = tmp("flat-dst");
        std::fs::create_dir_all(src.join("a/b")).unwrap();
        std::fs::write(src.join("a/b/deep.txt"), b"d").unwrap();
        std::fs::write(src.join("a/one.txt"), b"1").unwrap();

        let o = Options { flatten: true, ..opts() };
        let plan = build(&[src.clone()], &dst, &o).unwrap();
        let rels: Vec<&str> = plan.tasks.iter().map(|t| t.rel.as_str()).collect();
        assert_eq!(rels, vec!["deep.txt", "one.txt"]);
        assert!(plan.dirs.is_empty());

        std::fs::remove_dir_all(&src).ok();
        std::fs::remove_dir_all(&dst).ok();
    }

    #[test]
    fn test_include_filter_descends_unmatched_dirs() {
        let src = tmp("incl");
        let dst = tmp("incl-dst");
        std::fs::create_dir_all(src.join("docs")).unwrap();
        std::fs::create_dir_all(src.join("media")).unwrap();
        std::fs::write(src.join("docs/report.txt"), b"r").unwrap();
        std::fs::write(src.join("media/song.mp3"), b"s").unwrap();

        let o = Options { includes: vec!["*.txt".to_string()], ..opts() };
        let plan = build(&[src.clone()], &dst, &o).unwrap();
        let rels: Vec<&str> = plan.tasks.iter().map(|t| t.rel.as_str()).collect();
        assert_eq!(rels, vec!["docs/report.txt"]);

        std::fs::remove_dir_all(&src).ok();
        std::fs::remove_dir_all(&dst).ok();
    }

    #[test]
    fn test_exclude_and_single_file_source() {
        let src = tmp("excl");
        let dst = tmp("excl-dst");
        std::fs::create_dir_all(src.join("cache")).unwrap();
        std::fs::write(src.join("cache/junk.tmp"), b"j").unwrap();
        std::fs::write(src.join("keep.txt"), b"k").unwrap();

        let o = Options { excludes: vec!["*.tmp".to_string()], ..opts() };
        let plan = build(&[src.clone()], &dst, &o).unwrap();
        let rels: Vec<&str> = plan.tasks.iter().map(|t| t.rel.as_str()).collect();
        assert_eq!(rels, vec!["keep.txt"]);

        // single file source
        let plan2 = build(&[src.join("keep.txt")], &dst, &opts()).unwrap();
        assert_eq!(plan2.tasks[0].rel, "keep.txt");
        assert_eq!(plan2.tasks[0].dst, dst.join("keep.txt"));

        std::fs::remove_dir_all(&src).ok();
        std::fs::remove_dir_all(&dst).ok();
    }
}
