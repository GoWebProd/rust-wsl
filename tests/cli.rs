mod common;

#[cfg(unix)]
use std::ffi::OsStr;
use std::fs;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::time::{Duration, SystemTime};

use common::{Scratch, assert_bytes_eq, fixture, line, lines, run};
use rust_wsl::Rule;

const R8_HINT: &str = "R8 use-in-fn: move this import to the top of the module";

fn dirty() -> Vec<u8> {
    fixture("expr-max/r1_only.input")
}

fn clean() -> Vec<u8> {
    fixture("expr-max/r1_only.expected")
}

// R1 на строке 3 и R8 на строке 7.
fn with_r8() -> Vec<u8> {
    fixture("use-in-fn/combined.input")
}

fn with_r8_fixed() -> Vec<u8> {
    fixture("use-in-fn/combined.expected")
}

#[test]
fn check_prints_path_line_rule_and_exits_1() {
    let s = Scratch::new();
    let p = s.file("dirty.rs", &dirty());

    let o = run(&[&"--check", &p]);
    assert_eq!(o.code, 1);
    assert_eq!(o.stdout, format!("{}:3: R1 assign\n", p.display()));
    assert_bytes_eq(&fs::read(&p).unwrap(), &dirty(), "--check must not write");
}

#[test]
fn check_on_clean_file_exits_0_silently() {
    let s = Scratch::new();
    let p = s.file("clean.rs", &clean());

    let o = run(&[&"--check", &p]);
    assert_eq!(o.code, 0);
    assert!(o.stdout.is_empty() && o.stderr.is_empty());
}

#[test]
fn check_may_follow_the_files_and_reports_each_in_order() {
    let s = Scratch::new();
    let group = s.file("group.rs", &fixture("comment/group.input"));
    let other = s.file("other.rs", &fixture("comment/other_rule.input"));

    let o = run(&[&group, &other, &"--check"]);
    assert_eq!(o.code, 1);
    assert_eq!(
        o.stdout,
        lines(&[
            line(&group, 3, Rule::Comment),
            line(&other, 3, Rule::Assign)
        ])
    );
}

#[test]
fn check_reports_group_violation_once_at_the_action_line() {
    let s = Scratch::new();
    let p = s.file("r4c.rs", &fixture("expr-max/field_group.input"));

    let o = run(&[&"--check", &p]);
    assert_eq!(o.code, 1);
    assert_eq!(o.stdout, lines(&[line(&p, 5, Rule::ExprMax)]));
}

#[test]
fn fix_rewrites_in_place_silently_and_leaves_no_temp_files() {
    let s = Scratch::new();
    let p = s.file("dirty.rs", &dirty());

    let o = run(&[&p]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.stdout.is_empty() && o.stderr.is_empty());
    assert_bytes_eq(&fs::read(&p).unwrap(), &clean(), "fixed file");
    assert_eq!(s.entries(), ["dirty.rs"]);
}

#[test]
fn unchanged_file_is_not_written() {
    let s = Scratch::new();
    let p = s.file("clean.rs", &clean());

    // Старое mtime: любая перезапись его сдвинет.
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    fs::File::options()
        .write(true)
        .open(&p)
        .unwrap()
        .set_modified(old)
        .unwrap();

    let before = fs::metadata(&p).unwrap();
    assert_eq!(before.modified().unwrap(), old);

    let o = run(&[&p]);
    assert_eq!(o.code, 0, "{}", o.stderr);

    let after = fs::metadata(&p).unwrap();
    assert_eq!(
        after.modified().unwrap(),
        old,
        "mtime changed: file was written"
    );

    // Запись идёт через rename, так что новый inode выдаёт любую перезапись.
    #[cfg(unix)]
    assert_eq!(
        after.ino(),
        before.ino(),
        "inode changed: file was replaced"
    );
}

#[cfg(unix)]
#[test]
fn fix_keeps_the_permission_bits() {
    let s = Scratch::new();
    let p = s.file("dirty.rs", &dirty());

    fs::set_permissions(&p, fs::Permissions::from_mode(0o640)).unwrap();

    assert_eq!(run(&[&p]).code, 0);
    assert_bytes_eq(&fs::read(&p).unwrap(), &clean(), "fixed file");
    assert_eq!(
        fs::metadata(&p).unwrap().permissions().mode() & 0o7777,
        0o640
    );
}

