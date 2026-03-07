## Summary

<!-- What does this PR change and why? One paragraph is enough. -->

## Type of change

- [ ] Bug fix (non-breaking, fixes a specific issue)
- [ ] New feature (non-breaking, adds functionality)
- [ ] Breaking change (existing API changes or removals)
- [ ] Documentation update
- [ ] CI / tooling / dependency update
- [ ] Refactoring (no behaviour change)

## Checklist

### Code quality
- [ ] `cargo test --all-targets` passes
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `cargo fmt` applied (no diff)
- [ ] `cargo doc --no-deps` builds without warnings
- [ ] All `unsafe` blocks have a `// SAFETY:` comment

### Documentation
- [ ] Public API changes are documented in rustdoc
- [ ] `CHANGELOG.md` updated under `[Unreleased]` (for user-facing changes)
- [ ] Book (`book/src/`) updated if the change affects extension authors
- [ ] New FFI pitfall discovered → added to `LESSONS.md` and `book/src/reference/pitfalls.md`

### Safety (for changes touching `unsafe`)
- [ ] No new panics introduced in FFI callback paths
- [ ] No new paths that could cross the FFI boundary with an unwinding panic
- [ ] `#![deny(unsafe_op_in_unsafe_fn)]` remains satisfied
