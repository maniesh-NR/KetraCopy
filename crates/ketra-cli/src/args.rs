//! Hand-rolled argument parser (keeps the binary dependency-free).

use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyArg {
    Off,
    Inline,
    Manifest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictArg {
    Overwrite,
    Newer,
    Skip,
    Rename,
    Ask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressArg {
    Auto,
    Plain,
    Json,
    Quiet,
}

#[derive(Debug, Clone)]
pub struct CommonArgs {
    pub dest: PathBuf,
    pub verify: VerifyArg,
    pub limit_bps: u64,
    pub net_streams: usize,
    pub conflict: ConflictArg,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub flatten: bool,
    pub preserve_times: bool,
    pub preserve_attrs: bool,
    pub sparse: bool,
    pub clone: bool,
    pub resume: bool,
    pub progress: ProgressArg,
    pub report: Option<PathBuf>,
    pub yes: bool,
    pub dry_run: bool,
}

impl Default for CommonArgs {
    fn default() -> Self {
        Self {
            dest: PathBuf::new(),
            verify: VerifyArg::Inline,
            limit_bps: 0,
            net_streams: 0,
            conflict: ConflictArg::Newer,
            include: vec![],
            exclude: vec![],
            flatten: false,
            preserve_times: true,
            preserve_attrs: true,
            sparse: true,
            clone: true,
            resume: true,
            progress: ProgressArg::Auto,
            report: None,
            yes: false,
            dry_run: false,
        }
    }
}

#[derive(Debug)]
pub enum Command {
    Copy { sources: Vec<PathBuf>, common: CommonArgs },
    Move { sources: Vec<PathBuf>, common: CommonArgs },
    Test { sources: Vec<PathBuf>, hash: String, common: CommonArgs },
    Manifest { root: PathBuf, out: Option<PathBuf> },
    Verify { root: PathBuf, manifest: Option<PathBuf>, quiet: bool },
    Bench { path: Option<PathBuf>, size: u64 },
    Peers { wait_secs: u64 },
    Send { files: Vec<PathBuf>, peer: String, dest_dir: Option<String>, parallel: u8, chunk_size: u64, checksum: String, no_manifest: bool },
    Receive { dest_dir: PathBuf, port: u16, auto_accept: bool, alias: String, model: String },
    ClipPut { paths: Vec<PathBuf>, cut: bool },
    ClipGet { json: bool },
    ClipPaste { dest: PathBuf, common: CommonArgs },
    ShellInstall { targets: Vec<String> },
    ShellUninstall { targets: Vec<String> },
    ShellStatus,
    /// Hidden: X11 clipboard selection owner (Linux). Started detached by
    /// `ketra clip put` so the clipboard outlives the short-lived CLI call.
    ClipServer { file: PathBuf },
    ConfigShow,
    Version,
    Help,
}

#[derive(Debug)]
pub struct UsageError(pub String);

fn need_value(args: &[String], i: &mut usize, flag: &str) -> Result<String, UsageError> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| UsageError(format!("{flag} requires a value")))
}

fn parse_size(s: &str) -> Result<u64, UsageError> {
    let t = s.trim().to_ascii_uppercase();
    let (num, mult) = if let Some(v) = t.strip_suffix('K') {
        (v, 1024u64)
    } else if let Some(v) = t.strip_suffix('M') {
        (v, 1024 * 1024)
    } else if let Some(v) = t.strip_suffix('G') {
        (v, 1024 * 1024 * 1024)
    } else if let Some(v) = t.strip_suffix('B') {
        // plain bytes ("100B")
        (v, 1)
    } else {
        (t.as_str(), 1)
    };
    num.trim()
        .parse::<u64>()
        .map(|v| v * mult)
        .map_err(|_| UsageError(format!("bad size: {s}")))
}