#[cfg(unix)]
#[test]
fn fix_writes_through_a_symlink_and_keeps_it_a_symlink() {
    let s = Scratch::new();
    let real = s.file("real.rs", &dirty());
    let link = s.path("link.rs");

    std::os::unix::fs::symlink(&real, &link).unwrap();

    let o = run(&[&link]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_link(&link).unwrap(), real);
    assert_bytes_eq(&fs::read(&real).unwrap(), &clean(), "symlink target");
    assert_eq!(s.entries(), ["link.rs", "real.rs"]);
}

#[cfg(unix)]
#[test]
fn unwritable_directory_exits_2_and_keeps_the_file() {
    let s = Scratch::new();
    let p = s.file("dirty.rs", &dirty());

    if common::owned_by_root(&p) {
        eprintln!("skipped: root ignores directory permissions");

        return;
    }

    fs::set_permissions(s.dir(), fs::Permissions::from_mode(0o555)).unwrap();

    let o = run(&[&p]);
    assert_eq!(o.code, 2);
    assert!(o.stderr.contains("rust-wsl:"), "{}", o.stderr);
    assert_bytes_eq(&fs::read(&p).unwrap(), &dirty(), "file must stay untouched");
    assert_eq!(s.entries(), ["dirty.rs"]);
}

#[cfg(unix)]
#[test]
fn read_only_file_with_violations_exits_2_and_keeps_the_file() {
    let s = Scratch::new();
    let p = s.file("ro.rs", &dirty());

    fs::set_permissions(&p, fs::Permissions::from_mode(0o444)).unwrap();

    let o = run(&[&p]);
    assert_eq!(o.code, 2);
    assert!(o.stderr.contains("read-only"), "{}", o.stderr);
    assert_bytes_eq(&fs::read(&p).unwrap(), &dirty(), "file must stay untouched");
    assert_eq!(s.entries(), ["ro.rs"]);
}

