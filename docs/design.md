# struct-lint Design

## Background

Embedded firmware codebases like bond-core-2 use C structs extensively as
record formats for network packets, flash storage, and protocol headers.
These structs are typically packed with `#pragma pack(push, 1)` or
`__attribute__((packed))` to ensure no hidden padding corrupts wire/disk
formats.

Two classes of bugs arise:

1. **Misaligned members in packed structs.** A `uint32_t` at offset 1 inside
   a packed struct causes an unaligned access. On Xtensa (ESP32), this traps
   and falls back to a slow emulation path. Reordering members can achieve
   natural alignment while keeping the struct packed.

2. **Missing pack annotations.** A struct intended for serialization but
   lacking a pack pragma silently contains compiler-inserted padding, leading
   to format corruption when transmitted or stored.

These issues must be caught before PRs are merged to avoid breaking backwards
compatibility of existing wire/disk formats.

An acquaintance suggested inspecting object files alongside source code to
detect these problems. This design describes a standalone Rust CLI tool that
does exactly that, using DWARF debug information as the authoritative source
of struct layouts.

## Goal

Build a fast, standalone Rust CLI tool called **struct-lint** that analyzes
ELF binaries to detect struct alignment issues. The tool is designed to be
consumed by both humans and AI agents (Claude Code) as part of PR review
workflows.

## Architecture

struct-lint reads DWARF debug information from ELF object files (`.o`) and
executables (`.elf`). It uses the `gimli` crate for zero-copy DWARF parsing
and the `object` crate for ELF file handling. No source code parsing is
required -- DWARF contains complete struct layout information including member
names, types, sizes, offsets, and source file/line references.

### Key Dependencies

- **gimli** -- DWARF parsing (zero-copy, fast; used by cargo-bloat, addr2line)
- **object** -- ELF file parsing (companion to gimli)
- **regex** -- struct name pattern matching
- **clap** -- CLI argument parsing

### Target Architecture Detection

The tool reads the ELF header to determine the target architecture:
- 32-bit ELF: natural alignment capped at 4 bytes
- 64-bit ELF: natural alignment capped at 8 bytes

A member's natural alignment is `min(member_size, arch_max_alignment)`. For
example, a `uint16_t` needs 2-byte alignment on any architecture; a
`uint32_t` needs 4-byte alignment on 32-bit targets.

## Detection Logic

### Packed Inference from DWARF

A struct is inferred as packed if DWARF shows members placed with no padding
where padding would normally be required. Specifically: if any member with
natural alignment > 1 sits at an offset that is not a multiple of its natural
alignment, the compiler must have been told to pack. The tool also verifies
that the struct's total size equals the sum of member sizes (no trailing
padding).

### Check 1: Misaligned Members in Packed Structs

For each packed struct, iterate members and flag any where
`offset % natural_alignment != 0`. Report the struct name, member name, type,
offset, and required alignment.

Bitfield members are skipped -- they have sub-byte layouts and alignment does
not apply to them in the conventional sense.

### Check 2: Should-Be-Packed Structs

For each non-packed struct whose name matches the `-p` regex, flag it. Report
the struct name, source file/line, and the amount of padding the compiler
inserted.

### Deduplication

The same struct typedef appears in every `.o` file that includes its header.
The tool deduplicates by struct name + member layout, reporting each unique
struct once with the first source location found.

## CLI Interface

```
struct-lint - Detect struct alignment issues in ELF binaries

USAGE:
    struct-lint [OPTIONS] [PATH...]

ARGS:
    [PATH...]    ELF files, object files, or directories to scan.
                 Directories are searched recursively for .o and .elf files.
                 Defaults to current directory.

OPTIONS:
    -p, --pattern <REGEX>...    Struct name patterns to flag as "must be packed"
                                 [default: _(rec|pkt(_\w+)?|header)_t$]
    -g, --glob <GLOB>           Filter which ELF files to scan
    -f, --format <FMT>          Output format: text, json  [default: text]
    -q, --quiet                 Only output issues (no summary)
        --no-packed-check       Skip "should be packed" detection
        --no-alignment-check    Skip natural alignment analysis
        --list-structs          List all structs found (diagnostic)

EXIT CODES:
    0    No issues found
    1    Issues found
    2    Error (bad input, no ELF files found, etc.)
```

### Output Format

Compiler-style diagnostics (text, default):

```
proto/BondSync/BondSync_Common.h:42: bond_sync_pkt_t.seq (uint16_t, 2 bytes) at offset 1 not naturally aligned (needs 2)
proto/BondSync/BondSync_Common.h:42: bond_sync_pkt_t.crc (uint32_t, 4 bytes) at offset 19 not naturally aligned (needs 4)
feature/BFeature/BFeature_Internal.h:88: bfeature_rec_t is not packed (12 bytes padding, matches pattern '_rec_t$')

3 issues found in 847 structs (2 alignment, 1 missing pack) across 312 ELF files
```

JSON output (`-f json`) is also supported for internal testing and CI
integration, using JSON Lines format (one object per issue).

### Usage Examples

```bash
# Scan entire build directory
struct-lint target/zermatt-pro/build/

# Check just one component
struct-lint target/zermatt-pro/build/BFeature/

# Check specific files
struct-lint target/zermatt-pro/build/BFeature/BFeature.o

# Full project scan, quiet mode for CI
struct-lint -q target/zermatt-pro/build/ || echo "alignment issues found"

# Custom patterns
struct-lint -p '_frame_t$' -p '_wire_\w+_t$' target/
```

## Claude Code Integration

A standalone Claude Code skill `struct-alignment-review` wraps the tool for
use in PR review workflows. The skill:

1. Runs `struct-lint` against build artifacts in text mode.
2. Parses the compiler-style output.
3. Cross-references findings with the PR's changed files to distinguish new
   issues from pre-existing ones.
4. For new structs introduced in the PR: recommends fixing before merge.
5. For existing shipped structs: warns but does not recommend reordering
   (backwards compatibility).
6. Uses AI judgment to identify structs that should be records/packets but
   don't follow naming conventions.
7. Re-runs struct-lint after fixes to verify resolution.

### Triggering

The skill is referenced in consuming projects' CLAUDE.md:

```
## Struct Alignment
Before creating PRs that touch C struct definitions, run the
struct-alignment-review skill to check for alignment issues.
```

The skill is also invocable on demand by the user or agent.

## Non-Goals

- Source code parsing (DWARF is the source of truth)
- Automatic reordering / fix generation (the tool flags, the agent reasons)
- Build system integration (the tool operates on already-built artifacts)
- Supporting non-ELF formats (Mach-O, PE) in the initial version
