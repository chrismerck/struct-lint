#!/usr/bin/env python3
"""
gen_svg.py — Generate struct layout SVG diagrams from DWARF debug info.

Reads ELF files, extracts struct member layout from DWARF, and produces
SVG diagrams for the blog post. All layout data comes from the binary;
nothing is hardcoded.

Usage:
    python3 gen_svg.py \
        --pack1 out/rv32/sensor_reading.o \
        --pa4 out/rv32/sensor_reading.o \
        --unpacked out/rv32/sensor_reading.o \
        --evolved out/rv32/sensor_reading_evolved.o \
        --outdir out/svg
"""

import argparse
import io
import os
import struct as struct_mod
import sys
import tempfile
from elftools.elf.elffile import ELFFile
from elftools.dwarf.die import DIE


# ── RISC-V relocation patching ──────────────────────────────────

R_RISCV_32 = 1


def _create_patched_elf(elf_path):
    """Create a temporary copy of the ELF with RISC-V debug relocations applied.

    pyelftools doesn't support RISC-V relocation types, so we manually
    apply R_RISCV_32 relocations to debug sections. This fixes string
    references (DW_FORM_strp) that would otherwise all resolve to offset 0.

    Returns path to the temporary patched file (caller must delete).
    Returns None if no patching was needed.
    """
    with open(elf_path, "rb") as f:
        data = bytearray(f.read())

    elf = ELFFile(io.BytesIO(bytes(data)))

    # Check if this is a RISC-V relocatable file needing patching
    if elf["e_machine"] != "EM_RISCV":
        return None

    symtab = elf.get_section_by_name(".symtab")
    if not symtab:
        return None

    patched = False
    for section in elf.iter_sections():
        if section["sh_type"] != "SHT_RELA":
            continue
        target_name = section.name.replace(".rela", "", 1)
        if not target_name.startswith(".debug"):
            continue

        target_sec = elf.get_section_by_name(target_name)
        if not target_sec:
            continue

        sec_offset = target_sec["sh_offset"]

        for reloc in section.iter_relocations():
            rtype = reloc["r_info_type"]
            if rtype != R_RISCV_32:
                continue

            sym_idx = reloc["r_info_sym"]
            addend = reloc["r_addend"]
            offset = reloc["r_offset"]

            sym = symtab.get_symbol(sym_idx)
            sym_value = sym["st_value"]

            value = (sym_value + addend) & 0xFFFFFFFF
            file_offset = sec_offset + offset
            if file_offset + 4 <= len(data):
                struct_mod.pack_into("<I", data, file_offset, value)
                patched = True

    if not patched:
        return None

    tmp = tempfile.NamedTemporaryFile(suffix=".o", delete=False)
    tmp.write(bytes(data))
    tmp.close()
    return tmp.name


# ── DWARF extraction ────────────────────────────────────────────

def extract_structs(elf_path):
    """Extract all struct definitions from DWARF debug info.

    Returns dict of {struct_name: {"size": N, "members": [{"name", "type", "offset", "size"}, ...]}}
    """
    # Patch RISC-V relocations if needed
    patched_path = _create_patched_elf(elf_path)
    actual_path = patched_path if patched_path else elf_path

    try:
        return _extract_structs_impl(actual_path)
    finally:
        if patched_path:
            os.unlink(patched_path)


