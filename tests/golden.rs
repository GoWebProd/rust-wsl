mod common;

use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use common::{Scratch, cases, diff_bytes, fixture_dirs, fixtures_root, labels};
use rust_wsl::{Rule, check, fix};

// Не проходят rustfmt: `one_line` раскладывает rustfmt по строкам, `no_semicolon` он дополняет `;`,
// из `empty_stmt_loop` убирает `;` после loop/while/for.
const NOT_RUSTFMT_STABLE: &[&str] = &[
    "exceptions/one_line",
    "jump/no_semicolon",
    "exceptions/empty_stmt_loop",
];

fn golden(dir: &str) {
    let mut failures = Vec::new();

    for c in cases(dir) {
        let name = &c.name;

        let violations = check(&c.input)
            .unwrap_or_else(|e| panic!("{name}: check failed: {e:#}"))
            .unwrap_or_else(|| panic!("{name}: fixture does not parse"));

        let got = labels(&violations);
        match &c.violations {
            None if !got.is_empty() => {
                failures.push(format!("{name}: no .violations, but check reports {got:?}"));
            }
            Some(want) if *want != got => {
                failures.push(format!(
                    "{name}: check reports {got:?}, .violations says {want:?}"
                ));
            }
            _ => {}
        }

        let fixed = fix(&c.input)
            .unwrap_or_else(|e| panic!("{name}: fix failed: {e:#}"))
            .unwrap_or_else(|| panic!("{name}: fixture does not parse"));

        match (&fixed.text, &c.expected) {
            (Some(text), Some(want)) => {
                if let Some(d) = diff_bytes(text, want) {
                    failures.push(format!("{name}\n{d}"));
                }

                if *want == c.input {
                    failures.push(format!("{name}: .expected equals .input, delete it"));
                }
            }
            (None, Some(_)) => {
                failures.push(format!("{name}: .expected exists, but fix changes nothing"))
            }
            (Some(text), None) => failures.push(format!(
                "{name}: fix rewrites the file, but there is no .expected\n{}",
                diff_bytes(text, &c.input).unwrap_or_default()
            )),
            (None, None) => {}
        }

        // Без автоисправления остаётся ровно R8, и строки у него — по исходному тексту.
        let r8: Vec<_> = violations
            .iter()
            .filter(|v| v.rule == Rule::UseInFn)
            .copied()
            .collect();
        if labels(&fixed.leftover) != labels(&r8) {
            failures.push(format!(
                "{name}: leftover {:?}, check reports R8 at {:?}",
                labels(&fixed.leftover),
                labels(&r8)
            ));
        }
    }

    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

fn idempotent(dir: &str) {
    for c in cases(dir) {
        let f = fix(c.output()).unwrap().unwrap();

        assert_eq!(f.text, None, "{}: second run changed the output", c.name);
    }
}

fn rustfmt_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();

    let ok = *AVAILABLE.get_or_init(|| {
        Command::new("rustfmt")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    });
    if !ok {
        assert!(
            std::env::var("RUST_WSL_SKIP_RUSTFMT").is_ok_and(|v| v == "1"),
            "rustfmt is not in PATH; install it or set RUST_WSL_SKIP_RUSTFMT=1 to skip the stability check"
        );
        eprintln!("rustfmt is not in PATH: skipping the stability check (RUST_WSL_SKIP_RUSTFMT=1)");
    }

    ok
}

fn rustfmt_stable(dir: &str) {
    if !rustfmt_available() {
        return;
    }

    let scratch = Scratch::new();
    let files: Vec<_> = cases(dir)
        .iter()
        .filter(|c| !NOT_RUSTFMT_STABLE.contains(&c.name.as_str()))
        .map(|c| scratch.file(format!("{}.rs", c.stem), c.output()))
        .collect();

    if files.is_empty() {
        return;
    }

    let config = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/backend/rustfmt.toml");
    assert!(
        config.is_file(),
        "no rustfmt config at {}",
        config.display()
    );

    let out = Command::new("rustfmt")
        .args(["--edition", "2024", "--check", "--color", "never"])
        .arg("--config-path")
        .arg(&config)
        .args(&files)
        .output()
        .expect("run rustfmt");

    assert!(
        out.status.success(),
        "rustfmt would change the output in {dir} (file names are <case>.rs):\n--- stdout\n{}\n--- stderr\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

macro_rules! fixture_tests {
    ($($m:ident => $dir:literal),* $(,)?) => {
        $(
            mod $m {
                #[test]
                fn golden() { super::golden($dir) }
                #[test]
                fn idempotent() { super::idempotent($dir) }
                #[test]
                fn rustfmt_stable() { super::rustfmt_stable($dir) }
            }
        )*

        const REGISTERED: &[&str] = &[$($dir),*];
    };
}

// decl/same_kind_use несёт R8 по построению: R7 для `use` проверяется только в блоках, а блок — тело fn.
fixture_tests! {
    assign => "assign",
    block => "block",
    block_max => "block-max",
    expr_after => "expr-after",
    expr_unrelated => "expr-unrelated",
    expr_max => "expr-max",
    jump => "jump",
    tail => "tail",
    decl => "decl",
    block_kinds => "block-kinds",
    exceptions => "exceptions",
    use_in_fn => "use-in-fn",
    comment => "comment",
    strings => "strings",
}

#[test]
fn every_fixture_dir_is_registered() {
    let mut registered: Vec<&str> = REGISTERED.to_vec();
    registered.sort();

    assert_eq!(fixture_dirs(), registered);
}

#[test]
fn rustfmt_exceptions_point_at_existing_cases() {
    for name in NOT_RUSTFMT_STABLE {
        let (dir, stem) = name.split_once('/').unwrap();
        let input = fixtures_root().join(dir).join(format!("{stem}.input"));

        assert!(
            fs::metadata(&input).is_ok(),
            "{name}: no such fixture, drop it from NOT_RUSTFMT_STABLE"
        );
    }
}
