// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! Runs the generated `Makefile`'s own logic under `make`: the extension
//! version it writes into the metadata footer, and the check that fails
//! `make test` when the `SQLLogicTest` runner skips a file. The fragments are
//! cut out of the generated `Makefile` (extension-ci-tools is not here) and
//! run in a scratch directory, with a fake runner and real `git` repositories.
//! Unix only, and skipped where `make` or `git` is missing.

use std::path::Path;
use std::process::{Command, Output};

use super::*;

fn makefile() -> String {
    let config = ScaffoldConfig {
        name: "demo".to_string(),
        description: "Demo".to_string(),
        maintainer: "Jane Doe".to_string(),
        github_repo: "janedoe/demo".to_string(),
        ..ScaffoldConfig::default()
    };
    let files = generate_scaffold(&config).unwrap();
    files
        .into_iter()
        .find(|f| f.path == "Makefile")
        .expect("Makefile")
        .content
}

/// The lines of the generated `Makefile` from the first that starts with
/// `first` to the first after it that starts with `last`, inclusive.
fn fragment(makefile: &str, first: &str, last: &str) -> String {
    let lines: Vec<&str> = makefile.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.starts_with(first))
        .unwrap_or_else(|| panic!("no line starting with {first:?} in:\n{makefile}"));
    let end = start
        + lines[start..]
            .iter()
            .position(|l| l.starts_with(last))
            .unwrap_or_else(|| panic!("no line starting with {last:?} after {first:?}"));
    let mut text = lines[start..=end].join("\n");
    text.push('\n');
    text
}

fn have(tool: &str) -> bool {
    let found = Command::new(tool)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !found {
        eprintln!("{tool} not found; skipping");
    }
    found
}

fn run(dir: &Path, program: &str, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(dir)
        // Keep the user's git configuration (signing, hooks, templates) out.
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .env_remove("EXTENSION_VERSION")
        .env_remove("MAKEFLAGS");
    for (key, value) in env {
        command.env(key, value);
    }
    command
        .output()
        .unwrap_or_else(|e| panic!("{program}: {e}"))
}

fn git(dir: &Path, args: &[&str]) {
    let out = run(dir, "git", args, &[]);
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// What the version block sets `EXTENSION_VERSION` to in `dir`.
fn extension_version(dir: &Path, block: &str, env: &[(&str, &str)]) -> String {
    std::fs::write(
        dir.join("version.mk"),
        format!("{block}\nshow:\n\t@printf '%s\\n' '$(EXTENSION_VERSION)'\n"),
    )
    .unwrap();
    let out = run(dir, "make", &["-s", "-f", "version.mk", "show"], env);
    assert!(
        out.status.success(),
        "make: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .trim_end()
        .to_string()
}

/// extension-ci-tools writes `git tag --points-at HEAD` (every tag, one per
/// line) or git's error message into the footer's 32-byte version field, and
/// `DuckDB` 2.0 refuses a file where that overflows. The `Makefile` now
/// chooses one value that fits.
#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "spawns make and git")]
fn the_extension_version_fits_the_footer_field() {
    if !have("make") || !have("git") {
        return;
    }
    let block = fragment(
        &makefile(),
        "ifeq ($(EXTENSION_VERSION),)",
        "EXTENSION_VERSION := v$(shell",
    );
    let block = format!("{block}endif\n");
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.4.2\"\n",
    )
    .unwrap();

    // Not a repository, then a repository without a commit: the package version.
    assert_eq!(extension_version(dir, &block, &[]), "v0.4.2");
    git(dir, &["init", "-q"]);
    assert_eq!(extension_version(dir, &block, &[]), "v0.4.2");

    git(dir, &["commit", "-q", "--allow-empty", "-m", "one"]);
    let hash = String::from_utf8(run(dir, "git", &["rev-parse", "--short", "HEAD"], &[]).stdout)
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(extension_version(dir, &block, &[]), hash);

    // A tag that does not fit, or holds a character the shell would see.
    for tag in ["v1.0.0-a-very-long-pre-release-name", "it's", "a;b", "$x"] {
        git(dir, &["tag", tag]);
        assert_eq!(extension_version(dir, &block, &[]), hash, "{tag}");
        git(dir, &["tag", "-d", tag]);
    }

    // Exactly 32 bytes fits; of two tags, the first git lists.
    let tag32 = "v1.2.3-abcdefghijklmnopqrstuvwxy";
    assert_eq!(tag32.len(), 32);
    git(dir, &["tag", tag32]);
    assert_eq!(extension_version(dir, &block, &[]), tag32);
    git(dir, &["tag", "v1.2.3"]);
    assert_eq!(extension_version(dir, &block, &[]), "v1.2.3");

    // An explicit value wins.
    assert_eq!(
        extension_version(dir, &block, &[("EXTENSION_VERSION", "v9")]),
        "v9"
    );
}

/// Runs the generated test-runner wrapper with a fake runner that prints
/// `output` and exits with `status`.
fn run_tests(output: &str, status: i32) -> Output {
    let guard = fragment(&makefile(), "QUACK_RS_TEST_LOG", "TEST_RUNNER_DEBUG");
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    std::fs::write(
        dir.join("runner.sh"),
        format!(
            "printf '%s' '{}'\nexit {status}\n",
            output.replace('\'', "'\\''")
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("test.mk"),
        format!(
            "EXTENSION_NAME = demo\nTEST_RUNNER_BASE = sh runner.sh\n{guard}\n\
             test_release:\n\t@$(TEST_RUNNER_RELEASE)\n\
             test_debug:\n\t@$(TEST_RUNNER_DEBUG)\n"
        ),
    )
    .unwrap();
    let release = run(dir, "make", &["-s", "-f", "test.mk", "test_release"], &[]);
    let debug = run(dir, "make", &["-s", "-f", "test.mk", "test_debug"], &[]);
    assert_eq!(
        release.status.success(),
        debug.status.success(),
        "{output:?}"
    );
    release
}

/// The `SQLLogicTest` runner exits 0 when it skips a file whose `require`
/// is not met, so a project whose names had drifted apart passed `make test`
/// having tested nothing.
#[test]
#[cfg(unix)]
#[cfg_attr(miri, ignore = "spawns make")]
fn make_test_fails_when_a_test_file_is_skipped_or_none_runs() {
    if !have("make") {
        return;
    }
    let ok = run_tests("[1/1] test/sql/demo.test\nSUCCESS\n", 0);
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    // The runner's output is still shown.
    assert!(String::from_utf8_lossy(&ok.stdout).contains("SUCCESS"));
    // A path that happens to contain the word is not a skip.
    assert!(run_tests("[1/1] test/sql/SKIPPED.test\nSUCCESS\n", 0)
        .status
        .success());

    for (output, why) in [
        (
            "[1/1] test/sql/demo.test\nSKIPPED\nrequire demo: Missing\n",
            "skipped a test file",
        ),
        (
            "[1/2] test/sql/a.test\nSUCCESS\n[2/2] test/sql/b.test\nSKIPPED\n",
            "skipped a test file",
        ),
        (
            "[1/1] test/sql/demo.test\r\nSKIPPED\r\n",
            "skipped a test file",
        ),
        ("", "ran no test file"),
    ] {
        let out = run_tests(output, 0);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{output:?} passed");
        assert!(stderr.contains(why), "{output:?}: {stderr}");
    }

    // A failing run still fails, with the runner's output shown.
    let failed = run_tests("[1/1] test/sql/demo.test\nERROR\n", 1);
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stdout).contains("ERROR"));
}
