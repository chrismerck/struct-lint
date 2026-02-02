# Reproducing the Blog Post Findings

This guide explains how to reproduce every result in `FINDINGS.md` from scratch. It covers toolchain setup, the build process, what each output file contains, and how to verify each of the 8 claims made in the blog post.

## Prerequisites

### Host System

Results were generated on:
- macOS 15.7.3 (Darwin arm64 / Apple Silicon)
- Rust 1.89.0
- Python 3.11.13
- pyelftools 0.32

### Cross-Compiler Toolchains

Three bare-metal cross-compilers are required. On macOS with Homebrew:

```bash
# RISC-V 32-bit (primary architecture)
brew install riscv64-elf-gcc
# Provides: riscv64-elf-gcc, riscv64-elf-objdump
# Tested version: GCC 15.2.0

# ARM Cortex-M0
brew install arm-none-eabi-gcc
# Provides: arm-none-eabi-gcc, arm-none-eabi-objdump
# Tested version: GCC 10.3.1

# Xtensa ESP32 (via ESP-IDF / espup)
# The Makefile expects the compiler at:
#   $HOME/.rustup/toolchains/esp/xtensa-esp-elf/esp-14.2.0_20240906/
#     xtensa-esp-elf/bin/xtensa-esp32-elf-gcc
# Install via: cargo install espup && espup install
# Or adjust XTENSA_CC / XTENSA_OBJDUMP in the Makefile to your path.
```

### Python Dependencies

```bash
pip3 install pyelftools
# Verify:
python3 -c "from elftools.elf.elffile import ELFFile; print('OK')"
```

### struct-lint (this repo)

```bash
cd /path/to/struct-lint
cargo build
# Binary at: target/debug/struct-lint
```

## Directory Layout

```
test/blog/
  sensor_reading.c            # 3 struct variants (pack1, pa4, unpacked) + accessor functions
  sensor_reading_evolved.c    # Evolved struct with misaligned error_code field
  Makefile                    # Orchestrates compilation, disassembly, lint, SVGs
  gen_svg.py                  # DWARF-driven SVG diagram generator
  FINDINGS.md                 # Analysis results
  REPRODUCE.md                # This file
  .gitignore                  # Ignores out/
  out/                        # Generated (not committed)
    rv32/                     # RISC-V 32-bit object files
    xtensa/                   # Xtensa object files
    arm/                      # ARM Cortex-M0 object files
    disasm/                   # Per-function disassembly extracts
    svg/                      # Generated SVG diagrams
```

## Step 1: Build Everything

```bash
cd test/blog
make clean    # Remove any previous artifacts
make all      # Compile + extract disassembly + run struct-lint
make svg      # Generate SVG diagrams from DWARF data
```

`make all` runs three stages:

1. **compile** -- Builds both `.c` files for all 3 architectures (6 object files total). Each compilation uses `-O2 -g` (optimized with debug info). The `_Static_assert` checks in the source files verify struct sizes and offsets at compile time -- if any assertion fails, compilation stops.

2. **disasm** -- Extracts per-function disassembly from the object files into individual `.s` files. Produces 19 files:
   - 13 from RISC-V: 12 accessor functions from `sensor_reading.c` + `write_error` from the evolved struct
   - 3 from Xtensa: `write_temp_{pack1,pa4,unpacked}` for cross-architecture comparison
   - 3 from ARM: same three functions

3. **lint** -- Runs `struct-lint` on the compiled RISC-V objects. Two runs: verbose on all rv32 objects, and specific on the evolved struct.

`make svg` runs `gen_svg.py`, which reads DWARF debug info from the rv32 object files and generates 3 SVG diagrams.

## Step 2: Verify Struct Sizes (Claim 0, Claim 7)

The `_Static_assert` declarations in the source files verify sizes at compile time. If `make compile` succeeds, these are proven:

| Variant | sizeof | Assertion |
|---------|--------|-----------|
| `sensor_reading_pack1_t` | 19 | `sensor_reading.c:28` |
| `sensor_reading_pa4_t` | 20 | `sensor_reading.c:45` |
| `sensor_reading_unpacked_t` | 24 | `sensor_reading.c:62` |
| `sensor_reading_evolved_t` | 24 | `sensor_reading_evolved.c:23` |

**Claim 0** (unpacked wastes space): 24 bytes vs 19 bytes = 26% overhead.

**Claim 7** (aligned(4) changes array stride): sizeof pack1 = 19 vs sizeof pa4 = 20. In an array, `pack1[1]` starts at byte 19 but `pa4[1]` starts at byte 20. The 1-byte difference per element compounds.

## Step 3: Verify Disassembly Claims

All disassembly files are in `out/disasm/`. Each file contains one function's disassembly extracted from `objdump -d` output.

### Claim 1: pack(1) byte-decomposes ALL field accesses, even aligned ones

**Key file:** `out/disasm/rv32-write_temp_pack1.s`

This writes `int32_t temperature_mc` at offset 8. Offset 8 is naturally aligned for a 4-byte value -- yet pack(1) forces the compiler to decompose it into byte stores:

```asm
00000046 <write_temp_pack1>:
  46:   srli    a3,a1,0x8       # extract byte 1
  4a:   srli    a4,a1,0x10      # extract byte 2
  4e:   srli    a5,a1,0x18      # extract byte 3
  52:   sb      a1,8(a0)        # store byte 0
  56:   sb      a3,9(a0)        # store byte 1
  5a:   sb      a4,10(a0)       # store byte 2
  5e:   sb      a5,11(a0)       # store byte 3
```

7 instructions (3 shifts + 4 byte-stores) for a single 32-bit write. The compiler cannot assume the struct base is aligned, so even offset 8 gets decomposed.

For the most dramatic example, check `out/disasm/rv32-write_ts_pack1.s` -- writing `int64_t timestamp` at offset 0 produces 14 instructions (6 shifts + 8 byte-stores).

### Claim 2: packed+aligned(4) generates native access for aligned fields

**Key file:** `out/disasm/rv32-write_temp_pa4.s`

Same field, same offset, but with `packed+aligned(4)`:

```asm
00000064 <write_temp_pa4>:
  64:   sw      a1,8(a0)        # single native 32-bit store
```

1 instruction. The `aligned(4)` attribute tells the compiler the struct base is 4-aligned, so offset 8 is 4-aligned, and a native `sw` is safe.

**Comparison file:** `out/disasm/rv32-write_temp_unpacked.s` -- identical single `sw`, confirming pa4 matches unpacked performance for aligned fields.

### Claim 3: Genuinely misaligned fields are still byte-decomposed with pa4

**Key file:** `out/disasm/rv32-write_bat_pa4.s`

Writes `uint16_t battery_mv` at offset 17. Even with `aligned(4)`, offset 17 is odd (not 2-aligned), so the compiler correctly byte-decomposes:

```asm
0000007a <write_bat_pa4>:
  7a:   srli    a5,a1,0x8       # extract high byte
  7e:   sb      a1,17(a0)       # store low byte
  82:   sb      a5,18(a0)       # store high byte
```

3 instructions -- same as pack(1) for this field (`out/disasm/rv32-write_bat_pack1.s`). The `aligned(4)` attribute only helps where the alignment math works out.

**Comparison:** `out/disasm/rv32-write_bat_unpacked.s` uses a single `sh` (store halfword) because the unpacked struct places battery_mv at offset 18, which IS 2-aligned.

### Claim 4: Reads of misaligned fields can be optimized with known base alignment

**Key files:** `out/disasm/rv32-read_bat_pack1.s` vs `out/disasm/rv32-read_bat_pa4.s`