/// Parse the flags shared by copy/move and clip paste.
fn parse_common(rest: &[String], sources: &mut Vec<PathBuf>, common: &mut CommonArgs) -> Result<(), UsageError> {
    let mut i = 0;
    while i < rest.len() {
        let a = &rest[i];
        let mut val = |name: &str| -> Result<String, UsageError> {
            need_value(rest, &mut i, name)
        };
        match a.as_str() {
            "-d" | "--dest" => common.dest = PathBuf::from(val(a)?),
            "--verify" => {
                common.verify = match val(a)?.as_str() {
                    "off" => VerifyArg::Off,
                    "inline" | "byte" | "quick" => VerifyArg::Inline,
                    "manifest" | "full" | "hash" => VerifyArg::Manifest,
                    other => return Err(UsageError(format!("--verify: unknown level {other} (off|inline|manifest)"))),
                };
            }
            "--limit" => common.limit_bps = parse_size(&val(a)?)?,
            "--net-streams" => common.net_streams = val(a)?.parse().map_err(|_| UsageError("--net-streams N".into()))?,
            "--conflict" => {
                common.conflict = match val(a)?.as_str() {
                    "overwrite" => ConflictArg::Overwrite,
                    "newer" => ConflictArg::Newer,
                    "skip" => ConflictArg::Skip,
                    "rename" => ConflictArg::Rename,
                    "ask" => ConflictArg::Ask,
                    other => return Err(UsageError(format!("--conflict: unknown policy {other}"))),
                };
            }
            "--include" => common.include.push(val(a)?),
            "--exclude" => common.exclude.push(val(a)?),
            "--flatten" => common.flatten = true,
            "--no-times" => common.preserve_times = false,
            "--no-attrs" => common.preserve_attrs = false,
            "--sparse" => match val(a)?.as_str() {
                "on" | "auto" => common.sparse = true,
                "off" => common.sparse = false,
                other => return Err(UsageError(format!("--sparse: on|off (got {other})"))),
            },
            "--no-clone" => common.clone = false,
            "--no-resume" => common.resume = false,
            "--progress" => {
                common.progress = match val(a)?.as_str() {
                    "auto" => ProgressArg::Auto,
                    "plain" => ProgressArg::Plain,
                    "json" | "jsonl" => ProgressArg::Json,
                    "quiet" | "none" => ProgressArg::Quiet,
                    other => return Err(UsageError(format!("--progress: unknown mode {other}"))),
                };
            }
            "--report" => common.report = Some(PathBuf::from(val(a)?)),
            "-y" | "--yes" => common.yes = true,
            "--dry-run" => common.dry_run = true,
            other if other.starts_with('-') => {
                return Err(UsageError(format!("unknown flag {other}")))
            }
            _ => sources.push(PathBuf::from(&rest[i])),
        }
        i += 1;
    }
    Ok(())
}

