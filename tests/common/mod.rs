// Каждый тест-крейт подключает модуль целиком и использует только свою часть.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use rust_wsl::{Rule, Violation};

pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn run(args: &[&dyn AsRef<OsStr>]) -> Out {
    let o = Command::new(env!("CARGO_BIN_EXE_rust-wsl"))
        .args(args.iter().map(|a| a.as_ref()))
        .output()
        .expect("spawn rust-wsl");

    Out {
        code: o.status.code().expect("rust-wsl was killed by a signal"),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

/// Строка отчёта без перевода строки; `n` — номер строки с единицы, как в выводе.
pub fn line(path: &Path, n: usize, rule: Rule) -> String {
    Violation {
        rule,
        row: n - 1,
        line_start: 0,
    }
    .render(path)
}

pub fn lines(items: &[String]) -> String {
    items.iter().map(|l| format!("{l}\n")).collect()
}

/// Метки в формате `.violations`: `<строка с единицы> <метка правила>`.
pub fn labels(violations: &[Violation]) -> Vec<String> {
    violations
        .iter()
        .map(|v| format!("{} {}", v.row + 1, v.rule.label()))
        .collect()
}

fn show_line(l: Option<&[u8]>) -> String {
    l.map_or_else(
        || "<end of file>".to_owned(),
        |l| format!("{:?}", String::from_utf8_lossy(l)),
    )
}

pub fn diff_bytes(got: &[u8], want: &[u8]) -> Option<String> {
    if got == want {
        return None;
    }

    let mut g = got.split_inclusive(|b| *b == b'\n');
    let mut w = want.split_inclusive(|b| *b == b'\n');

    let mut n = 1;
    let (gl, wl) = loop {
        let (gl, wl) = (g.next(), w.next());
        if gl != wl {
            break (gl, wl);
        }

        n += 1;
    };

    Some(format!(
        "--- expected\n{}\n--- got\n{}\n--- first difference at line {n}\nexpected: {}\ngot:      {}",
        String::from_utf8_lossy(want),
        String::from_utf8_lossy(got),
        show_line(wl),
        show_line(gl),
    ))
}

#[track_caller]
pub fn assert_bytes_eq(got: &[u8], want: &[u8], what: &str) {
    if let Some(d) = diff_bytes(got, want) {
        panic!("{what}\n{d}");
    }
}

/// Временный каталог теста; удаляется в `Drop`, в том числе при панике.
pub struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    pub fn new() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);

        let thread = std::thread::current();
        let test: String = thread
            .name()
            .unwrap_or("test")
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();

        let dir = std::env::temp_dir().join(format!(
            "rust-wsl-{}-{test}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("create {}: {e}", dir.display()));

        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn path(&self, name: impl AsRef<Path>) -> PathBuf {
        self.dir.join(name)
    }

    pub fn file(&self, name: impl AsRef<Path>, content: &[u8]) -> PathBuf {
        let p = self.path(name);
        fs::write(&p, content).unwrap_or_else(|e| panic!("write {}: {e}", p.display()));

        p
    }

    /// Имена записей каталога по порядку: ловит забытые временные файлы.
    pub fn entries(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.dir)
            .unwrap_or_else(|e| panic!("read {}: {e}", self.dir.display()))
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();

        names
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        make_writable(&self.dir);

        if let Err(e) = fs::remove_dir_all(&self.dir) {
            eprintln!("cannot remove {}: {e}", self.dir.display());
        }
    }
}

// Права возвращаются до удаления: тест мог оставить read-only файл или каталог без записи.
fn make_writable(p: &Path) {
    let Ok(meta) = fs::symlink_metadata(p) else {
        return;
    };

    if meta.file_type().is_symlink() {
        return;
    }

    let mut perm = meta.permissions();

    #[cfg(unix)]
    std::os::unix::fs::PermissionsExt::set_mode(&mut perm, 0o700);
    #[cfg(not(unix))]
    perm.set_readonly(false);

    let _ = fs::set_permissions(p, perm);

    if meta.is_dir()
        && let Ok(entries) = fs::read_dir(p)
    {
        for e in entries.flatten() {
            make_writable(&e.path());
        }
    }
}

/// Под root проверки прав доступа не срабатывают; файл, созданный root, принадлежит uid 0.
#[cfg(unix)]
pub fn owned_by_root(created_file: &Path) -> bool {
    std::os::unix::fs::MetadataExt::uid(&fs::metadata(created_file).unwrap()) == 0
}

pub fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Файл фикстуры по пути вида `assign/violation.input`.
pub fn fixture(rel: &str) -> Vec<u8> {
    let p = fixtures_root().join(rel);

    fs::read(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

pub fn fixture_dirs() -> Vec<String> {
    let mut dirs: Vec<String> = fs::read_dir(fixtures_root())
        .expect("read fixtures root")
        .map(|e| e.unwrap())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    dirs.sort();

    dirs
}

/// Тройка `.input` / `.expected` / `.violations`; необязательны последние две.
pub struct Case {
    /// `<каталог>/<случай>`.
    pub name: String,
    pub stem: String,
    pub input: Vec<u8>,
    /// Нет файла — исправление ничего не меняет.
    pub expected: Option<Vec<u8>>,
    /// Нет файла — `check` ничего не находит; строки — как у [`labels`].
    pub violations: Option<Vec<String>>,
}

impl Case {
    /// Текст после исправления.
    pub fn output(&self) -> &[u8] {
        self.expected.as_deref().unwrap_or(&self.input)
    }
}

fn read_optional(p: &Path) -> Option<Vec<u8>> {
    match fs::read(p) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => panic!("read {}: {e}", p.display()),
    }
}

fn parse_violations(p: &Path) -> Option<Vec<String>> {
    let text = String::from_utf8(read_optional(p)?)
        .unwrap_or_else(|e| panic!("{}: not UTF-8: {e}", p.display()));

    let rows: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let ok = l
                .split_once(' ')
                .is_some_and(|(row, label)| row.parse::<usize>().is_ok() && !label.is_empty());
            assert!(ok, "{}: expected `<row> <label>`, got {l:?}", p.display());

            l.to_owned()
        })
        .collect();
    assert!(
        !rows.is_empty(),
        "{}: empty file, delete it (no violations)",
        p.display()
    );

    Some(rows)
}

/// Падает на файле без `.input` и на файле с неизвестным расширением.
pub fn cases(dir: &str) -> Vec<Case> {
    let d = fixtures_root().join(dir);

    let (mut inputs, mut others) = (BTreeSet::new(), Vec::new());

    for e in fs::read_dir(&d).unwrap_or_else(|e| panic!("read {}: {e}", d.display())) {
        let p = e.unwrap().path();

        let ext = p.extension().and_then(OsStr::to_str);
        let stem = p.file_stem().unwrap().to_string_lossy().into_owned();

        match ext {
            Some("input") => {
                inputs.insert(stem);
            }
            Some("expected" | "violations") => others.push((stem, p)),
            _ => panic!("{}: unexpected file in fixtures", p.display()),
        }
    }

    assert!(!inputs.is_empty(), "no fixtures in {dir}");

    for (stem, p) in &others {
        assert!(
            inputs.contains(stem),
            "{}: orphan, no {stem}.input next to it",
            p.display()
        );
    }

    inputs
        .into_iter()
        .map(|stem| {
            let base = d.join(&stem);

            Case {
                name: format!("{dir}/{stem}"),
                input: read_optional(&base.with_extension("input")).expect("listed above"),
                expected: read_optional(&base.with_extension("expected")),
                violations: parse_violations(&base.with_extension("violations")),
                stem,
            }
        })
        .collect()
}
