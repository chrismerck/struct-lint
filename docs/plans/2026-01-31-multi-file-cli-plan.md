# Multi-file CLI Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Extend struct-lint from a single-file POC into a multi-file CLI with cross-file deduplication, verbose output, and configurable checks.

**Architecture:** Add clap for CLI parsing, walk directories for `.o`/`.elf` files, accumulate structs and issues across files in a BTreeMap keyed by struct name + member layout, then report once at the end.

**Tech Stack:** Rust 2024, clap (derive), gimli, object, regex, walkdir

---

### Task 1: Add clap and walkdir dependencies

**Files:**
- Modify: `Cargo.toml`

**Step 1: Add dependencies**

Add `clap` and `walkdir` to `Cargo.toml`:

```toml
[package]
name = "struct-lint"
version = "0.1.0"
edition = "2024"

[dependencies]
clap = { version = "4", features = ["derive"] }
gimli = "0.33"
object = { version = "0.38", features = ["read"] }
regex = "1"
walkdir = "2"
```

**Step 2: Verify it compiles**

Run: `cargo build`
Expected: compiles with no errors (warnings about unused imports are fine)

**Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "deps: add clap and walkdir"
```

---

### Task 2: Replace manual arg parsing with clap CLI struct

**Files:**
- Modify: `src/main.rs:1-8` (imports)
- Modify: `src/main.rs:60-66` (current arg parsing in main)

**Step 1: Add the clap CLI struct and new imports**

At the top of `src/main.rs`, add `use clap::Parser;` and `use walkdir::WalkDir;` to the imports. Replace `use std::env;` since we'll only use it for `current_dir` in `make_relative`. Add:

```rust
use std::collections::BTreeMap;
use std::path::PathBuf;
```

After the existing type/struct definitions (after line 49), add the clap struct:

```rust
#[derive(Parser)]
#[command(name = "struct-lint")]
#[command(about = "Detect struct alignment issues in ELF binaries via DWARF debug info")]
#[command(version)]
struct Cli {
    /// ELF files or directories to scan (recursive .o/.elf search)
    #[arg(required = true)]
    paths: Vec<PathBuf>,

    /// Regex pattern for structs that should be packed
    #[arg(short, long, default_value = r"_(rec|pkt(_\w+)?|header)_t$")]
    pattern: String,

    /// Suppress summary line, only print issues
    #[arg(short, long)]
    quiet: bool,

    /// Also print structs that passed checks
    #[arg(short, long)]
    verbose: bool,

    /// Skip "should be packed" detection
    #[arg(long)]
    no_packed_check: bool,

