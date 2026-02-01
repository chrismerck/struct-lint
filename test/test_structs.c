// Test structs for struct-lint POC

#include <stdint.h>

// Packed struct with misaligned members (should trigger alignment warning)
#pragma pack(push, 1)
typedef struct {
    uint8_t  type;       // offset 0, size 1
    uint16_t seq;        // offset 1, size 2 -- misaligned! (needs 2)
    uint8_t  flags;      // offset 3, size 1
    uint32_t payload_len; // offset 4, size 4 -- OK on 4-byte boundary
    uint8_t  version;    // offset 8, size 1
    uint32_t crc;        // offset 9, size 4 -- misaligned! (needs 4)
} sync_pkt_t;
#pragma pack(pop)

// Packed struct with all members naturally aligned (no warnings expected)
#pragma pack(push, 1)
typedef struct {
    uint32_t id;         // offset 0, size 4
    uint16_t type;       // offset 4, size 2
    uint8_t  flags;      // offset 6, size 1
    uint8_t  pad;        // offset 7, size 1
} well_aligned_pkt_t;
#pragma pack(pop)

// NOT packed but name matches pattern -- should trigger "should be packed" warning
typedef struct {
    uint8_t  type;       // offset 0, size 1
    // compiler inserts 3 bytes padding here
    uint32_t value;      // offset 4, size 4
    uint8_t  flags;      // offset 8, size 1
    // compiler inserts 3 bytes trailing padding
} sensor_rec_t;

// Regular struct, not packed, name does NOT match pattern -- no warning
typedef struct {
    int x;
    int y;
    int z;
} point_t;

// Force symbols so structs appear in DWARF
volatile sync_pkt_t         g_pkt;
volatile well_aligned_pkt_t g_aligned;
volatile sensor_rec_t       g_rec;
volatile point_t            g_point;