pub fn parse(args: Vec<String>) -> Result<Command, UsageError> {
    let mut it = args.into_iter();
    let cmd = it.next().ok_or_else(|| UsageError("no command given".into()))?;
    let rest: Vec<String> = it.collect();
    match cmd.as_str() {
        "copy" | "move" | "test" => {
            let mut sources = Vec::new();
            let mut common = CommonArgs::default();
            let mut hash = "sha256".to_string();
            let mut i = 0;
            // `test` also accepts --hash <algo>; reuse the common loop then
            // pick it out below.
            // `--hash` belongs to `test` only; peel it off before the
            // common flag loop sees an unknown flag.
            let mut rest2: Vec<String> = Vec::new();
            let mut j = 0;
            while j < rest.len() {
                if rest[j] == "--hash" {
                    hash = need_value(&rest, &mut j, "--hash")?;
                } else {
                    rest2.push(rest[j].clone());
                }
                j += 1;
            }
            parse_common(&rest2, &mut sources, &mut common)?;
            if common.dest.as_os_str().is_empty() && cmd != "test" {
                return Err(UsageError("missing required --dest/-d".into()));
            }
            if sources.is_empty() {
                return Err(UsageError("no source paths given".into()));
            }
            let c = cmd.as_str();
            Ok(match c {
                "copy" => Command::Copy { sources, common },
                "move" => Command::Move { sources, common },
                _ => Command::Test { sources, hash, common },
            })
        }
        "manifest" => {
            let mut root = None;
            let mut out = None;
            let mut i = 0;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--root" | "-r" => root = Some(PathBuf::from(need_value(&rest, &mut i, "--root")?)),
                    "--out" | "-o" => out = Some(PathBuf::from(need_value(&rest, &mut i, "--out")?)),
                    o => return Err(UsageError(format!("manifest: unknown flag {o}"))),
                }
                i += 1;
            }
            Ok(Command::Manifest {
                root: root.ok_or_else(|| UsageError("manifest: missing --root".into()))?,
                out,
            })
        }
        "verify" => {
            let mut root = None;
            let mut manifest = None;
            let mut quiet = false;
            let mut i = 0;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--root" | "-r" => root = Some(PathBuf::from(need_value(&rest, &mut i, "--root")?)),
                    "--manifest" | "-m" => manifest = Some(PathBuf::from(need_value(&rest, &mut i, "--manifest")?)),
                    "--quiet" | "-q" => quiet = true,
                    o => return Err(UsageError(format!("verify: unknown flag {o}"))),
                }
                i += 1;
            }
            Ok(Command::Verify {
                root: root.ok_or_else(|| UsageError("verify: missing --root".into()))?,
                manifest,
                quiet,
            })
        }
        "bench" => {
            let mut path = None;
            let mut size = 128u64 << 20;
            let mut i = 0;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--path" => path = Some(PathBuf::from(need_value(&rest, &mut i, "--path")?)),
                    "--size" => size = parse_size(&need_value(&rest, &mut i, "--size")?)?,
                    o => return Err(UsageError(format!("bench: unknown flag {o}"))),
                }
                i += 1;
            }
            Ok(Command::Bench { path, size })
        }
        "config" => Ok(Command::ConfigShow),
        "peers" => {
            let mut wait_secs = 5u64;
            let mut i = 0;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--wait" => wait_secs = need_value(&rest, &mut i, "--wait")?.parse().unwrap_or(5),
                    o => return Err(UsageError(format!("peers: unknown flag {o}"))),
                }
                i += 1;
            }
            Ok(Command::Peers { wait_secs })
        }
        "send" => {
            let mut files: Vec<PathBuf> = Vec::new();
            let mut peer: Option<String> = None;
            let mut dest_dir: Option<String> = None;
            let mut parallel: u8 = 4;
            let mut chunk_size: u64 = 4 * 1024 * 1024;
            let mut checksum = "sha256".to_string();
            let mut no_manifest = false;
            let mut i = 0;
            while i < rest.len() {
                let a = &rest[i];
                if a == "--to" || a == "-t" {
                    peer = Some(need_value(&rest, &mut i, "--to")?);
                } else if a == "--dest" || a == "-d" {
                    dest_dir = Some(need_value(&rest, &mut i, "--dest")?);
                } else if a == "--parallel" {
                    parallel = need_value(&rest, &mut i, "--parallel")?.parse().unwrap_or(4);
                } else if a == "--chunk" {
                    chunk_size = parse_size(&need_value(&rest, &mut i, "--chunk")?)?;
                } else if a == "--checksum" {
                    checksum = need_value(&rest, &mut i, "--checksum")?;
                } else if a == "--no-manifest" {
                    no_manifest = true;
                } else if a.starts_with('-') {
                    return Err(UsageError(format!("send: unknown flag {a}")));
                } else {
                    files.push(PathBuf::from(a));
                }
                i += 1;
            }
            if files.is_empty() {
                return Err(UsageError("send: at least one file path is required".into()));
            }
            let peer = peer.ok_or_else(|| UsageError("send: --to <peer-fingerprint-or-alias> is required".into()))?;
            Ok(Command::Send { files, peer, dest_dir, parallel, chunk_size, checksum, no_manifest })
        }
        "receive" => {
            let mut dest_dir: Option<PathBuf> = None;
            let mut port: u16 = 53317;
            let mut auto_accept = true;
            let mut alias = std::env::var("USER").unwrap_or_else(|_| "Ketra".into());
            let mut model = hostname_string();
            let mut i = 0;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--dest" | "-d" => dest_dir = Some(PathBuf::from(need_value(&rest, &mut i, "--dest")?)),
                    "--port" | "-p" => port = need_value(&rest, &mut i, "--port")?.parse().unwrap_or(53317),
                    "--ask" => auto_accept = false,
                    "--alias" => alias = need_value(&rest, &mut i, "--alias")?,
                    "--model" => model = need_value(&rest, &mut i, "--model")?,
                    o => return Err(UsageError(format!("receive: unknown flag {o}"))),
                }
                i += 1;
            }
            let dest_dir = dest_dir.ok_or_else(|| UsageError("receive: --dest <dir> is required".into()))?;
            Ok(Command::Receive { dest_dir, port, auto_accept, alias, model })
        }
        "clip" => {
            let sub = rest.first().map(String::as_str).ok_or_else(|| {
                UsageError("clip: missing subcommand (put|get|paste)".into())
            })?;
            match sub {
                "put" => {
                    let mut paths = Vec::new();
                    let mut cut = false;
                    let mut i = 1;
                    while i < rest.len() {
                        match rest[i].as_str() {
                            "--cut" | "-x" => cut = true,
                            a if a.starts_with('-') => return Err(UsageError(format!("clip put: unknown flag {a}"))),
                            _ => paths.push(PathBuf::from(&rest[i])),
                        }
                        i += 1;
                    }
                    if paths.is_empty() {
                        return Err(UsageError("clip put: at least one path is required".into()));
                    }
                    Ok(Command::ClipPut { paths, cut })
                }
                "get" => {
                    let mut json = false;
                    for a in &rest[1..] {
                        match a.as_str() {
                            "--json" => json = true,
                            other => return Err(UsageError(format!("clip get: unknown flag {other}"))),
                        }
                    }
                    Ok(Command::ClipGet { json })
                }
                "paste" => {
                    let mut common = CommonArgs::default();
                    let mut i = 1;
                    let rest2: Vec<String> = rest[1..].to_vec();
                    let mut sources = Vec::new();
                    parse_common(&rest2, &mut sources, &mut common)?;
                    if !sources.is_empty() {
                        return Err(UsageError("clip paste: takes no source paths (files come from the clipboard)".into()));
                    }
                    if common.dest.as_os_str().is_empty() {
                        return Err(UsageError("clip paste: missing required --dest/-d".into()));
                    }
                    Ok(Command::ClipPaste { dest: common.dest.clone(), common })
                }
                other => Err(UsageError(format!("clip: unknown subcommand '{other}' (put|get|paste)"))),
            }
        }
        "shell" => {
            let sub = rest.first().map(String::as_str).ok_or_else(|| {
                UsageError("shell: missing subcommand (install|uninstall|status)".into())
            })?;
            let targets = |i0: usize| -> Vec<String> {
                let mut t: Vec<String> = Vec::new();
                let mut i = i0;
                while i < rest.len() {
                    let a = rest[i].as_str();
                    match a {
                        "--help" | "-h" => { t.push("help".into()); }
                        other if !other.starts_with('-') => t.push(other.to_string()),
                        _ => {}
                    }
                    i += 1;
                }
                t
            };
            match sub {
                "install" => Ok(Command::ShellInstall { targets: targets(1) }),
                "uninstall" => Ok(Command::ShellUninstall { targets: targets(1) }),
                "status" => Ok(Command::ShellStatus),
                other => Err(UsageError(format!("shell: unknown subcommand '{other}' (install|uninstall|status)"))),
            }
        }
        "__clip-server" => {
            // Hidden: internal clipboard selection owner (Linux).
            let mut file = None;
            let mut i = 0;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--file" => file = Some(PathBuf::from(need_value(&rest, &mut i, "--file")?)),
                    o => return Err(UsageError(format!("__clip-server: unknown flag {o}"))),
                }
                i += 1;
            }
            Ok(Command::ClipServer {
                file: file.ok_or_else(|| UsageError("__clip-server: missing --file".into()))?,
            })
        }
        "version" | "--version" | "-V" => Ok(Command::Version),
        "help" | "--help" | "-h" => Ok(Command::Help),
        other => Err(UsageError(format!(
            "unknown command '{other}' (try: copy, move, test, manifest, verify, bench, clip, shell, peers, send, receive, help)"
        ))),
    }
}

