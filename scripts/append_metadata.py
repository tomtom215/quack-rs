#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
# Copyright 2026 Tom F. <https://github.com/tomtom215/>
#
# Append a DuckDB extension metadata block to a compiled .so / .dylib / .dll file,
# producing a loadable .duckdb_extension file.
#
# Authoritative layout sourced from DuckDB 1.4.4 source:
#   src/main/extension/extension_load.cpp  ParseExtensionMetaData()
#   src/include/duckdb/main/extension.hpp  ParsedExtensionMetaData
#
# DuckDB reads the LAST 512 bytes of a .duckdb_extension file as the footer.
# The footer layout is:
#
#   Bytes   0 –  31  Field 0: reserved (zero-filled)
#   Bytes  32 –  63  Field 1: reserved (zero-filled)
#   Bytes  64 –  95  Field 2: reserved (zero-filled)
#   Bytes  96 – 127  Field 3: ABI type           ("C_STRUCT" | "C_STRUCT_UNSTABLE" | "CPP")
#   Bytes 128 – 159  Field 4: Extension version  (e.g. "v0.1.0")
#   Bytes 160 – 191  Field 5: DuckDB version     ("v1.2.0" C API min for C_STRUCT;
#                                                  "v1.4.0" exact release for CPP)
#   Bytes 192 – 223  Field 6: Platform           (e.g. "linux_amd64")
#   Bytes 224 – 255  Field 7: Magic bytes        (must be exactly "4")
#   Bytes 256 – 511  Signature area (RSA-2048; leave zero-filled for unsigned extensions)
#
# Each field is a null-terminated ASCII string padded to exactly 32 bytes.
# ParseExtensionMetaData reads fields 0-7 in order then reverses the array,
# so field 7 (magic) is checked first.
#
# Usage:
#   python3 scripts/append_metadata.py \
#       target/release/libhello_ext.so \
#       hello_ext.duckdb_extension \
#       --abi-type C_STRUCT \
#       --extension-version v0.1.0 \
#       --duckdb-version v1.2.0 \
#       --platform linux_amd64

import argparse
import sys
from pathlib import Path


FIELD_SIZE = 32
NUM_FIELDS = 8
METADATA_SIZE = 512
SIGNATURE_SIZE = 256


def make_field(s: str, size: int = FIELD_SIZE) -> bytes:
    """Encode a string as a null-terminated, zero-padded field of exactly `size` bytes."""
    b = s.encode("ascii")
    if len(b) >= size:
        raise ValueError(
            f"Field value {s!r} is {len(b)} bytes but max is {size - 1} "
            f"(must fit in {size} bytes including null terminator)"
        )
    return b + b"\x00" * (size - len(b))


def build_metadata(
    abi_type: str,
    extension_version: str,
    duckdb_version: str,
    platform: str,
) -> bytes:
    """Build the 512-byte metadata block.

    Field layout (verified against DuckDB 1.4.4 source):
      Field 3 (bytes  96-127): ABI type string
      Field 4 (bytes 128-159): extension version
      Field 5 (bytes 160-191): DuckDB C API version (C_STRUCT) or release version (CPP)
      Field 6 (bytes 192-223): platform
      Field 7 (bytes 224-255): magic = "4"
    """
    block = (
        make_field("") +               # Field 0: reserved
        make_field("") +               # Field 1: reserved
        make_field("") +               # Field 2: reserved
        make_field(abi_type) +         # Field 3: ABI type
        make_field(extension_version) + # Field 4: extension version
        make_field(duckdb_version) +   # Field 5: DuckDB version (CAPI min for C_STRUCT)
        make_field(platform) +         # Field 6: platform
        make_field("4") +              # Field 7: magic (must be "4")
        b"\x00" * SIGNATURE_SIZE       # Signature area (empty = unsigned)
    )
    assert len(block) == METADATA_SIZE, f"Metadata block is {len(block)} bytes, expected {METADATA_SIZE}"
    return block


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Append DuckDB extension metadata to a compiled shared library.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  # C API extension (quack-rs / Rust default):
  python3 scripts/append_metadata.py \\
      target/release/libhello_ext.so hello_ext.duckdb_extension \\
      --abi-type C_STRUCT --extension-version v0.1.0 --duckdb-version v1.2.0 \\
      --platform linux_amd64

  # C++ extension:
  python3 scripts/append_metadata.py \\
      build/libmy_ext.so my_ext.duckdb_extension \\
      --abi-type CPP --extension-version v0.1.0 --duckdb-version v1.4.0 \\
      --platform linux_amd64