#[cfg(unix)]
#[test]
fn non_utf8_path_is_processed() {
    let s = Scratch::new();
    let p = s.path(OsStr::from_bytes(b"bad_\xff.rs"));

    if fs::write(&p, dirty()).is_err() {
        eprintln!("skipped: the filesystem rejects non-UTF-8 names");

        return;
    }

    let o = run(&[&p]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert_bytes_eq(&fs::read(&p).unwrap(), &clean(), "fixed file");
}

#[test]
fn parse_error_is_skipped_in_fix_mode_and_fails_check() {
    let sources: [(&str, &[u8]); 2] = [
        ("broken", b"fn f( {\n    run();\n    let a = 1;\n"),
        (
            "edition_2024_extern",
            b"unsafe extern \"C\" {\n    pub safe fn f();\n}\n",
        ),
    ];

    let s = Scratch::new();
    for (name, src) in sources {
        let p = s.file(format!("{name}.rs"), src);

        let o = run(&[&"--check", &p]);
        assert_eq!(o.code, 2, "{name}");
        assert!(o.stdout.is_empty(), "{name}");
        assert_eq!(
            o.stderr,
            format!("rust-wsl: {}: parse error, cannot check\n", p.display())
        );

        let o = run(&[&p]);
        assert_eq!(o.code, 0, "{name}");
        assert_eq!(
            o.stderr,
            format!("rust-wsl: {}: parse error, skipped\n", p.display())
        );

        assert_bytes_eq(&fs::read(&p).unwrap(), src, name);
    }
}

#[test]
fn check_goes_on_after_a_parse_error() {
    let s = Scratch::new();
    let broken = s.file("broken.rs", b"fn f( {\n");
    let p = s.file("dirty.rs", &dirty());

    let o = run(&[&"--check", &broken, &p]);
    assert_eq!(o.code, 2);
    assert_eq!(o.stdout, lines(&[line(&p, 3, Rule::Assign)]));
}

#[test]
fn missing_file_exits_2_and_the_next_file_is_still_fixed() {
    let s = Scratch::new();
    let missing = s.path("missing.rs");
    let p = s.file("dirty.rs", &dirty());

    let o = run(&[&missing, &p]);
    assert_eq!(o.code, 2);
    assert!(o.stderr.contains("missing.rs"), "{}", o.stderr);
    assert_bytes_eq(
        &fs::read(&p).unwrap(),
        &clean(),
        "valid file after the missing one",
    );
}

#[test]
fn directory_instead_of_file_exits_2() {
    let s = Scratch::new();

    let o = run(&[&s.dir()]);
    assert_eq!(o.code, 2);
    assert!(o.stderr.contains("rust-wsl:"), "{}", o.stderr);
}

#[test]
fn unknown_flag_exits_2_and_touches_nothing() {
    let s = Scratch::new();
    let p = s.file("dirty.rs", &dirty());

    let o = run(&[&"--bogus", &p]);
    assert_eq!(o.code, 2);
    assert!(o.stderr.contains("unknown flag"), "{}", o.stderr);
    assert_bytes_eq(&fs::read(&p).unwrap(), &dirty(), "file must stay untouched");
}

#[test]
fn help_exits_0_and_prints_usage() {
    for flag in ["--help", "-h"] {
        let o = run(&[&flag]);

        assert_eq!(o.code, 0, "{flag}");
        assert!(o.stdout.starts_with("usage:"), "{flag}: {}", o.stdout);
    }
}

#[test]
fn no_files_exits_0() {
    for o in [run(&[]), run(&[&"--check"])] {
        assert_eq!(o.code, 0);
        assert!(o.stdout.is_empty() && o.stderr.is_empty());
    }
}

#[test]
fn fix_mode_applies_blank_lines_writes_file_and_exits_1_on_r8() {
    let s = Scratch::new();
    let p = s.file("r8.rs", &with_r8());

    let o = run(&[&p]);
    assert_eq!(o.code, 1);
    assert!(o.stdout.is_empty());
    assert_eq!(o.stderr, format!("{}:7: {R8_HINT}\n", p.display()));
    assert_bytes_eq(&fs::read(&p).unwrap(), &with_r8_fixed(), "fixed file");
}

#[test]
fn fix_mode_reports_r8_even_when_nothing_else_changes() {
    let s = Scratch::new();
    let p = s.file("r8_only.rs", &with_r8_fixed());

    let o = run(&[&p]);
    assert_eq!(o.code, 1);
    assert_eq!(o.stderr, lines(&[line(&p, 8, Rule::UseInFn)]));
    assert_bytes_eq(
        &fs::read(&p).unwrap(),
        &with_r8_fixed(),
        "file must stay untouched",
    );
}

#[test]
fn fix_mode_reports_r8_line_of_the_original_file() {
    let s = Scratch::new();
    let p = s.file("shifted.rs", &fixture("use-in-fn/shifted_rows.input"));

    let o = run(&[&p]);
    assert_eq!(o.code, 1);
    assert_eq!(o.stderr, lines(&[line(&p, 9, Rule::UseInFn)]));
    assert_bytes_eq(
        &fs::read(&p).unwrap(),
        &fixture("use-in-fn/shifted_rows.expected"),
        "two blank lines go above the use, the report keeps the original row",
    );
}

#[test]
fn check_mode_prints_r8_to_stdout_in_line_order() {
    let s = Scratch::new();
    let p = s.file("r8_check.rs", &with_r8());

    let o = run(&[&"--check", &p]);
    assert_eq!(o.code, 1);
    assert_eq!(
        o.stdout,
        lines(&[line(&p, 3, Rule::Assign), line(&p, 7, Rule::UseInFn)])
    );
    assert_bytes_eq(&fs::read(&p).unwrap(), &with_r8(), "--check must not write");
}

#[test]
fn r8_in_one_file_does_not_stop_fixing_the_next() {
    let s = Scratch::new();
    let a = s.file("r8_a.rs", &with_r8());
    let b = s.file("dirty_b.rs", &dirty());

    let o = run(&[&a, &b]);
    assert_eq!(o.code, 1);
    assert_bytes_eq(&fs::read(&b).unwrap(), &clean(), "second file");
}

#[test]
fn exit_2_wins_over_r8() {
    let s = Scratch::new();
    let a = s.file("r8.rs", &with_r8());

    let o = run(&[&a, &s.path("missing.rs")]);
    assert_eq!(o.code, 2);
}
