#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Classify every slot of DuckDB's C extension API as wrapped or deliberately not.

`duckdb_ext_api_v1` is the table of function pointers a loadable extension
calls DuckDB through (546 slots at v1.5.6, 548 at v2.0.0). Nothing used to say
which of them quack-rs wraps, so the answer was a hand count that disagreed with
itself (AUDIT.md section 5.1 records 98, 99 and 105) and a slot a new DuckDB
release added went unnoticed until someone read the header.

This script makes the answer mechanical. A slot is **used** when its name
appears in the *library code* under `src/`: not in a comment, not in a string
or character literal, not in an item under `#[cfg(test)]` (or a `cfg` that
requires `test`, such as `all(test, feature = "...")`), not in a file that is
only reached through such a `mod x;`, and not under `src/bin/`. A slot only a
test, a string or a comment names is not wrapped: counting those as wrapped
put `duckdb_open` and `duckdb_close` (used only by tests) in the total, and a
name kept in an error message satisfied the check after its wrapper was
deleted. A used slot is **wrapped** unless the allowlist classes it
`internal`. Every other slot must be listed in `scripts/api-coverage-allowlist.txt`
with a class and a reason:

    deprecated   DuckDB marks it deprecated in `duckdb.h`
    superseded   a newer call does the same job, or it is an internal helper
    host-side    only a host application (not an extension) has a use for it
    candidate    worth wrapping; not done yet
    internal     library code calls it, but no public API gives an extension
                 author what it does (it is used, so it must stay used)

It fails when

  * a slot is neither used nor listed (a new DuckDB release added it, or a
    wrapper was deleted, or only tests use it): wrap it, or list it;
  * a slot listed in any class but `internal` is used after all: delete its
    line (or class it `internal` if no public API exposes it);
  * a slot listed `internal` is not used: it is unused, so give it another
    class.

A listed name the header does not declare is ignored (the same list serves the
546-slot and the 548-slot header).

The header is fetched for the DuckDB release that `Cargo.lock`'s `libduckdb-sys`
corresponds to (`scripts/duckdb-version-from-lock.sh`), the same way
`check-abi-table.py` fetches its headers, or read from `--header`.

Exit codes:
    0  every slot is wrapped or listed, and no listed slot is wrapped
    1  the classification is wrong (see the output)
    2  the header could not be fetched -- "could not check", not a failure

Usage:
    python3 scripts/check-api-coverage.py
    python3 scripts/check-api-coverage.py --header path/to/duckdb_extension.h
    python3 scripts/check-api-coverage.py --list       # print the slots library code does not use
    python3 scripts/check-api-coverage.py --self-test  # the source scanner, on fixed snippets
