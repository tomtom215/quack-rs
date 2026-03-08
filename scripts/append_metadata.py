#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
# Copyright 2026 Tom F. <https://github.com/tomtom215/>
#
# Append a DuckDB extension metadata block to a compiled .so / .dylib / .dll file,
# producing a loadable .duckdb_extension file.
#
# DuckDB reads the LAST 512 bytes of a .duckdb_extension file as the metadata
# block.  The block layout is:
#
#   Bytes   0 –  31  Field 0: reserved (zero-filled)
#   Bytes  32 –  63  Field 1: reserved (zero-filled)
#   Bytes  64 –  95  Field 2: reserved (zero-filled)
#   Bytes  96 – 127  Field 3: ABI type  ("C_STRUCT" for C API, "CPP" for C++)
#   Bytes 128 – 159  Field 4: DuckDB release version   (e.g. "v1.4.0")
#   Bytes 160 – 191  Field 5: DuckDB C API version     (e.g. "v1.2.0")
#   Bytes 192 – 223  Field 6: Platform                 (e.g. "linux_amd64")
#   Bytes 224 – 255  Field 7: Magic bytes               ("4")
#   Bytes 256 – 511  Signature area (RSA-2048; leave zero-filled for unsigned extensions)
#
# Each field is a null-terminated ASCII string padded to exactly 32 bytes.
#
# Usage:
#   python3 scripts/append_metadata.py \
#       target/release/libhello_ext.so \
#       hello_ext.duckdb_extension \
#       --abi-type C_STRUCT \
#       --duckdb-version v1.4.0 \
#       --api-version v1.2.0 \
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
    duckdb_version: str,
    api_version: str,
    platform: str,
) -> bytes:
    """Build the 512-byte metadata block."""
    block = (
        make_field("") +            # Field 0: reserved
        make_field("") +            # Field 1: reserved
        make_field("") +            # Field 2: reserved
        make_field(abi_type) +      # Field 3: ABI type
        make_field(duckdb_version) + # Field 4: DuckDB release version
        make_field(api_version) +   # Field 5: DuckDB C API version
        make_field(platform) +      # Field 6: Platform
        make_field("4") +           # Field 7: Magic bytes (must be "4")
        b"\x00" * SIGNATURE_SIZE    # Signature area (empty = unsigned)
    )
    assert len(block) == METADATA_SIZE, f"Metadata block is {len(block)} bytes, expected {METADATA_SIZE}"
    return block


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Append DuckDB extension metadata to a compiled shared library.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  # C API extension (most Rust extensions use this):
  python3 scripts/append_metadata.py \\
      target/release/libhello_ext.so hello_ext.duckdb_extension \\
      --abi-type C_STRUCT --duckdb-version v1.4.0 --api-version v1.2.0 \\
      --platform linux_amd64

  # C++ extension:
  python3 scripts/append_metadata.py \\
      build/libmy_ext.so my_ext.duckdb_extension \\
      --abi-type CPP --duckdb-version v1.4.0 --api-version v1.2.0 \\
      --platform linux_amd64

ABI types:
  C_STRUCT   — C Extension API (duckdb_rs_extension_api_init / quack-rs default)
  CPP        — C++ Extension API (requires C++ build infrastructure)

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
        "--duckdb-version",
        default="v1.4.0",
        help="DuckDB release version this extension targets (default: v1.4.0)",
    )
    parser.add_argument(
        "--api-version",
        default="v1.2.0",
        help="DuckDB C API version constant (default: v1.2.0; see quack_rs::DUCKDB_API_VERSION)",
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
        duckdb_version=args.duckdb_version,
        api_version=args.api_version,
        platform=args.platform,
    )

    output_path.write_bytes(so_data + metadata)
    print(f"Written {len(so_data) + len(metadata)} bytes → {output_path}")

    if args.dump:
        print("\nMetadata fields:")
        for i in range(NUM_FIELDS):
            field = metadata[i * FIELD_SIZE : (i + 1) * FIELD_SIZE]
            null_pos = field.find(b"\x00")
            text = field[:null_pos] if null_pos >= 0 else field
            print(f"  Field {i}: {text.decode('ascii', errors='replace')!r}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