def _extract_structs_impl(elf_path):
    structs = {}
    with open(elf_path, "rb") as f:
        elf = ELFFile(f)
        if not elf.has_dwarf_info():
            print(f"Warning: {elf_path} has no DWARF info", file=sys.stderr)
            return structs

        dwarf = elf.get_dwarf_info(relocate_dwarf_sections=False)

        for cu in dwarf.iter_CUs():
            # First pass: build type name/size maps
            type_names = {}
            type_sizes = {}
            for die in cu.iter_DIEs():
                if die.tag and "DW_AT_name" in die.attributes:
                    type_names[die.offset] = die.attributes["DW_AT_name"].value.decode("utf-8")
                if die.tag and "DW_AT_byte_size" in die.attributes:
                    type_sizes[die.offset] = die.attributes["DW_AT_byte_size"].value

            # Second pass: collect typedef mappings
            typedef_map = {}
            for die in cu.iter_DIEs():
                if die.tag == "DW_TAG_typedef" and "DW_AT_name" in die.attributes:
                    name = die.attributes["DW_AT_name"].value.decode("utf-8")
                    if "DW_AT_type" in die.attributes:
                        target = die.attributes["DW_AT_type"].value + cu.cu_offset
                        typedef_map[target] = name

            # Third pass: extract struct members
            for die in cu.iter_DIEs():
                if die.tag != "DW_TAG_structure_type":
                    continue
                if "DW_AT_byte_size" not in die.attributes:
                    continue

                struct_size = die.attributes["DW_AT_byte_size"].value

                # Resolve name (direct or via typedef)
                if "DW_AT_name" in die.attributes:
                    struct_name = die.attributes["DW_AT_name"].value.decode("utf-8")
                elif die.offset in typedef_map:
                    struct_name = typedef_map[die.offset]
                else:
                    continue

                members = []
                for child in die.iter_children():
                    if child.tag != "DW_TAG_member":
                        continue
                    m_name = "?"
                    if "DW_AT_name" in child.attributes:
                        m_name = child.attributes["DW_AT_name"].value.decode("utf-8")

                    m_offset = 0
                    if "DW_AT_data_member_location" in child.attributes:
                        loc = child.attributes["DW_AT_data_member_location"]
                        if hasattr(loc, "value"):
                            m_offset = loc.value if isinstance(loc.value, int) else 0

                    # Resolve member type name and size
                    m_type = "?"
                    m_size = 0
                    if "DW_AT_type" in child.attributes:
                        type_ref = child.attributes["DW_AT_type"].value + cu.cu_offset
                        m_type = resolve_type_name(type_ref, type_names, cu)
                        m_size = resolve_type_size(type_ref, type_sizes, cu)

                    members.append({
                        "name": m_name,
                        "type": m_type,
                        "offset": m_offset,
                        "size": m_size,
                    })

                structs[struct_name] = {"size": struct_size, "members": members}

    return structs


def resolve_type_name(offset, type_names, cu):
    """Follow DW_AT_type chain to find a name."""
    if offset in type_names:
        return type_names[offset]
    # Follow the chain
    for die in cu.iter_DIEs():
        if die.offset == offset and "DW_AT_type" in die.attributes:
            next_off = die.attributes["DW_AT_type"].value + cu.cu_offset
            return resolve_type_name(next_off, type_names, cu)
    return "?"


def resolve_type_size(offset, type_sizes, cu):
    """Follow DW_AT_type chain to find a size."""
    if offset in type_sizes:
        return type_sizes[offset]
    for die in cu.iter_DIEs():
        if die.offset == offset and "DW_AT_type" in die.attributes:
            next_off = die.attributes["DW_AT_type"].value + cu.cu_offset
            return resolve_type_size(next_off, type_sizes, cu)
    return 0


# ── SVG generation ──────────────────────────────────────────────

SCALE = 32          # pixels per byte
ROW_H = 40          # row height
LABEL_W = 180       # left label column width
PAD = 20            # padding around diagram

COLORS = {
    "field": "#4a90d9",
    "padding": "#e74c3c",
    "native": "#27ae60",
    "decomposed": "#e67e22",
    "misaligned_new": "#e74c3c",
}


def svg_header(width, height):
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
        f'font-family="monospace" font-size="12">\n'
        f'<style>\n'
        f'  .field {{ stroke: #333; stroke-width: 1; }}\n'
        f'  .pad {{ fill: {COLORS["padding"]}; opacity: 0.3; stroke: #c0392b; stroke-width: 1; stroke-dasharray: 4,2; }}\n'
        f'  .label {{ text-anchor: end; dominant-baseline: middle; fill: #333; }}\n'
        f'  .offset {{ text-anchor: middle; dominant-baseline: hanging; fill: #666; font-size: 10; }}\n'
        f'  .caption {{ font-size: 14; font-weight: bold; fill: #333; }}\n'
        f'</style>\n'
    )


def svg_footer():
    return "</svg>\n"


