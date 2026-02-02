/*
 * sensor_reading.c — Blog post test cases for struct packing strategies.
 *
 * Compile for RISC-V 32-bit:
 *   riscv64-elf-gcc -march=rv32imac -mabi=ilp32 -O2 -ffreestanding -c sensor_reading.c
 *
 * Three variants of the same struct, differing only in packing:
 *   1. pack(1)           — no padding, alignment 1
 *   2. packed+aligned(4) — no padding, alignment 4
 *   3. unpacked          — natural padding, natural alignment
 */

#include <stdint.h>
#include <stddef.h>

/* ── Variant 1: #pragma pack(1) ────────────────────────────────── */

#pragma pack(push, 1)
typedef struct {
    int64_t  timestamp;       /* offset 0,  8 bytes */
    int32_t  temperature_mc;  /* offset 8,  4 bytes */
    int32_t  salinity_ppt;    /* offset 12, 4 bytes */
    uint8_t  status_flags;    /* offset 16, 1 byte  */
    uint16_t battery_mv;      /* offset 17, 2 bytes */
} sensor_reading_pack1_t;
#pragma pack(pop)

_Static_assert(sizeof(sensor_reading_pack1_t) == 19, "pack1 size");
_Static_assert(offsetof(sensor_reading_pack1_t, timestamp) == 0, "pack1 timestamp");
_Static_assert(offsetof(sensor_reading_pack1_t, temperature_mc) == 8, "pack1 temperature_mc");
_Static_assert(offsetof(sensor_reading_pack1_t, salinity_ppt) == 12, "pack1 salinity_ppt");
_Static_assert(offsetof(sensor_reading_pack1_t, status_flags) == 16, "pack1 status_flags");
_Static_assert(offsetof(sensor_reading_pack1_t, battery_mv) == 17, "pack1 battery_mv");

/* ── Variant 2: __attribute__((packed, aligned(4))) ────────────── */

typedef struct __attribute__((packed, aligned(4))) {
    int64_t  timestamp;       /* offset 0,  8 bytes */
    int32_t  temperature_mc;  /* offset 8,  4 bytes */
    int32_t  salinity_ppt;    /* offset 12, 4 bytes */
    uint8_t  status_flags;    /* offset 16, 1 byte  */
    uint16_t battery_mv;      /* offset 17, 2 bytes */
} sensor_reading_pa4_t;

_Static_assert(sizeof(sensor_reading_pa4_t) == 20, "pa4 size (rounded to 4)");
_Static_assert(offsetof(sensor_reading_pa4_t, timestamp) == 0, "pa4 timestamp");
_Static_assert(offsetof(sensor_reading_pa4_t, temperature_mc) == 8, "pa4 temperature_mc");
_Static_assert(offsetof(sensor_reading_pa4_t, salinity_ppt) == 12, "pa4 salinity_ppt");
_Static_assert(offsetof(sensor_reading_pa4_t, status_flags) == 16, "pa4 status_flags");
_Static_assert(offsetof(sensor_reading_pa4_t, battery_mv) == 17, "pa4 battery_mv");

/* ── Variant 3: unpacked (natural alignment) ───────────────────── */

typedef struct {
    int64_t  timestamp;       /* offset 0,  8 bytes */
    int32_t  temperature_mc;  /* offset 8,  4 bytes */
    int32_t  salinity_ppt;    /* offset 12, 4 bytes */
    uint8_t  status_flags;    /* offset 16, 1 byte  */
    uint16_t battery_mv;      /* offset 18, 2 bytes (padded for alignment) */
} sensor_reading_unpacked_t;

_Static_assert(sizeof(sensor_reading_unpacked_t) == 24, "unpacked size");
_Static_assert(offsetof(sensor_reading_unpacked_t, battery_mv) == 18, "unpacked battery_mv");


/* ── Accessor functions for disassembly comparison ─────────────── */

/*
 * Write int64_t timestamp at offset 0 (aligned in all variants).
 * pack(1): expect byte-decomposed (8 byte-stores)
 * pa4:     expect native (1-2 stores depending on arch)
 * unpacked: expect native
 */
void write_ts_pack1(sensor_reading_pack1_t *p, int64_t v)     { p->timestamp = v; }
void write_ts_pa4(sensor_reading_pa4_t *p, int64_t v)         { p->timestamp = v; }
void write_ts_unpacked(sensor_reading_unpacked_t *p, int64_t v){ p->timestamp = v; }

/*
 * Write int32_t temperature_mc at offset 8 (aligned in all variants).
 * pack(1): expect byte-decomposed (~12 instructions on Xtensa, ~8 on rv32)
 * pa4:     expect native (1 instruction)
 * unpacked: expect native
 */
void write_temp_pack1(sensor_reading_pack1_t *p, int32_t v)     { p->temperature_mc = v; }
void write_temp_pa4(sensor_reading_pa4_t *p, int32_t v)         { p->temperature_mc = v; }
void write_temp_unpacked(sensor_reading_unpacked_t *p, int32_t v){ p->temperature_mc = v; }

/*
 * Write uint16_t battery_mv at offset 17 (misaligned in pack1 and pa4).
 * pack(1): byte-decomposed
 * pa4:     byte-decomposed (correctly — offset 17 is not 2-aligned)
 * unpacked: native (offset 18 is 2-aligned)
 */
void write_bat_pack1(sensor_reading_pack1_t *p, uint16_t v)     { p->battery_mv = v; }
void write_bat_pa4(sensor_reading_pa4_t *p, uint16_t v)         { p->battery_mv = v; }
void write_bat_unpacked(sensor_reading_unpacked_t *p, uint16_t v){ p->battery_mv = v; }

/*
 * Read uint16_t battery_mv — to see load codegen and possible optimization
 * when base alignment is known (pa4 case).
 */
uint16_t read_bat_pack1(sensor_reading_pack1_t *p)     { return p->battery_mv; }
uint16_t read_bat_pa4(sensor_reading_pa4_t *p)         { return p->battery_mv; }
uint16_t read_bat_unpacked(sensor_reading_unpacked_t *p){ return p->battery_mv; }

/*
 * Force struct instances so they appear in DWARF even if accessors are
 * optimized away.
 */
volatile sensor_reading_pack1_t     g_pack1;
volatile sensor_reading_pa4_t       g_pa4;
volatile sensor_reading_unpacked_t  g_unpacked;
