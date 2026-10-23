use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use rust_wsl::{Violation, check, fix};

enum Status {
    Clean,
    Violations,
}

enum Sink {
    Stdout,
    Stderr,
}

fn report(violations: &[Violation], path: &Path, sink: Sink) -> Status {
    for v in violations {
        match sink {
            Sink::Stdout => println!("{}", v.render(path)),
            Sink::Stderr => eprintln!("{}", v.render(path)),
        }
    }

    if violations.is_empty() {
        Status::Clean
    } else {
        Status::Violations
    }
}

/// Хук (режим исправления) файл с ошибкой разбора пропускает: rustfmt упал бы раньше.
/// `--check` ничего не проверил, поэтому для него это ошибка.
fn unparsable(path: &Path, check_only: bool) -> Result<Status> {
    if check_only {
        bail!("{}: parse error, cannot check", path.display());
    }

    eprintln!("rust-wsl: {}: parse error, skipped", path.display());

    Ok(Status::Clean)
}

/// Пишет через временный файл рядом и `rename`, чтобы обрыв не оставил усечённый файл;
/// симлинк остаётся симлинком, запись идёт в его цель.
fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let target = path
        .canonicalize()
        .with_context(|| format!("resolve {}", path.display()))?;
    let dir = target.parent().context("path has no parent directory")?;
    let name = target.file_name().context("path has no file name")?;

    let mut tmp_name = OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(format!(".rust-wsl-{}.tmp", std::process::id()));

    let tmp = dir.join(tmp_name);
    let perm = fs::metadata(&target)
        .with_context(|| format!("stat {}", target.display()))?
        .permissions();

    // `rename` заменил бы файл без права записи; запрет владельца соблюдаем, как `fs::write`.
    if perm.readonly() {
        bail!("write {}: file is read-only", target.display());
    }

    let written = (|| -> Result<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .with_context(|| format!("create {}", tmp.display()))?;

        f.write_all(data)
            .with_context(|| format!("write {}", tmp.display()))?;
        f.sync_all()
            .with_context(|| format!("sync {}", tmp.display()))?;
        f.set_permissions(perm)
            .with_context(|| format!("chmod {}", tmp.display()))?;

        fs::rename(&tmp, &target)
            .with_context(|| format!("rename {} to {}", tmp.display(), target.display()))
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }

    written
}

fn process(path: &Path, check_only: bool) -> Result<Status> {
    let src = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if check_only {
        let Some(violations) = check(&src).with_context(|| path.display().to_string())? else {
            return unparsable(path, true);
        };

        return Ok(report(&violations, path, Sink::Stdout));
    }

    let Some(fixed) = fix(&src).with_context(|| path.display().to_string())? else {
        return unparsable(path, false);
    };

    if let Some(new) = &fixed.text {
        write_atomic(path, new)?;
    }

    // В режиме исправления stdout пуст, поэтому R8 идёт в stderr.
    Ok(report(&fixed.leftover, path, Sink::Stderr))
}

fn run() -> ExitCode {
    let mut check_only = false;

    let mut files = Vec::new();
    for a in std::env::args_os().skip(1) {
        match a.to_str() {
            Some("--check") => check_only = true,
            Some("-h" | "--help") => {
                println!("usage: rust-wsl [--check] <files...>");

                return ExitCode::SUCCESS;
            }
            Some(s) if s.starts_with("--") => {
                eprintln!("rust-wsl: unknown flag {s}");

                return ExitCode::from(2);
            }
            _ => files.push(PathBuf::from(a)),
        }
    }

    let (mut failed, mut found) = (false, false);
    for f in &files {
        match process(f, check_only) {
            Ok(Status::Violations) => found = true,
            Ok(Status::Clean) => {}
            Err(e) => {
                eprintln!("rust-wsl: {e:#}");

                failed = true;
            }
        }
    }

    if failed {
        ExitCode::from(2)
    } else if found {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn main() -> ExitCode {
    // Рекурсия по выражениям идёт на глубину вложенности кода; 64 МБ — запас на сгенерированные файлы.
    match std::thread::Builder::new().stack_size(64 << 20).spawn(run) {
        Ok(h) => h.join().unwrap_or_else(|_| ExitCode::from(2)),
        Err(e) => {
            eprintln!("rust-wsl: cannot start worker thread: {e}");

            ExitCode::from(2)
        }
    }
}
