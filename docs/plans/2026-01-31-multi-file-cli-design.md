# Multi-file CLI Design

## Summary

Extend struct-lint from a single-file POC into a proper multi-file CLI tool
with cross-file deduplication, verbose output, and configurable checks.

## CLI Interface

```
struct-lint [OPTIONS] [PATH...]

Arguments:
  [PATH...]    ELF files or directories to scan (recursive .o/.elf search)

Options:
  -p, --pattern <REGEX>    Regex for structs that should be packed
                           [default: _(rec|pkt(_\w+)?|header)_t$]
  -q, --quiet              Suppress summary line, only print issues
  -v, --verbose            Also print structs that passed checks
      --no-packed-check    Skip "should be packed" detection
      --no-alignment-check Skip misaligned member detection
  -h, --help               Print help
  -V, --version            Print version
```

When no PATH is given, print help and exit with code 2. When a path is a
directory, recursively find all `.o` and `.elf` files within it. Non-ELF
files are silently skipped with a warning to stderr.

Exit codes: 0 = no issues, 1 = issues found, 2 = usage error or no input.

## Multi-file Input and Cross-file Deduplication

1. **Collect paths.** Expand CLI arguments: files taken as-is, directories
   walked recursively collecting `*.o` and `*.elf`. Maintain a count of ELF
   files processed.

2. **Process each file.** For each ELF file, call the existing
   `extract_structs()` and `analyze_structs()`. Collect results into a
   global pool.

3. **Deduplicate across files.** The existing dedup key is struct name +
   member layout (name, type, offset, size for each member). Lift this to a
   global `HashMap` -- first occurrence wins for source location. Issues are
   deduped the same way: keyed by struct name + issue variant + member name.

4. **Report once.** After all files are processed, print the deduplicated
   issues (and clean structs if `--verbose`). One summary line at the end.

## Implementation Changes

All work is in `src/main.rs` and `Cargo.toml`. No new source files.

**Add clap dependency.** Use clap with derive macros for the CLI struct.
Replaces current manual `std::env::args()` parsing.

**Refactor `main()` into three phases:**

- **Phase 1: Collect.** Walk input paths, build a list of ELF file paths.
  Error and exit 2 if no files found.
- **Phase 2: Analyze.** Loop over files, calling existing `extract_structs()`
  and `analyze_structs()` per file. Accumulate into a global
  `BTreeMap<DeduplicationKey, (StructInfo, Vec<Issue>)>` for deterministic
  output sorted by struct name.
- **Phase 3: Report.** Iterate deduplicated results. Print issues (always)
  and clean structs (if `--verbose`). Print summary (unless `--quiet`).

**Existing functions stay largely untouched.** `extract_structs()` and
`analyze_structs()` already work correctly for a single file. The change is
lifting deduplication and reporting out of per-file into the global level.
`infer_packed()` and issue detection logic don't change.

**`--no-packed-check` / `--no-alignment-check`** are filters applied when
collecting issues, or passed into `analyze_structs()` to skip the check.

## Output Format

### Default (issues only)

```
test_structs.c:7: sync_pkt_t.seq (uint16_t, 2 bytes) at offset 1 not naturally aligned (needs 2)
test_structs.c:7: sync_pkt_t.crc (uint32_t, 4 bytes) at offset 9 not naturally aligned (needs 4)
test_structs.c:28: sensor_rec_t is not packed (6 bytes padding, matches pattern)

3 issues in 2 structs across 1 file (2 alignment, 1 missing pack)
```

### Verbose (`-v`)

Adds lines for structs that passed checks:

```
test_structs.c:14: well_aligned_pkt_t (8 bytes, packed, 3 members) ok
test_structs.c:33: point_t (8 bytes, 2 members) ok
```

### Quiet (`-q`)

Suppresses the summary line. Only issue lines print.

### No issues

```
No issues found in 42 structs across 769 files
```
