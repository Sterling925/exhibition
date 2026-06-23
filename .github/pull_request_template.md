## Summary

<!-- What does this change and why? -->

## Checklist

- [ ] `cargo test` passes
- [ ] `cargo fmt --all -- --check` is clean
- [ ] `cargo clippy --all-targets -- -D warnings` is clean
- [ ] C++ tests pass (`cmake -S src/engine/autogen_content/cpp/tests -B build -G Ninja && cmake --build build && ctest --test-dir build`), if C++ was touched
- [ ] New source files carry the SPDX header (`// SPDX-License-Identifier: MIT OR Apache-2.0`)
- [ ] Docs updated if behavior changed

## Notes

<!-- Anything reviewers should know: trade-offs, follow-ups, etc. -->