pub fn usage() -> &'static str {
    r#"ketra — KetraCopy command line (clean-room verified file transfer)

USAGE:
  ketra copy|move <src>... -d <dest> [options]
  ketra test <src>... [--hash sha256]
  ketra manifest --root <dir> [--out <file>]
  ketra verify  --root <dir> [--manifest <file>] [--quiet]
  ketra bench   [--path <dir>] [--size 128M]
  ketra clip put <path>... [--cut]     put files on the OS clipboard
  ketra clip get [--json]              print clipboard files + copy/cut mode
  ketra clip paste -d <dir> [options]  transfer clipboard files (cut = move)
  ketra shell install [targets]        register Explorer/Finder/Nemo menus
  ketra shell uninstall [targets]
  ketra shell status
  ketra peers   [--wait 5]
  ketra send    <file>... --to <peer> [--dest dir] [--parallel 4] [--chunk 4M]
  ketra receive --dest <dir> [--port 53317] [--ask] [--alias <name>]
  ketra config  (show effective defaults)

OPTIONS (copy/move/paste):
  -d, --dest <dir>          Destination directory (required)
      --verify <level>      off | inline (default, byte compare) | manifest (SHA-256)
      --limit <n>           Speed limit, e.g. 10M (bytes/sec), 0 = unlimited
      --net-streams <n>     Parallel ranged streams for network mounts, 0 = off
      --conflict <policy>   overwrite | newer (default) | skip | rename | ask
      --include <glob>      Include filter (repeatable)
      --exclude <glob>      Exclude filter (repeatable)
      --flatten             Flatten directory structure
      --no-times            Do not preserve modification time
      --no-attrs            Do not preserve permission bits
      --sparse on|off       Sparse hole handling (default on)
      --no-clone            Disable reflink/copy_file_range fast paths
      --no-resume           Ignore a previous interrupted run's journal
      --progress <mode>     auto | plain | json | quiet
      --report <file>       Write JSON error report
      -y, --yes             Never prompt (non-interactive)
      --dry-run             Show the plan and exit