    /// Skip misaligned member detection
    #[arg(long)]
    no_alignment_check: bool,
}
```

**Step 2: Replace the arg parsing in main**

Replace lines 60-66 (the current `let args` through `std::process::exit(2)`) with:

```rust
fn main() {
    let cli = Cli::parse();
```

For now, keep the rest of main temporarily broken -- we'll fix it in the next task. Just make sure the clap struct compiles.

**Step 3: Verify help output**

Run: `cargo run -- --help`
Expected: Displays the help text with all flags. Verify it matches the design. Copy the output and include it in the commit message for the user to review.

**Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat: add clap CLI with all flags

<paste help output here for review>"
```

---

### Task 3: Implement path collection (Phase 1)

**Files:**
- Modify: `src/main.rs` (main function)

**Step 1: Write the `collect_elf_paths` function**

Add this function before `main()`:

```rust
fn collect_elf_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut elf_paths = Vec::new();
    for path in paths {
        if path.is_dir() {
            for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
                let p = entry.into_path();
                if let Some(ext) = p.extension() {
                    if ext == "o" || ext == "elf" {
                        elf_paths.push(p);
                    }
                }
            }
        } else {
            elf_paths.push(path.clone());
        }
    }
    elf_paths
}
```

**Step 2: Wire it into main**

After `let cli = Cli::parse();`, add:

```rust
    let elf_paths = collect_elf_paths(&cli.paths);
    if elf_paths.is_empty() {
        eprintln!("No ELF files found in the specified paths.");
        std::process::exit(2);
    }
```

**Step 3: Test with directory input**

Run: `cargo run -- test/`
Expected: Should find `test/test_structs_xtensa.o` and process it (the rest of main still handles single-file logic, so it may error -- that's fine, we just need to verify the path collection works). Add a temporary `eprintln!("Found {} ELF files", elf_paths.len());` to verify.

**Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat: collect ELF paths from files and directories"
```

---

### Task 4: Implement multi-file analysis loop with cross-file dedup (Phase 2)

**Files:**
- Modify: `src/main.rs` (main function, `analyze_structs` signature)

This is the core refactor. We change `main()` to loop over all ELF files, accumulate deduplicated structs and their issues, then report once.

**Step 1: Modify `analyze_structs` to accept pattern and check flags**

Change the signature from:

```rust
fn analyze_structs(structs: &[StructInfo], max_align: u64) -> Vec<Issue> {
```

to:

```rust
fn analyze_structs(structs: &[StructInfo], max_align: u64, pattern: &regex::Regex, no_packed_check: bool, no_alignment_check: bool) -> Vec<Issue> {
```

Replace the hardcoded regex on line 470 (`let pack_pattern = ...`) with the passed-in `pattern`. Wrap the packed-check block (lines 475-493) with `if !no_alignment_check { ... }`. Wrap the not-packed block (lines 494-508) with `if !no_packed_check { ... }`. Use `pattern` directly instead of `pack_pattern`.

**Step 2: Rewrite `main()` with the three-phase structure**

Replace everything in `main()` after `collect_elf_paths` with:

```rust
    let pattern = regex::Regex::new(&cli.pattern).unwrap_or_else(|e| {
        eprintln!("Invalid pattern '{}': {}", cli.pattern, e);
        std::process::exit(2);
    });

    // Phase 2: Analyze all files, accumulate deduplicated structs + issues
    // Key: struct name + member layout string, Value: (StructInfo, Vec<Issue>)
    let mut global_structs: BTreeMap<String, (StructInfo, Vec<Issue>)> = BTreeMap::new();
    let mut file_count: usize = 0;

    for path in &elf_paths {
        let data = match fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Warning: cannot read {}: {}", path.display(), e);
                continue;
            }
        };
        let data: &'static [u8] = Box::leak(data.into_boxed_slice());

        let obj = match object::File::parse(data) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("Warning: cannot parse {}: {}", path.display(), e);
                continue;
            }
        };

        let max_align: u64 = if obj.is_64() { 8 } else { 4 };

        let dwarf = match gimli::Dwarf::load(|section_id| -> Result<R, gimli::Error> {
            let data = obj
                .section_by_name(section_id.name())
                .map(|s| s.data().unwrap_or(&[]))
                .unwrap_or(&[]);
            Ok(EndianSlice::new(data, LittleEndian))
        }) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Warning: cannot load DWARF from {}: {}", path.display(), e);
                continue;
            }
        };

        let structs = extract_structs(&dwarf);
        let issues = analyze_structs(&structs, max_align, &pattern, cli.no_packed_check, cli.no_alignment_check);
        file_count += 1;

        // Build issue map keyed by struct name for this file
        let mut issue_map: HashMap<String, Vec<Issue>> = HashMap::new();
        for issue in issues {
            let key = match &issue {
                Issue::MisalignedMember { struct_name, .. } => struct_name.clone(),
                Issue::NotPacked { struct_name, .. } => struct_name.clone(),
            };
            issue_map.entry(key).or_default().push(issue);
        }

        // Merge into global map with dedup
        for s in structs {
            let dedup_key = format!(
                "{}:{}",
                s.name,
                s.members
                    .iter()
                    .map(|m| format!("{}@{}", m.name, m.offset))
                    .collect::<Vec<_>>()
                    .join(",")
            );
            global_structs
                .entry(dedup_key)
                .or_insert_with(|| {
                    let issues = issue_map.remove(&s.name).unwrap_or_default();
                    (s, issues)
                });
        }
    }
