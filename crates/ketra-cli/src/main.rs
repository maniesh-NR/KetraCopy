//! ketra — the KetraCopy command line interface.

mod args;
mod bench;
mod clip;
mod lan;
mod manifest_cmd;
mod run_cmd;
mod shell;
mod sig;
mod view;

use args::Command;

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let code = match args::parse(raw) {
        Ok(Command::Copy { sources, common }) => run_cmd::run(run_cmd::TransferKind::Copy, sources, &common).code,
        Ok(Command::Move { sources, common }) => run_cmd::run(run_cmd::TransferKind::Move, sources, &common).code,
        Ok(Command::Test { sources, hash, .. }) => run_cmd::run(run_cmd::TransferKind::Test(hash), sources, &args::CommonArgs::default()).code,
        Ok(Command::Manifest { root, out }) => manifest_cmd::manifest_cmd(root, out),
        Ok(Command::Verify { root, manifest, quiet }) => manifest_cmd::verify_cmd(root, manifest, quiet),
        Ok(Command::Bench { path, size }) => bench::run(path, size),
        Ok(Command::Peers { wait_secs }) => lan::peers_cmd(wait_secs),
        Ok(Command::Send { files, peer, dest_dir, parallel, chunk_size, checksum, no_manifest }) => {
            lan::send_cmd(files, peer, dest_dir, parallel, chunk_size, checksum, no_manifest)
        }
        Ok(Command::Receive { dest_dir, port, auto_accept, alias, model }) => {
            lan::receive_cmd(dest_dir, port, auto_accept, alias, model)
        }
        Ok(Command::ClipPut { paths, cut }) => clip::put_cmd(paths, cut),
        Ok(Command::ClipGet { json }) => clip::get_cmd(json),
        Ok(Command::ClipPaste { dest, common }) => clip::paste_cmd(dest, &common),
        Ok(Command::ShellInstall { targets }) => shell::install_cmd(targets),
        Ok(Command::ShellUninstall { targets }) => shell::uninstall_cmd(targets),
        Ok(Command::ShellStatus) => shell::status_cmd(),
        Ok(Command::ClipServer { file }) => {
            #[cfg(all(unix, not(target_os = "macos")))]
            {
                let _ = &file;
                clip::x11::serve(&file)
            }
            #[cfg(not(all(unix, not(target_os = "macos"))))]
            {
                let _ = file;
                eprintln!("ketra: clipboard daemon is not available on this platform");
                3
            }
        }
        Ok(Command::ConfigShow) => {
            let path = ketra_core::settings::Settings::default_path();
            println!("settings file: {}", path.display());
            match ketra_core::settings::Settings::load(&path) {
                Ok(s) => {
                    println!(
                        "verify={} limit={} buffer={} clone={} direct_io={} theme={}",
                        s.verify.as_deref().unwrap_or("(default)"),
                        s.speed_limit_bps.map(|v| v.to_string()).unwrap_or_else(|| "unlimited".into()),
                        s.buffer.map(|v| v.to_string()).unwrap_or_else(|| "(default)".into()),
                        s.use_clone.map(|b| b.to_string()).unwrap_or_else(|| "(default)".into()),
                        s.direct_io.map(|b| b.to_string()).unwrap_or_else(|| "(default)".into()),
                        s.theme.as_deref().unwrap_or("(default)"),
                    );
                    0
                }
                Err(e) => {
                    eprintln!("ketra: {e}");
                    1
                }
            }
        }
        Ok(Command::Version) => {
            println!("ketra {} (KetraCopy engine {})", env!("CARGO_PKG_VERSION"), ketra_core::VERSION);
            0
        }
        Ok(Command::Help) => {
            println!("{}", args::usage());
            0
        }
        Err(args::UsageError(msg)) => {
            eprintln!("ketra: {msg}\n");
            eprintln!("{}", args::usage());
            3
        }
    };
    std::process::exit(code);
}