pack(1) uses two byte-loads:
```asm
0000008e <read_bat_pack1>:
  8e:   lbu     a5,18(a0)       # load high byte
  92:   lbu     a0,17(a0)       # load low byte
```

packed+aligned(4) uses a single aligned word-load:
```asm
0000009c <read_bat_pa4>:
  9c:   lw      a0,16(a0)       # load full 32-bit word from aligned offset 16
```

The compiler knows offset 16 is 4-aligned (relative to a 4-aligned base), so it loads a full word from there and lets the caller extract bytes 17-18. This is an aggressive optimization -- 1 instruction vs 2.

### Claim 5: This isn't platform-specific

**Key files:** `out/disasm/xtensa-write_temp_*.s` and `out/disasm/arm-write_temp_*.s`

Count the body instructions (excluding prologue/epilogue) for each architecture:

| Architecture | pack(1) body | pa4 body | Ratio |
|-------------|-------------|----------|-------|
| RISC-V 32 | 7 (3 `srli` + 4 `sb`) | 1 (`sw`) | 7x |
| Xtensa | 7 (3 `extui` + 4 `s8i`) | 1 (`s32i.n`) | 7x |
| ARM Cortex-M0 | 7 (2 `lsrs` + `lsls` + 4 `strb`) | 1 (`str`) | 7x |

The 7x ratio is consistent. Different instruction mnemonics, same pattern: pack(1) decomposes aligned 32-bit stores into 7 operations across all tested architectures.

To count body instructions, subtract function prologue/epilogue:
- Xtensa: subtract `entry` and `retw.n`
- ARM: subtract `bx lr` (and `nop` padding if present)
- RISC-V: no prologue/epilogue for leaf functions

### Claim 6: Adding a field silently creates misalignment

**Key file:** `out/disasm/rv32-write_error.s`

The evolved struct adds `uint32_t error_code` at offset 19 (not 4-aligned):

```asm
00000000 <write_error>:
   0:   srli    a3,a1,0x8
   4:   srli    a4,a1,0x10
   8:   srli    a5,a1,0x18
   c:   sb      a1,19(a0)
  10:   sb      a3,20(a0)
  14:   sb      a4,21(a0)
  18:   sb      a5,22(a0)
```

7 instructions for a uint32_t write that should be 1 `sw`. This is the "struct evolution" hazard.