```

**Step 3: Verify multi-file analysis works**

Run: `cargo run -- test/test_structs_xtensa.o`
Expected: Same 3 issues as before (output may differ slightly in format since we haven't written Phase 3 yet).

**Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat: multi-file analysis loop with cross-file dedup"
```

---

### Task 5: Implement reporting (Phase 3)

**Files:**
- Modify: `src/main.rs` (main function, after the analysis loop)

**Step 1: Write the reporting phase**

After the analysis loop in `main()`, add:

```rust
    // Phase 3: Report
    let mut total_issues = 0usize;
    let mut structs_with_issues = 0usize;
    let mut alignment_issues = 0usize;
    let mut packing_issues = 0usize;
    let total_structs = global_structs.len();

    for (_key, (s, issues)) in &global_structs {
        if !issues.is_empty() {
            structs_with_issues += 1;
            for issue in issues {
                total_issues += 1;
                match issue {
                    Issue::MisalignedMember {
                        struct_name,
                        member_name,
                        type_name,
                        member_size,
                        offset,
                        natural_alignment,
                        decl_file,
                        decl_line,
                    } => {
                        alignment_issues += 1;
                        println!(
                            "{}:{}: {}.{} ({}, {} bytes) at offset {} not naturally aligned (needs {})",
                            make_relative(decl_file),
                            decl_line,
                            struct_name,
                            member_name,
                            type_name,
                            member_size,
                            offset,
                            natural_alignment,
                        );
                    }
                    Issue::NotPacked {
                        struct_name,
                        padding_bytes,
                        pattern,
                        decl_file,
                        decl_line,
                    } => {
                        packing_issues += 1;
                        println!(
                            "{}:{}: {} is not packed ({} bytes padding, matches pattern '{}')",
                            make_relative(decl_file),
                            decl_line,
                            struct_name,
                            padding_bytes,
                            pattern,
                        );
                    }
                }
            }
        } else if cli.verbose {
            let packed_str = if infer_packed(s, if s.size > 0 { 4 } else { 4 }) {
                "packed, "
            } else {
                ""
            };
            println!(
                "{}:{}: {} ({} bytes, {}{} members) ok",
                make_relative(&s.decl_file),
                s.decl_line,
                s.name,
                s.size,
                packed_str,
                s.members.len(),
            );
        }
    }

    // Summary line
    if !cli.quiet {
        let file_word = if file_count == 1 { "file" } else { "files" };
        if total_issues == 0 {
            println!(
                "No issues found in {} structs across {} {}",
                total_structs, file_count, file_word,
            );
        } else {
            println!(
                "\n{} issues in {} structs across {} {} ({} alignment, {} missing pack)",
                total_issues, structs_with_issues, file_count, file_word,
                alignment_issues, packing_issues,
            );
        }
    }

    if total_issues > 0 {
        std::process::exit(1);
    }
```

**Step 2: Remove old reporting code and per-file debug prints**

Delete the old reporting block that was in main (the `if issues.is_empty()` through `std::process::exit(1)` block, and the `eprintln!("ELF: ...")` and `eprintln!("Found {} structs", ...)` lines).

**Step 3: Test against the fixture**

Run: `cargo run -- test/test_structs_xtensa.o`
Expected output:
```
test/test_structs.c:7: sync_pkt_t.crc (uint32_t, 4 bytes) at offset 9 not naturally aligned (needs 4)
test/test_structs.c:7: sync_pkt_t.seq (uint16_t, 2 bytes) at offset 1 not naturally aligned (needs 2)
test/test_structs.c:28: sensor_rec_t is not packed (6 bytes padding, matches pattern '_(rec|pkt(_\w+)?|header)_t$')

3 issues in 2 structs across 1 file (2 alignment, 1 missing pack)
```

