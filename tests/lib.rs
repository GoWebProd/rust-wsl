mod common;

use common::{assert_bytes_eq, cases, diff_bytes, fixture, fixture_dirs, labels};
use rust_wsl::{Rule, check, fix, same_tokens};

fn crlf(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len() + b.len() / 16);
    for &x in b {
        if x == b'\n' {
            out.push(b'\r');
        }

        out.push(x);
    }

    out
}

fn without_final_newline(b: &[u8]) -> &[u8] {
    b.strip_suffix(b"\n").expect("fixture ends with a newline")
}

#[test]
fn crlf_input_gets_crlf_blank_lines_in_every_fixture() {
    let mut failures = Vec::new();

    for dir in fixture_dirs() {
        for c in cases(&dir) {
            let src = crlf(&c.input);

            let got = labels(&check(&src).unwrap().unwrap());
            let want = c.violations.clone().unwrap_or_default();

            if got != want {
                failures.push(format!("{}: check {got:?}, expected {want:?}", c.name));
            }

            let text = fix(&src).unwrap().unwrap().text;
            let want = c.expected.as_deref().map(crlf);

            if text != want {
                let d = match (&text, &want) {
                    (Some(t), Some(w)) => diff_bytes(t, w).unwrap_or_default(),
                    _ => format!(
                        "changed: {}, expected change: {}",
                        text.is_some(),
                        want.is_some()
                    ),
                };

                failures.push(format!("{}: CRLF fix differs from the LF one\n{d}", c.name));
            }
        }
    }

    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn file_without_final_newline_is_fixed_and_stays_without_it() {
    let input = fixture("expr-max/r1_only.input");
    let expected = fixture("expr-max/r1_only.expected");

    for (src, want) in [
        (
            without_final_newline(&input).to_vec(),
            without_final_newline(&expected).to_vec(),
        ),
        (
            crlf(without_final_newline(&input)),
            crlf(without_final_newline(&expected)),
        ),
    ] {
        let fixed = fix(&src).unwrap().unwrap().text.expect("expected a change");
        assert_bytes_eq(&fixed, &want, "fix without final newline");

        assert_eq!(fix(&fixed).unwrap().unwrap().text, None, "not idempotent");
        assert!(check(&fixed).unwrap().unwrap().is_empty());
    }
}

#[test]
fn check_after_fix_reports_only_r8_in_every_fixture() {
    for dir in fixture_dirs() {
        for c in cases(&dir) {
            let left = check(c.output()).unwrap().unwrap();

            assert!(
                left.iter().all(|v| v.rule == Rule::UseInFn),
                "{}: {:?} remain after fix",
                c.name,
                labels(&left)
            );
        }
    }
}

#[test]
fn unparsable_source_is_none_for_check_and_fix() {
    for src in [
        &b"fn f( {\n    run();\n    let a = 1;\n"[..],
        b"unsafe extern \"C\" {\n    pub safe fn f();\n}\n",
    ] {
        assert!(check(src).unwrap().is_none());
        assert!(fix(src).unwrap().is_none());
    }
}

#[test]
fn trailing_comment_belongs_to_previous_statement() {
    let src = b"fn f() {\n    run(); // note\n    let a = 1;\n    run(a);\n}\n";

    let v = check(src).unwrap().unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].row, 2);
}

#[test]
fn same_tokens_ignores_whitespace_between_tokens() {
    let dirty = fixture("expr-max/r1_only.input");
    let clean = fixture("expr-max/r1_only.expected");

    assert!(same_tokens(&dirty, &clean).unwrap());
    assert!(same_tokens(b"fn f() { run(); }", b"fn  f( )\n{\n\n run ( ) ;\n}").unwrap());
}

#[test]
fn same_tokens_sees_changes_in_tokens_literals_and_comments() {
    let pairs: &[(&[u8], &[u8])] = &[
        (b"fn f() { let a = 1; }", b"fn f() { let a = 2; }"),
        (
            b"fn f() { let s = \"a\nb\"; }",
            b"fn f() { let s = \"a\n\nb\"; }",
        ),
        (
            b"fn f() { let s = \"\n\"; }",
            b"fn f() { let s = \"\n\n\"; }",
        ),
        (
            b"fn f() { let s = r#\"\n \"#; }",
            b"fn f() { let s = r#\"\n\n \"#; }",
        ),
        (b"fn f() { // a\n}", b"fn f() { // b\n}"),
        (b"fn f() { /* a */ }", b"fn f() { /* a\n */ }"),
    ];

    for (a, b) in pairs {
        assert!(
            !same_tokens(a, b).unwrap(),
            "{:?} and {:?} must differ",
            String::from_utf8_lossy(a),
            String::from_utf8_lossy(b)
        );
    }
}
