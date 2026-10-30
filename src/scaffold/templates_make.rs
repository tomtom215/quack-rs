// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F. <https://github.com/tomtom215/>
// My way of giving something small back to the open source community
// and encouraging more Rust development!

//! The generated `Makefile`: extension-ci-tools' variables and targets, the
//! extension-version choice for the metadata footer, the check that fails
//! `make test` on a skipped test file, and (for pinned bindings) the
//! `check_duckdb_pin` rule.

use super::ScaffoldConfig;

pub(super) fn generate_makefile(config: &ScaffoldConfig) -> String {
    // Mirrors duckdb/extension-template-rs: `EXTENSION_NAME`,
    // `TARGET_DUCKDB_VERSION` and `USE_UNSTABLE_C_API` are the variables
    // `extension-ci-tools/makefiles/c_api_extensions/base.Makefile` actually
    // reads, and the aggregate targets below are what `make` users invoke.
    let unstable = u8::from(config.use_unstable_c_api);
    let abi_note = if config.use_unstable_c_api {
        "# USE_UNSTABLE_C_API=1: the binary is stamped C_STRUCT_UNSTABLE and DuckDB will\n\
         # only load it into exactly TARGET_DUCKDB_VERSION, a DuckDB 1.x release. (DuckDB\n\
         # 2.0 loads C_STRUCT_UNSTABLE only through the C API v2 entry point, which\n\
         # quack-rs does not generate: for 2.0 use USE_UNSTABLE_C_API=0 with\n\
         # TARGET_DUCKDB_VERSION=v1.5.6.)"
    } else if super::pins_bindings(config) {
        "# USE_UNSTABLE_C_API=0 with TARGET_DUCKDB_VERSION=v1.5.6: the binary is stamped\n\
         # C_STRUCT for C API v1.5.6, which DuckDB v1.5.6 and v2.0.0 load and older\n\
         # releases refuse. This is the stamping for quack-rs's `duckdb-1-5` / `-3` / `-4`\n\
         # features: DuckDB declared that whole layout stable from v1.5.6. TARGET_DUCKDB_VERSION\n\
         # is the *C API* version here, not a DuckDB release. See\n\
         # https://quack-rs.com/reference/pitfalls.html#p2-metadata-version-is-c-api-version-not-duckdb-version"
    } else {
        "# USE_UNSTABLE_C_API=0: the binary is stamped C_STRUCT and loads into any DuckDB\n\
         # whose C extension API version is >= TARGET_DUCKDB_VERSION (every release from\n\
         # v1.2.0, 2.0 included). Only valid while you stay off quack-rs's `duckdb-1-5` /\n\
         # `-3` / `-4` features (for those, TARGET_DUCKDB_VERSION=v1.5.6);\n\
         # TARGET_DUCKDB_VERSION is the *C API* version (v1.2.0), not a DuckDB release. See\n\
         # https://quack-rs.com/reference/pitfalls.html#p2-metadata-version-is-c-api-version-not-duckdb-version"
    };
    // Unstable builds only: tell quack-rs's ABI check which release the bindings
    // were built against (so it accepts a release its layout table predates,
    // which is what happens every time DuckDB ships and the community
    // repository rebuilds this extension from unchanged source), test against
    // that same release, and refuse to build when the resolved libduckdb-sys
    // is for a different one. Without that last guard, bumping
    // TARGET_DUCKDB_VERSION alone makes the extension *declare* a release its
    // bindings do not match — and for a release quack-rs has no layout entry
    // for, the declaration is trusted.
    let (declare_block, check_prereq, check_target) = if config.use_unstable_c_api {
        (
            "\n# pip version of DuckDB that `make test` runs against: the pinned release.\n\
             DUCKDB_TEST_VERSION=$(patsubst v%,%,$(TARGET_DUCKDB_VERSION))\n\
             \n\
             # Lets quack-rs's ABI check accept a DuckDB release newer than its layout table.\n\
             export QUACK_RS_TARGET_DUCKDB_VERSION = $(TARGET_DUCKDB_VERSION)\n",
            "check_duckdb_pin ",
            CHECK_DUCKDB_PIN_TARGET,
        )
    } else if super::pins_bindings(config) {
        // Tested against whatever DuckDB extension-ci-tools installs (the
        // newest release), which is the point: the binary is meant for v2.0.
        (
            "\n# The bindings are DuckDB v1.5.6's (see Cargo.toml): tells quack-rs's ABI check so.\n\
             export QUACK_RS_TARGET_DUCKDB_VERSION = $(TARGET_DUCKDB_VERSION)\n",
            "check_duckdb_pin ",
            CHECK_DUCKDB_PIN_TARGET,
        )
    } else {
        ("", "", "")
    };
    let ci_tools_ref = super::extension_ci_tools_ref(config);
    format!(
        r"# DuckDB Rust extension Makefile.
# Delegates to cargo for building and to extension-ci-tools for metadata.

.PHONY: all configure debug release test test_debug test_release clean clean_all{phony_check}

PROJ_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))