SHELL INTEGRATION (ketra shell install [targets]):
  windows:  Explorer context menus  (copy / cut / paste / copy-to / move-to / test)
  linux:    nemo nautilus caja dolphin thunar pick shortcuts all
  macos:    finder-services         (Finder Quick Actions / Services)
  Running `ketra shell install` with no targets installs the default set
  for the detected platform. `shortcuts` additionally registers the
  platform keyboard shortcuts (Windows needs the optional AutoHotkey
  helper; macOS assigns shortcuts in System Settings).

EXIT CODES: 0 ok | 1 file errors | 2 canceled | 3 usage | 4 verify failed

EXAMPLES:
  ketra copy ~/Photos -d /mnt/backup --verify inline
  ketra move *.iso -d /media/disk --verify manifest
  ketra copy . -d /tmp/x --exclude "*.tmp" --exclude ".git/**"
  ketra clip put ~/report.pdf ~/photos --cut   (Explorer: paste works natively)
  ketra clip paste -d ~/Documents              (uses Explorer/Nemo clipboard)
  ketra shell install                          (wire up right-click menus)
  ketra peers                       (list LocalSend/Ketra peers on the LAN)
  ketra send report.pdf -t <fp>     (send to a peer by fingerprint)
  ketra receive -d ~/Downloads      (listen on :53317, accept incoming)
  ketra verify --root /mnt/backup"#
}

/// Best-effort hostname for the device model string.
pub fn hostname_string_pub() -> String {
    hostname_string()
}

fn hostname_string() -> String {
    #[cfg(unix)]
    {
        use std::ffi::CStr;
        let mut buf = [0i8; 256];
        let r = unsafe { libc_gethostname(buf.as_mut_ptr(), buf.len()) };
        if r == 0 {
            let cstr = unsafe { CStr::from_ptr(buf.as_ptr()) };
            return cstr.to_string_lossy().to_string();
        }
    }
    std::env::var("HOSTNAME").unwrap_or_else(|_| "Ketra".into())
}

