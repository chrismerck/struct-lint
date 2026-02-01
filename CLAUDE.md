# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

struct-lint is a Rust CLI tool that detects struct alignment issues in embedded firmware by reading DWARF debug info from ELF binaries. It catches two bug classes: misaligned members in packed structs (causes unaligned access traps on e.g. Xtensa/ESP32) and missing pack annotations on serialization structs (causes silent padding corruption).

## Build & Test Commands

```bash
cargo build                          # Debug build
cargo build --release                # Release build
cargo clippy                         # Lint
cargo fmt -- --check                 # Format check
./target/debug/struct-lint test/     # Run against test fixtures (expect 3 issues, exit code 1)
```

There are no automated tests (no `cargo test`). Verification is done by running the binary against `test/test_structs_xtensa.o` and checking the output.

## Architecture

Single-file tool (`src/main.rs`, ~700 lines) with this pipeline:

1. **CLI parsing** (`Cli` struct via clap derive) — accepts paths, regex pattern, verbosity/quiet/check-disable flags
2. **ELF discovery** (`collect_elf_paths()`) — walks directories finding `.o`/`.elf` files via `walkdir`
3. **DWARF extraction** (`extract_structs()`) — three-pass algorithm using `gimli` for zero-copy parsing:
   - Pass 1: collect type names/sizes
   - Pass 2: find `DW_TAG_structure_type` entries
   - Pass 3: extract member info (offsets, sizes, bitfields, typedef chains)
4. **Analysis** (`analyze_structs()`) — two checks per struct:
   - Misaligned members: in packed structs, members whose offset isn't naturally aligned
   - Missing pack: non-packed structs matching the name pattern with unused padding
5. **Global dedup** — `BTreeMap` keyed by `name:member1@offset1,...` prevents duplicate reports across files
6. **Reporting** — compiler-style diagnostics (`file:line: struct.member ...`), exit code 0/1/2

### Key design details

- Packed inference (`infer_packed()`): a struct is packed if any member sits at a misaligned offset
- Natural alignment = `min(member_size, arch_max_align)` where arch_max_align is 4 (32-bit) or 8 (64-bit), read from ELF header
- Default pattern for "should be packed" structs: `_(rec|pkt(_\w+)?|header)_t$`

## Test Fixtures

- `test/test_structs.c` — C source with 4 example structs covering all check paths
- `test/test_structs_xtensa.o` — pre-compiled 32-bit Xtensa ELF with DWARF info (committed binary)