EXTENSION_NAME={name}

{abi_note}
USE_UNSTABLE_C_API={unstable}
TARGET_DUCKDB_VERSION={target_version}
{declare_block}{version_block}
all: configure release

# Include extension-ci-tools build rules. A freshly generated project has a
# .gitmodules but no submodule yet, and `git submodule update --init` does
# nothing until the submodule has been added once
# (https://quack-rs.com/reference/pitfalls.html#p4-extension-ci-tools-submodule-must-be-initialized).
# Use the branch the distribution workflow names as `ci_tools_version`.
ifeq ($(wildcard extension-ci-tools/makefiles/c_api_extensions/base.Makefile),)
$(error extension-ci-tools is missing. In a new repository run: git submodule add -b {ci_tools_ref} https://github.com/duckdb/extension-ci-tools.git extension-ci-tools -- in a clone of an existing one: git submodule update --init --recursive)
endif
include extension-ci-tools/makefiles/c_api_extensions/base.Makefile
include extension-ci-tools/makefiles/c_api_extensions/rust.Makefile
{skip_guard}
configure: venv platform extension_version

debug: {check_prereq}build_extension_library_debug build_extension_with_metadata_debug
release: {check_prereq}build_extension_library_release build_extension_with_metadata_release
{check_target}
test: test_release
test_debug: test_extension_debug
test_release: test_extension_release

clean: clean_build clean_rust
clean_all: clean_configure clean
",
        name = config.name,
        ci_tools_ref = ci_tools_ref,
        abi_note = abi_note,
        unstable = unstable,
        target_version = config.target_duckdb_version,
        declare_block = declare_block,
        version_block = VERSION_BLOCK,
        skip_guard = SKIP_GUARD,
        check_prereq = check_prereq,
        check_target = check_target,
        phony_check = if super::pins_bindings(config) {
            " check_duckdb_pin"
        } else {
            ""
        },
    )
}