Note: order may differ from POC since BTreeMap sorts by dedup key. That's fine.

**Step 4: Test verbose mode**

Run: `cargo run -- -v test/test_structs_xtensa.o`
Expected: Same issues plus two "ok" lines for `point_t` and `well_aligned_pkt_t`.

**Step 5: Test quiet mode**

Run: `cargo run -- -q test/test_structs_xtensa.o`
Expected: Only the 3 issue lines, no summary.

**Step 6: Test check-disable flags**

Run: `cargo run -- --no-alignment-check test/test_structs_xtensa.o`
Expected: Only the `sensor_rec_t` not-packed issue (1 issue).

Run: `cargo run -- --no-packed-check test/test_structs_xtensa.o`
Expected: Only the 2 alignment issues for `sync_pkt_t`.

**Step 7: Test directory input**

Run: `cargo run -- test/`
Expected: Same output as `test/test_structs_xtensa.o` since that's the only `.o` file in the directory.

**Step 8: Commit**

```bash
git add src/main.rs
git commit -m "feat: unified reporting with verbose, quiet, and check-disable flags"
```

---

### Task 6: Fix verbose mode max_align for packed inference

**Files:**
- Modify: `src/main.rs` (the verbose output block in Phase 3)

The verbose "ok" output calls `infer_packed(s, 4)` with a hardcoded max_align. We need to store max_align alongside each struct during Phase 2.

**Step 1: Store max_align per struct**

Change the global_structs type from `BTreeMap<String, (StructInfo, Vec<Issue>)>` to `BTreeMap<String, (StructInfo, Vec<Issue>, u64)>` where the u64 is max_align.

Update the insertion:
```rust
    global_structs
        .entry(dedup_key)
        .or_insert_with(|| {
            let issues = issue_map.remove(&s.name).unwrap_or_default();
            (s, issues, max_align)
        });
```

Update the Phase 3 loop destructuring:
```rust
    for (_key, (s, issues, max_align)) in &global_structs {
```

And fix the verbose packed inference:
```rust
            let packed_str = if infer_packed(s, *max_align) {
```

**Step 2: Test verbose output**

Run: `cargo run -- -v test/test_structs_xtensa.o`
Expected: `well_aligned_pkt_t` shows as `(8 bytes, packed, 4 members) ok`.

**Step 3: Commit**

```bash
git add src/main.rs
git commit -m "fix: use per-file max_align for packed inference in verbose output"
```

---

### Task 7: Clean up and final verification

**Files:**
- Modify: `src/main.rs` (remove dead code, unused imports)

**Step 1: Remove unused imports**

Remove `use std::env;` if no longer used (check -- `make_relative` uses `env::current_dir()`). Remove any other unused imports. Run `cargo build` and fix any warnings.

**Step 2: Run full test suite**

Run: `cargo run -- test/test_structs_xtensa.o 2>&1`
Run: `cargo run -- -v test/test_structs_xtensa.o 2>&1`
Run: `cargo run -- -q test/test_structs_xtensa.o 2>&1`
Run: `cargo run -- --no-alignment-check test/test_structs_xtensa.o 2>&1`
Run: `cargo run -- --no-packed-check test/test_structs_xtensa.o 2>&1`
Run: `cargo run -- --help 2>&1`
Run: `cargo run -- test/ 2>&1`
Run: `cargo run -- 2>&1` (should show help/error and exit 2)

Verify all output matches expectations from the design.

**Step 3: Commit**

```bash
git add src/main.rs
git commit -m "chore: clean up unused imports and dead code"
```

---

### Task 8: Remove stale docs/next-steps.md

**Files:**
- Delete: `docs/next-steps.md`

The next-steps doc is now superseded by the design and plan docs.

**Step 1: Delete and commit**

```bash
git rm docs/next-steps.md
git commit -m "docs: remove next-steps.md, superseded by implementation plan"
```
