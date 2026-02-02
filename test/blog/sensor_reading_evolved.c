/*
 * sensor_reading_evolved.c — Struct evolution scenario.
 *
 * The original sensor_reading_pa4_t was 19 bytes of content (20 with
 * alignment padding). Someone adds error_code at the end. It lands at
 * offset 19 — not naturally aligned for a uint32_t.
 *
 * struct-lint should flag this.
 */

#include <stdint.h>
#include <stddef.h>

typedef struct __attribute__((packed, aligned(4))) {
    int64_t  timestamp;       /* offset 0,  8 bytes */
    int32_t  temperature_mc;  /* offset 8,  4 bytes */
    int32_t  salinity_ppt;    /* offset 12, 4 bytes */
    uint8_t  status_flags;    /* offset 16, 1 byte  */
    uint16_t battery_mv;      /* offset 17, 2 bytes */
    uint32_t error_code;      /* offset 19, 4 bytes — MISALIGNED */
} sensor_reading_evolved_t;

_Static_assert(sizeof(sensor_reading_evolved_t) == 24, "evolved size");
_Static_assert(offsetof(sensor_reading_evolved_t, error_code) == 19, "error_code offset");

void write_error(sensor_reading_evolved_t *p, uint32_t v) { p->error_code = v; }

volatile sensor_reading_evolved_t g_evolved;
