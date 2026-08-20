# DuckDB Rust extension Makefile.
# Delegates to cargo for building and to extension-ci-tools for metadata.

.PHONY: all configure debug release test test_debug test_release clean clean_all

PROJ_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))

EXTENSION_NAME=my_extension
EXT_CONFIG=$(PROJ_DIR)extension_config.cmake

# USE_UNSTABLE_C_API=0: the binary is stamped C_STRUCT and loads into any DuckDB
# whose C extension API version is >= TARGET_DUCKDB_VERSION. Only valid while you
# stay off quack-rs's `duckdb-1-5` / `duckdb-1-5-3` features; TARGET_DUCKDB_VERSION
# is then the *C API* version (v1.2.0), not a DuckDB release. See LESSONS.md P2.
USE_UNSTABLE_C_API=0
TARGET_DUCKDB_VERSION=v1.2.0

all: configure release

# Include extension-ci-tools build rules
include extension-ci-tools/makefiles/c_api_extensions/base.Makefile
include extension-ci-tools/makefiles/c_api_extensions/rust.Makefile

configure: venv platform extension_version

debug: build_extension_library_debug build_extension_with_metadata_debug
release: build_extension_library_release build_extension_with_metadata_release

test: test_release
test_debug: test_extension_debug
test_release: test_extension_release

clean: clean_build clean_rust
clean_all: clean_configure clean