def render_struct_row(members, struct_size, y, max_bytes, label="", color_fn=None):
    """Render one struct as a row of colored byte-boxes.

    color_fn(member) -> color string, or None to use default.
    Returns SVG string.
    """
    svg = ""

    # Label on the left
    if label:
        svg += f'<text x="{LABEL_W - 10}" y="{y + ROW_H // 2}" class="label">{label}</text>\n'

    x0 = LABEL_W

    # Build byte map: which member (or padding) occupies each byte
    byte_map = [None] * struct_size
    for m in members:
        for b in range(m["size"]):
            if m["offset"] + b < struct_size:
                byte_map[m["offset"] + b] = m

    # Render fields as rectangles
    rendered = set()
    for m in members:
        if id(m) in rendered:
            continue
        rendered.add(id(m))
        x = x0 + m["offset"] * SCALE
        w = m["size"] * SCALE
        color = color_fn(m) if color_fn else COLORS["field"]
        svg += (
            f'<rect x="{x}" y="{y}" width="{w}" height="{ROW_H}" '
            f'fill="{color}" class="field" />\n'
        )
        # Field name inside
        tx = x + w / 2
        svg += (
            f'<text x="{tx}" y="{y + ROW_H // 2}" '
            f'text-anchor="middle" dominant-baseline="middle" fill="white" font-size="10">'
            f'{m["name"]}</text>\n'
        )

    # Render padding bytes
    for i, occupant in enumerate(byte_map):
        if occupant is None:
            x = x0 + i * SCALE
            svg += f'<rect x="{x}" y="{y}" width="{SCALE}" height="{ROW_H}" class="pad" />\n'

    # Offset markers below
    for m in members:
        x = x0 + m["offset"] * SCALE
        svg += f'<text x="{x}" y="{y + ROW_H + 3}" class="offset">{m["offset"]}</text>\n'
    # End marker
    svg += f'<text x="{x0 + struct_size * SCALE}" y="{y + ROW_H + 3}" class="offset">{struct_size}</text>\n'

    return svg


def generate_padding_waste_svg(unpacked, pack1, outpath):
    """SVG 1: Show unpacked struct with padding highlighted vs packed."""
    members_u = unpacked["members"]
    members_p = pack1["members"]
    max_bytes = max(unpacked["size"], pack1["size"])

    width = LABEL_W + max_bytes * SCALE + PAD * 2
    height = PAD + (ROW_H + 30) * 2 + 40

    svg = svg_header(width, height)
    svg += f'<text x="{PAD}" y="{PAD}" class="caption">Struct Layout: Unpacked vs Packed</text>\n'

    y1 = PAD + 25
    svg += render_struct_row(members_u, unpacked["size"], y1, max_bytes,
                             label=f'unpacked ({unpacked["size"]}B)')

    y2 = y1 + ROW_H + 30
    svg += render_struct_row(members_p, pack1["size"], y2, max_bytes,
                             label=f'pack(1) ({pack1["size"]}B)')

    svg += svg_footer()
    with open(outpath, "w") as f:
        f.write(svg)
    print(f"  wrote {outpath}")


def generate_field_access_svg(pa4, outpath):
    """SVG 2: Show packed+aligned(4) struct with native vs byte-decomposed."""
    members = pa4["members"]
    max_bytes = pa4["size"]
    max_align = 4  # 32-bit target

    def color_fn(m):
        if m["size"] <= 1:
            return COLORS["native"]  # byte access either way
        natural = min(m["size"], max_align)
        if m["offset"] % natural == 0:
            return COLORS["native"]
        return COLORS["decomposed"]

    width = LABEL_W + max_bytes * SCALE + PAD * 2
    height = PAD + ROW_H + 30 + 60  # extra space for legend

    svg = svg_header(width, height)
    svg += f'<text x="{PAD}" y="{PAD}" class="caption">Field Access: packed, aligned(4)</text>\n'

    y = PAD + 25
    svg += render_struct_row(members, pa4["size"], y, max_bytes,
                             label=f'pa4 ({pa4["size"]}B)', color_fn=color_fn)

    # Legend
    ly = y + ROW_H + 25
    svg += f'<rect x="{LABEL_W}" y="{ly}" width="14" height="14" fill="{COLORS["native"]}" />\n'
    svg += f'<text x="{LABEL_W + 20}" y="{ly + 10}" font-size="11" fill="#333">native access</text>\n'
    svg += f'<rect x="{LABEL_W + 140}" y="{ly}" width="14" height="14" fill="{COLORS["decomposed"]}" />\n'
    svg += f'<text x="{LABEL_W + 160}" y="{ly + 10}" font-size="11" fill="#333">byte-decomposed</text>\n'

    svg += svg_footer()
    with open(outpath, "w") as f:
        f.write(svg)
    print(f"  wrote {outpath}")