**struct-lint detection:** Run `make lint` and look for:
```
timestamp.timestamp (timestamp, 4 bytes) at offset 19 not naturally aligned (needs 4)
```
(The struct name displays as "timestamp" rather than the typedef name due to a known DWARF resolution limitation for anonymous typedef'd structs.)

## Step 4: Verify SVG Diagrams

```bash
ls -la out/svg/
```

Three SVG files:

| File | Shows | How to verify |
|------|-------|---------------|
| `padding-waste.svg` | Unpacked (24B) vs pack(1) (19B) side-by-side | Open in browser. Unpacked row should have red-dashed padding bytes between status_flags and battery_mv, and at the end. Pack(1) row should have no padding. |
| `field-access.svg` | packed+aligned(4) struct with color-coded access | Green = native access (timestamp, temperature_mc, salinity_ppt, status_flags). Orange = byte-decomposed (battery_mv at offset 17). |
| `struct-evolution.svg` | Before (pa4, 20B) and after (evolved, 24B) | Top row shows original struct. Bottom row adds error_code in red at offset 19, highlighting the misalignment. |

All data in these SVGs comes from DWARF debug info via `pyelftools` -- the script reads struct member offsets and sizes from the compiled ELF files. The RISC-V `.o` files require relocation patching (handled automatically by `gen_svg.py`) because pyelftools doesn't natively support `R_RISCV_32` relocations in debug sections.

## Step 5: Verify struct-lint Output

Run struct-lint independently:

```bash
# Verbose on all rv32 objects
../../target/debug/struct-lint -v out/rv32/

# Just the evolved struct
../../target/debug/struct-lint out/rv32/sensor_reading_evolved.o
```

Expected detections:
- `battery_mv` (2 bytes) at offset 17: not 2-aligned (in pack1 and pa4 variants)
- `error_code` (4 bytes) at offset 19: not 4-aligned (in evolved struct)

## Source Files Reference

### sensor_reading.c

Defines 3 variants of the same struct with identical fields but different packing:

| Variant | Packing attribute | sizeof |
|---------|------------------|--------|
| `sensor_reading_pack1_t` | `#pragma pack(push, 1)` | 19 |
| `sensor_reading_pa4_t` | `__attribute__((packed, aligned(4)))` | 20 |
| `sensor_reading_unpacked_t` | (none -- natural alignment) | 24 |

Fields (same in all variants):

| Field | Type | Size | Offset (packed) | Offset (unpacked) |
|-------|------|------|-----------------|-------------------|
| `timestamp` | `int64_t` | 8 | 0 | 0 |
| `temperature_mc` | `int32_t` | 4 | 8 | 8 |
| `salinity_ppt` | `int32_t` | 4 | 12 | 12 |
| `status_flags` | `uint8_t` | 1 | 16 | 16 |
| `battery_mv` | `uint16_t` | 2 | 17 | 18 |

12 accessor functions for disassembly comparison:
- `write_ts_{pack1,pa4,unpacked}` -- write int64_t at offset 0
- `write_temp_{pack1,pa4,unpacked}` -- write int32_t at offset 8
- `write_bat_{pack1,pa4,unpacked}` -- write uint16_t at offset 17/18
- `read_bat_{pack1,pa4,unpacked}` -- read uint16_t at offset 17/18

3 volatile global instances force structs into DWARF even if functions are optimized away.

### sensor_reading_evolved.c

Demonstrates struct evolution: the original pa4 struct with an appended `uint32_t error_code` at offset 19 (misaligned). One accessor function (`write_error`) and one volatile global.

### Compiler Flags

All targets use `-O2` (optimization) and `-g` (debug info / DWARF):

| Target | Compiler | Architecture flags |
|--------|----------|--------------------|
| RISC-V 32 | `riscv64-elf-gcc` | `-march=rv32imac -mabi=ilp32 -ffreestanding` |
| Xtensa | `xtensa-esp32-elf-gcc` | (defaults to ESP32 Xtensa LX6) |
| ARM Cortex-M0 | `arm-none-eabi-gcc` | `-mcpu=cortex-m0 -mthumb -ffreestanding` |

`-ffreestanding` is needed for rv32 and ARM because their bare-metal toolchains lack a sysroot with `stdint.h` -- the flag uses GCC's built-in headers. The Xtensa ESP32 toolchain includes its own sysroot and doesn't need it.

## Troubleshooting

**Xtensa compiler not found:** The Makefile hardcodes the path via `$(HOME)/.rustup/toolchains/esp/...`. If your Xtensa toolchain is elsewhere, edit `XTENSA_CC` and `XTENSA_OBJDUMP` in the Makefile, or run with just rv32 and ARM:
```bash
make RV32_OBJS="out/rv32/sensor_reading.o out/rv32/sensor_reading_evolved.o" \
     ARM_OBJS="out/arm/sensor_reading.o out/arm/sensor_reading_evolved.o" \
     XTENSA_OBJS="" compile disasm lint
```

**_Static_assert fails:** Your compiler's ABI differs from the expected layout. This would be a significant finding -- document the compiler version and target triple.

**Empty disassembly files:** The awk extraction pattern expects objdump to output function labels as `<function_name>:`. If using a different objdump version, the format may differ. Check with `riscv64-elf-objdump -d out/rv32/sensor_reading.o | head -30`.

**gen_svg.py fails with garbled struct names:** The script patches RISC-V ELF relocations internally. If DWARF format or relocation types change with a different GCC version, the patching logic may need updating. Run with `--help` for usage.