#[cfg(unix)]
extern "C" {
    #[link_name = "gethostname"]
    fn libc_gethostname(buf: *mut i8, len: usize) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_copy_requires_dest() {
        let e = parse(v(&["copy", "a.txt"])).unwrap_err();
        assert!(e.0.contains("--dest"));
        let c = parse(v(&["copy", "a.txt", "-d", "out"])).unwrap();
        match c {
            Command::Copy { sources, common } => {
                assert_eq!(sources.len(), 1);
                assert_eq!(common.verify, VerifyArg::Inline);
                assert_eq!(common.conflict, ConflictArg::Newer);
            }
            _ => panic!("expected copy"),
        }
    }

    #[test]
    fn test_verify_aliases() {
        let c = parse(v(&["copy", "a", "-d", "b", "--verify", "manifest"])).unwrap();
        match c {
            Command::Copy { common, .. } => assert_eq!(common.verify, VerifyArg::Manifest),
            _ => panic!(),
        }
        let c = parse(v(&["copy", "a", "-d", "b", "--verify", "quick"])).unwrap();
        match c {
            Command::Copy { common, .. } => assert_eq!(common.verify, VerifyArg::Inline),
            _ => panic!(),
        }
    }

    #[test]
    fn test_move_and_flags() {
        let c = parse(v(&["move", "a", "-d", "b", "--flatten", "--limit", "10M",
                          "--conflict", "rename", "--sparse", "off", "--no-clone",
                          "-y", "--dry-run", "--exclude", "*.tmp"])).unwrap();
        match c {
            Command::Move { sources, common } => {
                assert_eq!(sources.len(), 1);
                assert!(common.flatten && common.yes && common.dry_run);
                assert_eq!(common.limit_bps, 10 * 1024 * 1024);
                assert_eq!(common.conflict, ConflictArg::Rename);
                assert!(!common.sparse);
                assert!(!common.clone);
                assert_eq!(common.exclude, vec!["*.tmp".to_string()]);
            }
            _ => panic!("expected move"),
        }
    }

    #[test]
    fn test_test_hash() {
        let c = parse(v(&["test", "a", "--hash", "xxh64"])).unwrap();
        match c {
            Command::Test { hash, .. } => assert_eq!(hash, "xxh64"),
            _ => panic!("expected test"),
        }
    }

    #[test]
    fn test_clip_put() {
        let c = parse(v(&["clip", "put", "a", "b", "--cut"])).unwrap();
        match c {
            Command::ClipPut { paths, cut } => {
                assert_eq!(paths.len(), 2);
                assert!(cut);
            }
            _ => panic!("expected clip put"),
        }
        assert!(parse(v(&["clip", "put"])).is_err());
    }

    #[test]
    fn test_clip_paste_needs_dest() {
        let e = parse(v(&["clip", "paste"])).unwrap_err();
        assert!(e.0.contains("--dest"));
        let c = parse(v(&["clip", "paste", "-d", "out", "-y"])).unwrap();
        match c {
            Command::ClipPaste { dest, common } => {
                assert_eq!(dest, PathBuf::from("out"));
                assert!(common.yes);
            }
            _ => panic!("expected clip paste"),
        }
    }

    #[test]
    fn test_shell_targets() {
        let c = parse(v(&["shell", "install", "nemo", "nautilus"])).unwrap();
        match c {
            Command::ShellInstall { targets } => assert_eq!(targets, vec!["nemo", "nautilus"]),
            _ => panic!("expected shell install"),
        }
    }

    #[test]
    fn test_size_suffixes() {
        assert_eq!(parse_size("10M").unwrap(), 10 * 1024 * 1024);
        assert_eq!(parse_size("1G").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_size("512K").unwrap(), 512 * 1024);
        assert_eq!(parse_size("100B").unwrap(), 100);
        assert_eq!(parse_size("7").unwrap(), 7);
        assert!(parse_size("x").is_err());
    }
}