"""

from __future__ import annotations

import argparse
import importlib.util
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ALLOWLIST = ROOT / "scripts" / "api-coverage-allowlist.txt"
CLASSES = {"deprecated", "superseded", "host-side", "candidate", "internal"}
NAME = re.compile(r"\bduckdb_\w+\b")


def load_abi_table_module():
    """`check-abi-table.py` (its name is not importable) for its header parser."""
    path = ROOT / "scripts" / "check-abi-table.py"
    spec = importlib.util.spec_from_file_location("check_abi_table", path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


# --------------------------------------------------------------------- scanner

CHAR_LITERAL = re.compile(r"'(?:[^'\\\n]|\\(?:[nrt\\0'\"]|x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]{1,6}\}))'")
RAW_STRING = re.compile(r'(?:b|c)?r(#*)"')


def strip(text: str) -> tuple[str, str]:
    """`text` without comments, and without comments or literals.

    Returns (code with string literals kept, code with every string, raw
    string, byte string and character literal replaced by `""`). Comments
    (`//`, `///`, `//!`, nested `/* */`) become a space; newlines inside them
    are kept so positions stay roughly comparable. Lifetimes (`'a`) are code.
    """
    keep: list[str] = []
    bare: list[str] = []
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            keep.append(" ")
            bare.append(" ")
            i = j
            continue
        if text.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif text.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            blank = " " + "\n" * text.count("\n", i, j)
            keep.append(blank)
            bare.append(blank)
            i = j
            continue
        prev_ident = i > 0 and (text[i - 1].isalnum() or text[i - 1] == "_")
        raw = None if prev_ident else RAW_STRING.match(text, i)
        if raw:
            close = '"' + raw.group(1)
            j = text.find(close, raw.end())
            j = n if j < 0 else j + len(close)
            keep.append(text[i:j])
            bare.append('""')
            i = j
            continue
        if c == '"' or (c in "bc" and not prev_ident and text.startswith('"', i + 1)):
            j = i + (2 if c != '"' else 1)
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            j = min(j + 1, n)
            keep.append(text[i:j])
            bare.append('""')
            i = j
            continue
        if c == "'":
            m = CHAR_LITERAL.match(text, i)
            if m:
                keep.append(m.group(0))
                bare.append("''")
                i = m.end()
                continue
        keep.append(c)
        bare.append(c)
        i += 1
    return "".join(keep), "".join(bare)


def split_args(body: str) -> list[str]:
    """The comma-separated arguments of a cfg predicate list, at depth 0."""
    args, depth, start = [], 0, 0
    for k, ch in enumerate(body):
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
        elif ch == "," and depth == 0:
            args.append(body[start:k])
            start = k + 1
    args.append(body[start:])
    return [a.strip() for a in args if a.strip()]


def requires_test(predicate: str) -> bool:
    """Whether a `cfg(...)` predicate can only hold when compiling tests."""
    p = predicate.strip()
    if p == "test":
        return True
    m = re.fullmatch(r"(all|any|not)\s*\((.*)\)", p, re.DOTALL)
    if not m:
        return False
    args = split_args(m.group(2))
    if m.group(1) == "all":
        return any(requires_test(a) for a in args)
    if m.group(1) == "any":
        return bool(args) and all(requires_test(a) for a in args)
    return False  # not(...): never test-only


CFG_ATTR = re.compile(r"#\[\s*cfg\s*\(")


def cfg_attrs(code: str):
    """(start, end, predicate) of each outer `#[cfg(...)]` attribute in `code`."""
    for m in CFG_ATTR.finditer(code):
        depth, k = 1, m.end()
        while k < len(code) and depth:
            depth += {"(": 1, ")": -1}.get(code[k], 0)
            k += 1
        close = code.find("]", k)
        if close < 0:
            continue
        yield m.start(), close + 1, code[m.end() : k - 1]


def item_end(code: str, start: int) -> int:
    """End of the item that begins at `start` (after its attributes): the
    matching `}` of its first top-level `{`, or its first top-level `;`."""
    k, n = start, len(code)
    # Further attributes on the same item.
    while True:
        while k < n and code[k].isspace():
            k += 1
        if code.startswith("#[", k):
            depth = 0
            while k < n:
                depth += {"[": 1, "]": -1}.get(code[k], 0)
                k += 1
                if depth == 0:
                    break
            continue
        break
    parens = 0
    while k < n:
        ch = code[k]
        if ch in "([":
            parens += 1
        elif ch in ")]":
            parens -= 1
        elif ch == ";" and parens == 0:
            return k + 1
        elif ch == "{" and parens == 0:
            depth = 0
            while k < n:
                depth += {"{": 1, "}": -1}.get(code[k], 0)
                k += 1
                if depth == 0:
                    return k
            return n
        k += 1
    return n


# What may follow a test-only cfg attribute for it to be a `mod x;` declaration:
# other attributes, at most one of them `#[path = "..."]`, then `mod name;`.
MOD_DECL = re.compile(
    r"(?:\s*#\[(?!\s*path\b)[^\]]*\])*"
    r"(?:\s*#\[\s*path\s*=\s*\"(?P<path>[^\"]+)\"\s*\])?"
    r"(?:\s*#\[[^\]]*\])*"
    r"\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(?P<name>\w+)\s*;"
)


def library_code(path: Path, text: str, test_files: set[Path]) -> str:
    """The library code of one file: comments, literals and test-only items removed.
    Adds to `test_files` the files its test-only `mod x;` declarations load."""
    keep, bare = strip(text)
    if re.search(r"#!\[\s*cfg\s*\(\s*test\s*\)\s*\]", bare):
        return ""
    # Test-only `mod x;` declarations, read where string literals are intact
    # (a `#[path = "..."]` needs its string). A declaration inside a string (the
    # scaffold's templates hold some) resolves to no file, so it changes nothing.
    for _, end, predicate in cfg_attrs(keep):
        decl = MOD_DECL.match(keep, end) if requires_test(predicate) else None
        if decl is None:
            continue
        name, explicit = decl.group("name"), decl.group("path")
        if explicit:
            candidates = [path.parent / explicit]
        elif path.name in ("mod.rs", "lib.rs", "main.rs"):
            candidates = [path.parent / f"{name}.rs", path.parent / name / "mod.rs"]
        else:
            candidates = [path.parent / path.stem / f"{name}.rs", path.parent / path.stem / name / "mod.rs"]
        test_files.update(c.resolve() for c in candidates if c.exists())
    # Test-only items, cut from the literal-free code.
    cut = [(start, item_end(bare, end)) for start, end, predicate in cfg_attrs(bare) if requires_test(predicate)]
    out, last = [], 0
    for start, stop in sorted(cut):
        if start >= last:
            out.append(bare[last:start])
            last = stop
    out.append(bare[last:])
    return "".join(out)


def scan(src: Path) -> tuple[dict[str, set[str]], set[str]]:
    """(name -> library files using it, names used only outside library code)."""
    files = sorted(p for p in src.rglob("*.rs") if "bin" not in p.relative_to(src).parts[:1])
    test_files: set[Path] = set()
    code = {p.resolve(): library_code(p, p.read_text(encoding="utf-8"), test_files) for p in files}
    # A test module's own children are test code too.
    changed = True
    while changed:
        changed = False
        for p in code:
            if p in test_files:
                continue
            for t in list(test_files):
                tdir = t.parent if t.name == "mod.rs" else t.parent / t.stem
                if tdir in p.parents:
                    test_files.add(p)
                    changed = True
                    break
    used: dict[str, set[str]] = {}
    for p, text in code.items():
        if p in test_files:
            continue
        for name in set(NAME.findall(text)):
            used.setdefault(name, set()).add(p.relative_to(src.resolve()).as_posix())
    elsewhere = set()
    for p in src.rglob("*.rs"):
        elsewhere |= set(NAME.findall(p.read_text(encoding="utf-8")))
    return used, elsewhere - set(used)


SELF_TEST = [
    # (file name, source, names that must count as used, names that must not)
    ("a.rs", 'fn f() { unsafe { duckdb_a() } }', {"duckdb_a"}, set()),
    ("b.rs", "// duckdb_b()\n/* duckdb_c /* nested */ duckdb_d */\n/// duckdb_e\nfn g() {}",
     set(), {"duckdb_b", "duckdb_c", "duckdb_d", "duckdb_e"}),
    ("c.rs", ("fn h() -> &'static str { \"duckdb_f\" }\n"
              'fn i() { let _ = r#"duckdb_g"#; let _ = b"duckdb_h"; }'),
     set(), {"duckdb_f", "duckdb_g", "duckdb_h"}),
    ("d.rs", "fn j() { let q = '\"'; duckdb_i(); let r = '\\''; }", {"duckdb_i"}, set()),
    ("e.rs", "fn k<'a>(x: &'a u8) { duckdb_j() }\n#[cfg(test)]\nmod tests { fn t() { duckdb_k() } }",
     {"duckdb_j"}, {"duckdb_k"}),
    ("f.rs", ('#[cfg(all(feature = "x", test))]\nfn t() { duckdb_l() }\n'
              '#[cfg(any(test, feature = "y"))]\nfn u() { duckdb_m() }\n#[cfg(not(test))]\nfn v() { duckdb_n() }'),
     {"duckdb_m", "duckdb_n"}, {"duckdb_l"}),
    ("g.rs", ("#[cfg(test)]\n#[allow(dead_code)]\nconst T: [u8; 2] = [1, 2];\nfn w() { duckdb_o() }\n"
              "#[cfg(test)]\nuse x::duckdb_p;"),
     {"duckdb_o"}, {"duckdb_p"}),
    ("h.rs", 'fn s() { let _ = "a \\" duckdb_q"; duckdb_r() }', {"duckdb_r"}, {"duckdb_q"}),
]


def self_test() -> int:
    import tempfile

    failures = total = 0
    with tempfile.TemporaryDirectory() as tmp:
        src = Path(tmp) / "src"
        (src / "m").mkdir(parents=True)
        for name, source, _, _ in SELF_TEST:
            (src / name).write_text(source, encoding="utf-8")
        # A test-only module file and its child, reached through `mod`.
        (src / "m.rs").write_text("#[cfg(test)]\nmod live;\nfn x() { duckdb_s() }", encoding="utf-8")
        (src / "m" / "live.rs").write_text("fn y() { duckdb_t() }", encoding="utf-8")
        (src / "m" / "live").mkdir()
        (src / "m" / "live" / "deep.rs").write_text("fn z() { duckdb_u() }", encoding="utf-8")
        (src / "bin").mkdir()
        (src / "bin" / "tool.rs").write_text("fn main() { duckdb_v() }", encoding="utf-8")
        used, elsewhere = scan(src)
        cases = [(n, s, u, nu) for n, s, u, nu in SELF_TEST] + [
            ("m.rs", "", {"duckdb_s"}, {"duckdb_t", "duckdb_u"}),
            ("bin/tool.rs", "", set(), {"duckdb_v"}),
        ]
        for name, _, want, not_want in cases:
            for w in want:
                total += 1
                if w not in used:
                    print(f"self-test: {name}: {w} should count as used")
                    failures += 1
            for w in not_want:
                total += 1
                if w in used:
                    print(f"self-test: {name}: {w} should not count as used (found in {sorted(used[w])})")
                    failures += 1
        total += 1
        if "duckdb_k" not in elsewhere:
            print("self-test: a name used only by a test should be reported as such")
            failures += 1
    print(f"self-test: {total - failures}/{total} expectations hold")
    return 1 if failures else 0


# ------------------------------------------------------------------ allowlist


def read_allowlist() -> dict[str, tuple[str, str]]:
    entries: dict[str, tuple[str, str]] = {}
    for number, line in enumerate(ALLOWLIST.read_text(encoding="utf-8").splitlines(), 1):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split(None, 2)
        if len(parts) != 3 or parts[1] not in CLASSES:
            sys.exit(f"{ALLOWLIST.name}:{number}: expected `name class reason`, class one of {sorted(CLASSES)}")
        name, cls, reason = parts
        if name in entries:
            sys.exit(f"{ALLOWLIST.name}:{number}: {name} is listed twice")
        entries[name] = (cls, reason)
    return entries


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--header", type=Path, help="duckdb_extension.h to read instead of fetching one")
    parser.add_argument("--list", action="store_true", help="print the slots library code does not use, and exit")
    parser.add_argument("--self-test", action="store_true", help="check the source scanner on fixed snippets, and exit")
    args = parser.parse_args()
    if args.self_test:
        return self_test()

    abi = load_abi_table_module()
    if args.header:
        header = args.header.read_text(encoding="utf-8")
        label = str(args.header)
    else:
        tag = subprocess.run(
            ["bash", str(ROOT / "scripts" / "duckdb-version-from-lock.sh")],
            check=True, capture_output=True, text=True, cwd=ROOT,
        ).stdout.strip()
        status, header = abi.fetch(tag)
        if status != "ok":
            print(f"could not fetch duckdb_extension.h for {tag}: {status} {header or ''}; check skipped")
            return 2
        label = f"DuckDB {tag}"

    slots = abi.struct_fields(header, unstable=True, api=None)
    used_in, not_library = scan(ROOT / "src")
    used = [name for name in slots if name in used_in]
    unused = [name for name in slots if name not in used_in]
    if args.list:
        print("\n".join(unused))
        return 0

    allowed = read_allowlist()
    problems: list[str] = []
    for name in unused:
        if name not in allowed:
            hint = " (only tests, strings or comments under src/ name it)" if name in not_library else ""
            problems.append(f"unclassified: {name}{hint} (wrap it, or list it in {ALLOWLIST.name} with a reason)")
        elif allowed[name][0] == "internal":
            problems.append(f"stale: {name} is listed as internal but library code does not use it; give it another class")
    for name in used:
        cls = allowed.get(name, ("",))[0]
        if cls and cls != "internal":
            problems.append(
                f"stale: {name} is listed as {cls} but library code uses it ({', '.join(sorted(used_in[name]))}); "
                "delete its line, or class it `internal` if no public API exposes it"
            )

    internal = [n for n in used if allowed.get(n, ("",))[0] == "internal"]
    wrapped = len(used) - len(internal)
    counts = {cls: sum(1 for n in unused if allowed.get(n, ("",))[0] == cls) for cls in sorted(CLASSES - {"internal"})}
    print(
        f"{label}: {len(slots)} slots, {wrapped} wrapped, {len(internal)} used internally only, "
        f"{len(unused)} not used: " + ", ".join(f"{n} {cls}" for cls, n in counts.items())
    )
    test_only = sorted(n for n in unused if n in not_library)
    if test_only:
        print(f"  named under src/ only by tests, strings or comments: {' '.join(test_only)}")
    for problem in problems:
        print(f"  {problem}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