Field layout (bytes 0-255 of the 512-byte footer, per DuckDB 1.4.4 source):
  Field 0-2 (bytes   0- 95): reserved, zero-filled
  Field 3   (bytes  96-127): ABI type string
  Field 4   (bytes 128-159): extension version  ← --extension-version
  Field 5   (bytes 160-191): DuckDB version     ← --duckdb-version
             (C_STRUCT: minimum C API version, e.g. v1.2.0)
             (CPP/C_STRUCT_UNSTABLE: exact DuckDB release, e.g. v1.4.0)
  Field 6   (bytes 192-223): platform           ← --platform
  Field 7   (bytes 224-255): magic = "4"
  Bytes 256-511: RSA-2048 signature (zeros = unsigned)

ABI types:
  C_STRUCT          — C Extension API (quack-rs / duckdb_rs_extension_api_init)
  C_STRUCT_UNSTABLE — C Extension API with unstable functions
  CPP               — C++ Extension API (requires C++ build infrastructure)

Platforms:
  linux_amd64, linux_arm64, osx_amd64, osx_arm64, windows_amd64
""",
    )
    parser.add_argument("input", help="Input .so / .dylib / .dll file")
    parser.add_argument("output", help="Output .duckdb_extension file")
    parser.add_argument(
        "--abi-type",
        default="C_STRUCT",
        choices=["C_STRUCT", "CPP", "C_STRUCT_UNSTABLE"],
        help="Extension ABI type (default: C_STRUCT)",
    )
    parser.add_argument(
        "--extension-version",
        default="v0.1.0",
        help="Your extension's version string (default: v0.1.0)",
    )
    parser.add_argument(
        "--duckdb-version",
        default="v1.2.0",
        help=(
            "For C_STRUCT: minimum DuckDB C API version (default: v1.2.0 = quack_rs::DUCKDB_API_VERSION). "
            "For CPP/C_STRUCT_UNSTABLE: exact DuckDB release version (e.g. v1.4.0)."
        ),
    )
    parser.add_argument(
        "--platform",
        default="linux_amd64",
        help="DuckDB build platform (default: linux_amd64)",
    )
    parser.add_argument(
        "--dump",
        action="store_true",
        help="After writing, print the metadata fields for verification",
    )

    args = parser.parse_args()

    input_path = Path(args.input)
    output_path = Path(args.output)

    if not input_path.exists():
        print(f"error: input file not found: {input_path}", file=sys.stderr)
        return 1

    so_data = input_path.read_bytes()
    metadata = build_metadata(
        abi_type=args.abi_type,
        extension_version=args.extension_version,
        duckdb_version=args.duckdb_version,
        platform=args.platform,
    )

    output_path.write_bytes(so_data + metadata)
    print(f"Written {len(so_data) + len(metadata)} bytes → {output_path}")

    if args.dump:
        print("\nMetadata fields (on-disk order):")
        field_names = [
            "reserved", "reserved", "reserved",
            "abi_type", "extension_version", "duckdb_version", "platform", "magic",
        ]
        for i in range(NUM_FIELDS):
            field = metadata[i * FIELD_SIZE : (i + 1) * FIELD_SIZE]
            null_pos = field.find(b"\x00")
            text = field[:null_pos] if null_pos >= 0 else field
            print(f"  Field {i} [{field_names[i]:20s}]: {text.decode('ascii', errors='replace')!r}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
