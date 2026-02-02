# test/blog/Makefile — Reproducible experiments for the packed-struct blog post.
#
# Usage:
#   make            # Build everything, extract disassembly, run struct-lint
#   make clean      # Remove generated artifacts
#   make disasm     # Just the disassembly extraction
#   make lint       # Just the struct-lint run
#   make svg        # Generate SVG diagrams from DWARF data

SHELL := /bin/bash

# ── Toolchains ──────────────────────────────────────────────────
RV32_CC      := riscv64-elf-gcc
RV32_OBJDUMP := riscv64-elf-objdump
RV32_CFLAGS  := -march=rv32imac -mabi=ilp32 -O2 -ffreestanding -g

XTENSA_CC      := $(HOME)/.rustup/toolchains/esp/xtensa-esp-elf/esp-14.2.0_20240906/xtensa-esp-elf/bin/xtensa-esp32-elf-gcc
XTENSA_OBJDUMP := $(HOME)/.rustup/toolchains/esp/xtensa-esp-elf/esp-14.2.0_20240906/xtensa-esp-elf/bin/xtensa-esp32-elf-objdump
XTENSA_CFLAGS  := -O2 -g

ARM_CC      := arm-none-eabi-gcc
ARM_OBJDUMP := arm-none-eabi-objdump
ARM_CFLAGS  := -mcpu=cortex-m0 -mthumb -O2 -ffreestanding -g

STRUCT_LINT := ../../target/debug/struct-lint

# ── Sources and outputs ────────────────────────────────────────
SRCS     := sensor_reading.c sensor_reading_evolved.c
OUTDIR   := out
DISDIR   := $(OUTDIR)/disasm
SVGDIR   := $(OUTDIR)/svg

# Object files per target
RV32_OBJS    := $(SRCS:%.c=$(OUTDIR)/rv32/%.o)
XTENSA_OBJS  := $(SRCS:%.c=$(OUTDIR)/xtensa/%.o)
ARM_OBJS     := $(SRCS:%.c=$(OUTDIR)/arm/%.o)

# Functions to extract disassembly for (from sensor_reading.c)
DISASM_FUNCS := write_ts_pack1 write_ts_pa4 write_ts_unpacked \
                write_temp_pack1 write_temp_pa4 write_temp_unpacked \
                write_bat_pack1 write_bat_pa4 write_bat_unpacked \
                read_bat_pack1 read_bat_pa4 read_bat_unpacked

# Functions from evolved file
EVOLVED_FUNCS := write_error

.PHONY: all clean disasm lint svg

all: compile disasm lint

compile: $(RV32_OBJS) $(XTENSA_OBJS) $(ARM_OBJS)

# ── Compilation rules ───────────────────────────────────────────
$(OUTDIR)/rv32/%.o: %.c | $(OUTDIR)/rv32
	$(RV32_CC) $(RV32_CFLAGS) -c $< -o $@

$(OUTDIR)/xtensa/%.o: %.c | $(OUTDIR)/xtensa
	$(XTENSA_CC) $(XTENSA_CFLAGS) -c $< -o $@

$(OUTDIR)/arm/%.o: %.c | $(OUTDIR)/arm
	$(ARM_CC) $(ARM_CFLAGS) -c $< -o $@

$(OUTDIR)/rv32 $(OUTDIR)/xtensa $(OUTDIR)/arm $(DISDIR) $(SVGDIR):
	mkdir -p $@

# ── Disassembly extraction ─────────────────────────────────────
# Extract individual function disassembly from object files.
# Uses objdump -d and awk to isolate each function.
disasm: compile | $(DISDIR)
	@echo "=== Extracting disassembly ==="
	@# RV32 — primary architecture
	@for func in $(DISASM_FUNCS); do \
		$(RV32_OBJDUMP) -d $(OUTDIR)/rv32/sensor_reading.o | \
			awk '/^[0-9a-f]+ <'"$$func"'>:/{found=1} found{print} found && /^$$/{exit}' \
			> $(DISDIR)/rv32-$$func.s; \
	done
	@for func in $(EVOLVED_FUNCS); do \
		$(RV32_OBJDUMP) -d $(OUTDIR)/rv32/sensor_reading_evolved.o | \
			awk '/^[0-9a-f]+ <'"$$func"'>:/{found=1} found{print} found && /^$$/{exit}' \
			> $(DISDIR)/rv32-$$func.s; \
	done
	@# Xtensa — breadth
	@for func in write_temp_pack1 write_temp_pa4 write_temp_unpacked; do \
		$(XTENSA_OBJDUMP) -d $(OUTDIR)/xtensa/sensor_reading.o | \
			awk '/^[0-9a-f]+ <'"$$func"'>:/{found=1} found{print} found && /^$$/{exit}' \
			> $(DISDIR)/xtensa-$$func.s; \
	done
	@# ARM — breadth
	@for func in write_temp_pack1 write_temp_pa4 write_temp_unpacked; do \
		$(ARM_OBJDUMP) -d $(OUTDIR)/arm/sensor_reading.o | \
			awk '/^[0-9a-f]+ <'"$$func"'>:/{found=1} found{print} found && /^$$/{exit}' \
			> $(DISDIR)/arm-$$func.s; \
	done
	@echo "Disassembly files written to $(DISDIR)/"
	@wc -l $(DISDIR)/*.s | tail -1

# ── struct-lint ─────────────────────────────────────────────────
lint: compile
	@echo "=== struct-lint on rv32 objects ==="
	$(STRUCT_LINT) -v $(OUTDIR)/rv32/ || true
	@echo ""
	@echo "=== struct-lint on evolved struct ==="
	$(STRUCT_LINT) $(OUTDIR)/rv32/sensor_reading_evolved.o || true

# ── SVG diagrams ────────────────────────────────────────────────
svg: compile | $(SVGDIR)
	python3 gen_svg.py \
		--pack1 $(OUTDIR)/rv32/sensor_reading.o \
		--pa4 $(OUTDIR)/rv32/sensor_reading.o \
		--unpacked $(OUTDIR)/rv32/sensor_reading.o \
		--evolved $(OUTDIR)/rv32/sensor_reading_evolved.o \
		--outdir $(SVGDIR)

clean:
	rm -rf $(OUTDIR)
