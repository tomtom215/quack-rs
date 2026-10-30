#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Prepare a COPY of the tree so cargo-mutants can see code it skips by design.

cargo-mutants 27.1.0 generates no mutants inside `unsafe fn` bodies (free,
associated, `const`, trait-default and `unsafe extern "C"` functions alike) and
never mutates integer literals. In this crate that hides the pointer arithmetic:
the strides in `VectorReader` / `VectorWriter`, the bit tests of the validity
mask, `ListBuilder`'s growth arithmetic, the Arrow layout walker, `FfiState`'s
callbacks. `cargo mutants --list` on `src/vector/reader.rs` and `writer.rs` lists
6 mutants; with the `unsafe` keyword removed from the signatures it lists 192,
and with the literals wrapped as well 344 (measured on 0.20.0 development).

The crate denies `unsafe_op_in_unsafe_fn`, so every unsafe operation inside an
`unsafe fn` is already in its own `unsafe { }` block: dropping the keyword from
the signature still compiles (callers get `unused_unsafe` warnings). With
`--literals`, an integer literal that is an operand of `+ - * / % << >> &`, or
the argument of `.add()` / `.sub()` / `.offset()`, is rewritten as `(L - 1 + 1)`
(`(0 + 1 - 1)` for 0), which gives cargo-mutants operators to flip.

This is a heuristic over text (comments and strings are blanked, braces matched,
`#[cfg(test)]` modules skipped), for an audit, not for CI: run it on a copy,
check that the copy still builds, then run the live suite against the mutants:

    cp -r . /tmp/mutants-copy && cd /tmp/mutants-copy
    python3 scripts/mutants-expose-unsafe.py . --literals src/vector/reader.rs src/vector/writer.rs
    cargo check --lib --features duckdb-1-5-4
    cargo mutants --in-place --no-config --features duckdb-1-5-4 \\
        --file src/vector/reader.rs --file src/vector/writer.rs \\
        --cargo-arg=--test --cargo-arg=ffi_roundtrip   # live: DUCKDB_LIB_DIR, bundled-test-prebuilt

