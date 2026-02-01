# struct-lint

Detect struct alignment issues in embedded firmware by reading DWARF debug info from ELF binaries.

Catches two bug classes:

1. **Misaligned members in packed structs** -- members whose offset isn't naturally aligned, causing unaligned access traps (e.g. Xtensa, ARM Cortex-M0) or byte-decomposed codegen
2. **Missing pack annotations** -- structs matching a naming pattern (e.g. `_rec_t`, `_pkt_t`, `_header_t`) that have unused padding, suggesting a forgotten `__attribute__((packed))`

## Install

From GitHub:

```
cargo install --git https://github.com/chrismerck/struct-lint
```

Or clone and build locally:

```
git clone https://github.com/chrismerck/struct-lint
cd struct-lint
cargo build --release
```

## Usage

Point it at ELF files or directories containing `.o`/`.elf` files:

```
struct-lint path/to/build/
```

It walks directories recursively, extracts struct definitions from DWARF debug info, and reports issues:

```
$ struct-lint test/
test/test_structs.c:28: sensor_rec_t is not packed (6 bytes padding, matches pattern '_(rec|pkt(_\w+)?|header)_t$')
test/test_structs.c:7: sync_pkt_t.seq (uint16_t, 2 bytes) at offset 1 not naturally aligned (needs 2)
test/test_structs.c:7: sync_pkt_t.crc (uint32_t, 4 bytes) at offset 9 not naturally aligned (needs 4)

3 issues in 2 structs across 2 files (2 alignment, 1 missing pack)
```

Exit codes: 0 = no issues, 1 = issues found, 2 = error.

### Options

```
-p, --pattern <REGEX>   Regex for structs that should be packed [default: _(rec|pkt(_\w+)?|header)_t$]
-v, --verbose           Also print structs that passed checks
-q, --quiet             Suppress summary line
--no-packed-check       Skip "should be packed" detection
--no-alignment-check    Skip misaligned member detection
```

### Custom patterns

The default pattern matches common embedded naming conventions (`_rec_t`, `_pkt_t`, `_pkt_foo_t`, `_header_t`). Override it to match your codebase:

```
struct-lint -p '_msg_t$' path/to/build/
```

## How it works

struct-lint reads ELF binaries using `gimli` for zero-copy DWARF parsing. It does not require source code -- just compiled objects with debug info (`-g`). The analysis is:

1. Find all `DW_TAG_structure_type` entries in DWARF
2. For packed structs: check if each member's offset is naturally aligned (`offset % min(member_size, arch_max_align) == 0`)
3. For structs matching the name pattern: check if `sizeof(struct)` equals the sum of member sizes (i.e. no padding)

Natural alignment uses the ELF header to determine the architecture's maximum alignment (4 for 32-bit, 8 for 64-bit).

## License

MIT