def generate_evolution_svg(pa4, evolved, outpath):
    """SVG 3: Before and after adding error_code."""
    members_before = pa4["members"]
    members_after = evolved["members"]
    max_bytes = max(pa4["size"], evolved["size"])
    max_align = 4

    def color_fn_before(m):
        if m["size"] <= 1:
            return COLORS["native"]
        natural = min(m["size"], max_align)
        return COLORS["native"] if m["offset"] % natural == 0 else COLORS["decomposed"]

    def color_fn_after(m):
        # Highlight error_code as the new misaligned field
        if m["name"] == "error_code":
            return COLORS["misaligned_new"]
        return color_fn_before(m)

    width = LABEL_W + max_bytes * SCALE + PAD * 2
    height = PAD + (ROW_H + 30) * 2 + 40

    svg = svg_header(width, height)
    svg += f'<text x="{PAD}" y="{PAD}" class="caption">Struct Evolution: Adding error_code</text>\n'

    y1 = PAD + 25
    svg += render_struct_row(members_before, pa4["size"], y1, max_bytes,
                             label=f'before ({pa4["size"]}B)', color_fn=color_fn_before)

    y2 = y1 + ROW_H + 30
    svg += render_struct_row(members_after, evolved["size"], y2, max_bytes,
                             label=f'after ({evolved["size"]}B)', color_fn=color_fn_after)

    svg += svg_footer()
    with open(outpath, "w") as f:
        f.write(svg)
    print(f"  wrote {outpath}")


def main():
    parser = argparse.ArgumentParser(description="Generate struct layout SVGs from DWARF")
    parser.add_argument("--pack1", required=True, help="ELF with pack(1) struct")
    parser.add_argument("--pa4", required=True, help="ELF with packed+aligned(4) struct")
    parser.add_argument("--unpacked", required=True, help="ELF with unpacked struct")
    parser.add_argument("--evolved", required=True, help="ELF with evolved struct")
    parser.add_argument("--outdir", required=True, help="Output directory for SVGs")
    args = parser.parse_args()

    os.makedirs(args.outdir, exist_ok=True)

    print("Extracting DWARF struct info...")
    pack1_structs = extract_structs(args.pack1)
    pa4_structs = extract_structs(args.pa4)
    unpacked_structs = extract_structs(args.unpacked)
    evolved_structs = extract_structs(args.evolved)

    # Find the structs we need by name pattern
    pack1 = next((v for k, v in pack1_structs.items() if "pack1" in k), None)
    pa4 = next((v for k, v in pa4_structs.items() if "pa4" in k), None)
    unpacked = next((v for k, v in unpacked_structs.items() if "unpacked" in k), None)
    evolved = next((v for k, v in evolved_structs.items() if "evolved" in k), None)

    if not all([pack1, pa4, unpacked, evolved]):
        missing = []
        if not pack1: missing.append("pack1")
        if not pa4: missing.append("pa4")
        if not unpacked: missing.append("unpacked")
        if not evolved: missing.append("evolved")
        print(f"ERROR: Could not find structs: {', '.join(missing)}", file=sys.stderr)
        print(f"  Found in pack1 ELF: {list(pack1_structs.keys())}", file=sys.stderr)
        print(f"  Found in pa4 ELF: {list(pa4_structs.keys())}", file=sys.stderr)
        print(f"  Found in unpacked ELF: {list(unpacked_structs.keys())}", file=sys.stderr)
        print(f"  Found in evolved ELF: {list(evolved_structs.keys())}", file=sys.stderr)
        sys.exit(1)

    print("Generating SVGs...")
    generate_padding_waste_svg(unpacked, pack1, os.path.join(args.outdir, "padding-waste.svg"))
    generate_field_access_svg(pa4, os.path.join(args.outdir, "field-access.svg"))
    generate_evolution_svg(pa4, evolved, os.path.join(args.outdir, "struct-evolution.svg"))
    print("Done.")


if __name__ == "__main__":
    main()