/// The `Makefile`'s choice of the extension version for the metadata footer.
///
/// extension-ci-tools' `configure_helper.py` writes `git tag --points-at
/// HEAD` into the footer's 32-byte version field — every tag on HEAD, one per
/// line — or else `git log -1 --format=%h`, and in a repository with no commit
/// yet git's error message. Several tags, a tag over 32 bytes or that message
/// overflow the field, and `DuckDB` 2.0 refuses the file (`INTERNAL Error:
/// Unknown ABI type for extension`). So this sets `EXTENSION_VERSION`, which
/// `base.Makefile` then writes instead: the first tag on HEAD if it fits and
/// holds only `A-Za-z0-9._+-` (it reaches a shell `echo` in `base.Makefile`),
/// else the short hash, else the package version. No `#` anywhere: make before
/// 4.3 (macOS ships 3.81) reads one inside `$(shell …)` as a comment. And no
/// unbalanced parenthesis: make ends `$(shell …)` at the first `)` that closes
/// it, which is why the `case` pattern is written `(pattern)`.
const VERSION_BLOCK: &str = r#"
# Extension version written into the metadata footer's 32-byte field: the first
# tag on HEAD if it fits (letters, digits and . _ + - only), else the short
# commit hash, else -- before the first commit -- the package version in
# Cargo.toml. extension-ci-tools would write every tag on HEAD, one per line,
# or git's error message in a repository without a commit; both can overflow
# the field, and DuckDB then refuses the file ('Unknown ABI type for
# extension'). Set EXTENSION_VERSION to override. The value is cached in
# configure/extension_version.txt: run `make clean_all configure` after
# committing or tagging.
ifeq ($(EXTENSION_VERSION),)
EXTENSION_VERSION := $(shell t=$$(git tag --points-at HEAD 2>/dev/null | head -n 1); case "$$t" in (*[!A-Za-z0-9._+-]*) t= ;; esac; if [ -n "$$t" ] && [ "$$(printf '%s' "$$t" | wc -c | tr -d ' ')" -le 32 ]; then printf '%s' "$$t"; else git rev-parse --short HEAD 2>/dev/null; fi)
endif
ifeq ($(EXTENSION_VERSION),)
EXTENSION_VERSION := v$(shell sed -n 's/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml | head -n 1)
endif
"#;

/// Wraps extension-ci-tools' `SQLLogicTest` runner so that a skipped test file
/// fails `make test`.
///
/// The runner (`duckdb_sqllogictest`, also what the community build runs)
/// reports a file whose `require <name>` is not met as `SKIPPED` and exits 0:
/// when the extension name, the `[lib]` name and the `.test` file drift apart,
/// or the extension file is missing, `make test` passed having tested nothing
/// (checked against `v2.0-cyanoptera`). `base.Makefile` runs the
/// commands below as `@$(TEST_RUNNER_RELEASE)` / `@$(TEST_RUNNER_DEBUG)`, and
/// a recipe expands its variables when it runs, so redefining them after the
/// `include` takes effect; its own `tests_skipped` (platforms it cannot test
/// on) is untouched. The runner prints `SKIPPED` on a line of its own (a
/// `\r` may follow on Windows), so the check matches it at a line start, not
/// inside a test file's path.
const SKIP_GUARD: &str = r#"
# The SQLLogicTest runner reports a test file whose `require <name>` is not met
# as SKIPPED and still exits 0 -- which is what a rename that leaves the
# extension name, the [lib] name and test/sql out of step, or a missing
# extension file, looks like -- so `make test` would pass having tested
# nothing. Run it through a check that fails on SKIPPED, and when no test file
# ran at all.
QUACK_RS_TEST_LOG = build/sqllogictest.log
QUACK_RS_CHECK_TEST_LOG = status=$$?; cat $(QUACK_RS_TEST_LOG); if [ $$status -ne 0 ]; then exit $$status; fi; if grep -q '^SKIPPED' $(QUACK_RS_TEST_LOG); then echo "error: the SQLLogicTest runner skipped a test file (SKIPPED above): its 'require' must name EXTENSION_NAME=$(EXTENSION_NAME), the [lib] name in Cargo.toml, and the built extension must exist" >&2; exit 1; fi; if ! grep -q '^SUCCESS' $(QUACK_RS_TEST_LOG); then echo "error: the SQLLogicTest runner ran no test file: test/sql holds no .test file" >&2; exit 1; fi
TEST_RUNNER_RELEASE = mkdir -p build && $(TEST_RUNNER_BASE) --external-extension build/release/$(EXTENSION_NAME).duckdb_extension > $(QUACK_RS_TEST_LOG) 2>&1; $(QUACK_RS_CHECK_TEST_LOG)
TEST_RUNNER_DEBUG = mkdir -p build && $(TEST_RUNNER_BASE) --external-extension build/debug/$(EXTENSION_NAME).duckdb_extension > $(QUACK_RS_TEST_LOG) 2>&1; $(QUACK_RS_CHECK_TEST_LOG)
"#;

/// The `check_duckdb_pin` rule of an unstable-ABI `Makefile`.
///
/// Asks cargo which `libduckdb-sys` the build resolves to, decodes the
/// `DuckDB` release it ships (the same mapping as quack-rs's
/// `scripts/duckdb-version-from-lock.sh`) and fails unless it is
/// `TARGET_DUCKDB_VERSION`. A plain string so the recipe's tab and `$$` reach
/// the file exactly.
const CHECK_DUCKDB_PIN_TARGET: &str = "
# The bindings compiled in come from the libduckdb-sys pin in Cargo.toml, not
# from TARGET_DUCKDB_VERSION. Refuse to build when they name different releases.
check_duckdb_pin:
\t@v=$$(cargo tree -e normal -i libduckdb-sys --depth 0 --prefix none | sed -n 's/^libduckdb-sys v//p' | head -n 1); \\
\tm=$$(echo \"$$v\" | cut -d. -f2); \\
\tif [ \"$${m:-0}\" -ge 10000 ] 2>/dev/null; then got=\"v$$((m / 10000)).$$((m / 100 % 100)).$$((m % 100))\"; else got=\"v$$v\"; fi; \\
\tif [ \"$$got\" != \"$(TARGET_DUCKDB_VERSION)\" ]; then \\
\t\techo \"error: libduckdb-sys $$v is DuckDB $$got, but TARGET_DUCKDB_VERSION=$(TARGET_DUCKDB_VERSION); update the libduckdb-sys pin in Cargo.toml\" >&2; \\
\t\texit 1; \\
\tfi
";