Usage: mutants-expose-unsafe.py TREE [--literals] [--sites FILE] FILE...
FILE is relative to TREE and edited in place; --sites writes a JSON list of the
rewritten literal sites (file, line).
"""
import json
import os
import re
import sys


def strip_comments_strings(src):
    """Replace comments and string/char literal contents with spaces (keeps offsets)."""
    out = list(src)
    i, n = 0, len(src)
    while i < n:
        c = src[i]
        if src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j < 0 else j
            for k in range(i, j): out[k] = " "
            i = j
        elif src.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if src.startswith("/*", j): depth += 1; j += 2
                elif src.startswith("*/", j): depth -= 1; j += 2
                else: j += 1
            for k in range(i, j):
                if out[k] != "\n": out[k] = " "
            i = j
        elif c == 'r' and re.match(r'r#*"', src[i:i+10]):
            m = re.match(r'r(#*)"', src[i:])
            hashes = m.group(1)
            end = '"' + hashes
            j = src.find(end, i + len(m.group(0)))
            j = n if j < 0 else j + len(end)
            for k in range(i + len(m.group(0)), j - len(end)):
                if out[k] != "\n": out[k] = " "
            i = j
        elif c == '"':
            j = i + 1
            while j < n and src[j] != '"':
                j += 2 if src[j] == '\\' else 1
            for k in range(i + 1, min(j, n)):
                if out[k] != "\n": out[k] = " "
            i = j + 1
        elif c == "'":
            m = re.match(r"'(\\.[^']*|[^'\\])'", src[i:i+12])
            if m:
                for k in range(i + 1, i + len(m.group(0)) - 1): out[k] = " "
                i += len(m.group(0))
            else:
                i += 1
        else:
            i += 1
    return "".join(out)


tree = sys.argv[1]
args = sys.argv[2:]
literals = "--literals" in args
sites_file = None
if "--sites" in args:
    sites_file = args[args.index("--sites") + 1]
    del args[args.index("--sites"):args.index("--sites") + 2]
files = [a for a in args if not a.startswith("--")]
sites = []
stats = {}

def test_regions(clean):
    regs = []
    for m in re.finditer(r'#\[cfg\((?:all\()?test[^\]]*\]\s*(?:#\[[^\]]*\]\s*)*(?:pub\s+)?mod\s+\w+\s*\{', clean):
        i = clean.index("{", m.start()); depth = 0
        for j in range(i, len(clean)):
            if clean[j] == "{": depth += 1
            elif clean[j] == "}":
                depth -= 1
                if depth == 0:
                    regs.append((m.start(), j)); break
    return regs

LIT = r'(?:0[xX][0-9a-fA-F_]+|0[bB][01_]+|[0-9][0-9_]*)(?:_?(?:u8|u16|u32|u64|u128|usize|i8|i16|i32|i64|i128|isize))?'
for rel in files:
    path = os.path.join(tree, rel)
    src = open(path, encoding="utf-8").read()
    clean = strip_comments_strings(src)
    regs = test_regions(clean)
    in_test = lambda pos: any(a <= pos <= b for a, b in regs)
    edits = []  # (start, end, replacement)
    n_unsafe = 0
    for m in re.finditer(r'\bunsafe\s+(?=(?:extern\s+"[^"]*"\s+)?fn\s+\w)', clean):
        if in_test(m.start()): continue
        # skip trait method declarations/impls inside `unsafe trait`/`unsafe impl` blocks: detect `unsafe impl`/`unsafe trait` ancestors crudely
        before = clean[:m.start()]
        last_impl = max(before.rfind("unsafe impl"), before.rfind("unsafe trait"))
        if last_impl >= 0:
            # still inside that block if braces are unbalanced since
            seg = clean[last_impl:m.start()]
            if seg.count("{") > seg.count("}"): continue
        edits.append((m.start(), m.end(), "")); n_unsafe += 1
    n_lit = 0
    if literals:
        # candidates: literal preceded by `.add(`/.sub(/.offset( or by a binary operator after an operand
        for m in re.finditer(LIT, clean):
            s, e = m.span()
            if in_test(s): continue
            if s > 0 and (clean[s-1].isalnum() or clean[s-1] in "_."): continue
            if e < len(clean) and (clean[e].isalnum() or clean[e] in "_."): continue
            # reject floats like 1.5 / ranges
            if clean[e:e+1] == "." and clean[e+1:e+2].isdigit(): continue
            # find previous non-space char(s)
            k = s - 1
            while k >= 0 and clean[k] in " \t\n": k -= 1
            prev = clean[k]
            ok = False
            if prev == "(":
                head = clean[max(0, k-14):k+1]
                if re.search(r'\.(add|sub|offset|wrapping_add|wrapping_sub|byte_add|byte_sub)\($', head): ok = True
            elif prev in "*/%&+-<>" :
                # binary operator? look at what precedes the operator
                op_end = k
                op_start = k
                while op_start > 0 and clean[op_start-1] in "*/%&+-<>=": op_start -= 1
                op = clean[op_start:op_end+1]
                if op in ("*", "/", "%", "&", "+", "-", "<<", ">>"):
                    j = op_start - 1
                    while j >= 0 and clean[j] in " \t\n": j -= 1
                    if j >= 0 and (clean[j].isalnum() or clean[j] in ")]_"):
                        # exclude `as` casts / keywords before op (e.g. `return -1`)
                        word = re.search(r'([A-Za-z_]\w*)$', clean[max(0, j-12):j+1])
                        if not (word and word.group(1) in ("return", "in", "if", "else", "match", "break", "as", "mut", "let")):
                            ok = True
            if not ok: continue
            # skip pattern/attribute contexts: line starts with #[ or contains '=>' before literal on same line
            line_start = clean.rfind("\n", 0, s) + 1
            line = clean[line_start:s]
            if line.lstrip().startswith("#") or "=>" in line.split("{")[-1] and re.match(r'\s*[^=]*=>\s*$', line) : continue
            text = src[s:e]
            val = text.replace("_", "")
            lit_val = None
            try:
                core = re.sub(r'(u8|u16|u32|u64|u128|usize|i8|i16|i32|i64|i128|isize)$', '', val)
                lit_val = int(core, 0)
            except ValueError:
                pass
            rep = f"({text} + 1 - 1)" if lit_val == 0 else f"({text} - 1 + 1)"
            edits.append((s, e, rep)); n_lit += 1
            sites.append((rel, src.count("\n", 0, s) + 1))
    edits.sort(key=lambda t: t[0], reverse=True)
    out = src
    for s, e, rep in edits:
        out = out[:s] + rep + out[e:]
    open(path, "w", encoding="utf-8").write(out)
    stats[rel] = (n_unsafe, n_lit)
if sites_file:
    with open(sites_file, "w", encoding="utf-8") as handle:
        json.dump(sites, handle)
for k, v in stats.items(): print(f"{k}: unsafe-stripped={v[0]} literals-wrapped={v[1]}")
