use clap::Parser;
use gimli::{
    AttributeValue, DebuggingInformationEntry, EndianSlice, LittleEndian, Unit, UnitOffset,
};
use object::{Object, ObjectSection};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

type R = EndianSlice<'static, LittleEndian>;

#[derive(Debug, Clone)]
struct MemberInfo {
    name: String,
    type_name: String,
    offset: u64,
    size: u64,
    is_bitfield: bool,
}

#[derive(Debug, Clone)]
struct StructInfo {
    name: String,
    size: u64,
    members: Vec<MemberInfo>,
    decl_file: String,
    decl_line: u64,
}

#[derive(Debug)]
enum Issue {
    MisalignedMember {
        struct_name: String,
        member_name: String,
        type_name: String,
        member_size: u64,
        offset: u64,
        natural_alignment: u64,
        decl_file: String,
        decl_line: u64,
    },
    NotPacked {
        struct_name: String,
        padding_bytes: u64,
        pattern: String,
        decl_file: String,
        decl_line: u64,
    },
}

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

fn make_relative(path: &str) -> String {
    let cwd = env::current_dir().unwrap_or_default();
    let p = Path::new(path);
    p.strip_prefix(&cwd)
        .unwrap_or(p)
        .to_string_lossy()
        .to_string()
}

fn main() {
    let cli = Cli::parse();

    let path = cli.paths[0].to_str().unwrap();
    let data = fs::read(path).unwrap_or_else(|e| {
        eprintln!("Error reading {}: {}", path, e);
        std::process::exit(2);
    });
    // Leak the data so we get a 'static lifetime for gimli's zero-copy parsing
    let data: &'static [u8] = Box::leak(data.into_boxed_slice());

    let obj = object::File::parse(data).unwrap_or_else(|e| {
        eprintln!("Error parsing ELF {}: {}", path, e);
        std::process::exit(2);
    });

    let is_64bit = obj.is_64();
    let max_align: u64 = if is_64bit { 8 } else { 4 };
    eprintln!(
        "ELF: {}-bit, max natural alignment = {}",
        if is_64bit { 64 } else { 32 },
        max_align
    );

    let dwarf = gimli::Dwarf::load(|section_id| -> Result<R, gimli::Error> {
        let data = obj
            .section_by_name(section_id.name())
            .map(|s| s.data().unwrap_or(&[]))
            .unwrap_or(&[]);
        Ok(EndianSlice::new(data, LittleEndian))
    })
    .unwrap();

    let structs = extract_structs(&dwarf);
    eprintln!("Found {} structs", structs.len());

    let issues = analyze_structs(&structs, max_align);

    if issues.is_empty() {
        println!("No issues found in {} structs.", structs.len());
    } else {
        for issue in &issues {
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
        println!(
            "\n{} issues found in {} structs",
            issues.len(),
            structs.len()
        );
        std::process::exit(1);
    }
}

fn extract_structs(dwarf: &gimli::Dwarf<R>) -> Vec<StructInfo> {
    let mut structs = Vec::new();
    let mut units_iter = dwarf.units();

    while let Ok(Some(unit_header)) = units_iter.next() {
        let unit = match dwarf.unit(unit_header) {
            Ok(u) => u,
            Err(_) => continue,
        };

        let mut type_names: HashMap<UnitOffset<usize>, String> = HashMap::new();
        let mut type_sizes: HashMap<UnitOffset<usize>, u64> = HashMap::new();

        // First pass: collect type names and sizes
        let mut entries = unit.entries();
        while let Ok(Some(entry)) = entries.next_dfs() {
            let offset = entry.offset();
            if let Some(name) = get_name(dwarf, entry) {
                type_names.insert(offset, name);
            }
            if let Some(size) = get_byte_size(entry) {
                type_sizes.insert(offset, size);
            }
        }

        // Second pass: collect struct entries (named and unnamed) and typedefs
        let mut struct_entries_by_offset: HashMap<
            UnitOffset<usize>,
            (Option<String>, u64, String, u64),
        > = HashMap::new();
        let mut typedef_map: HashMap<UnitOffset<usize>, String> = HashMap::new();

        let mut entries = unit.entries();
        while let Ok(Some(entry)) = entries.next_dfs() {
            if entry.tag() == gimli::DW_TAG_typedef {
                let typedef_name = match get_name(dwarf, entry) {
                    Some(n) => n,
                    None => continue,
                };
                if let Some(AttributeValue::UnitRef(target)) =
                    entry.attr_value(gimli::DW_AT_type)
                {
                    typedef_map.insert(target, typedef_name);
                }
                continue;
            }

            if entry.tag() != gimli::DW_TAG_structure_type {
                continue;
            }

            let struct_name = get_name(dwarf, entry);
            let struct_size = match get_byte_size(entry) {
                Some(s) => s,
                None => continue,
            };
            let (decl_file, decl_line) = get_source_location(dwarf, &unit, entry);
            let offset = entry.offset();
            struct_entries_by_offset
                .insert(offset, (struct_name, struct_size, decl_file, decl_line));
        }

        // Third pass: extract members for each struct, resolving names from typedefs
        for (offset, (name_opt, struct_size, decl_file, decl_line)) in &struct_entries_by_offset {
            let struct_name = match name_opt {
                Some(n) => n.clone(),
                None => match typedef_map.get(offset) {
                    Some(n) => n.clone(),
                    None => continue,
                },
            };

            let mut cursor = match unit.entries_at_offset(*offset) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let entry = match cursor.next_dfs() {
                Ok(Some(e)) => e,
                _ => continue,
            };

            let members = extract_members(dwarf, &unit, entry, &type_names, &type_sizes);
            if members.is_empty() {
                continue;
            }

            structs.push(StructInfo {
                name: struct_name,
                size: *struct_size,
                members,
                decl_file: decl_file.clone(),
                decl_line: *decl_line,
            });
        }
    }

    // Deduplicate by name + member layout
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut deduped = Vec::new();
    for s in structs {
        let key = format!(
            "{}:{}",
            s.name,
            s.members
                .iter()
                .map(|m| format!("{}@{}", m.name, m.offset))
                .collect::<Vec<_>>()
                .join(",")
        );
        if seen.contains_key(&key) {
            continue;
        }
        seen.insert(key, deduped.len());
        deduped.push(s);
    }
    deduped
}

fn extract_members(
    dwarf: &gimli::Dwarf<R>,
    unit: &Unit<R, usize>,
    struct_entry: &DebuggingInformationEntry<R, usize>,
    type_names: &HashMap<UnitOffset<usize>, String>,
    type_sizes: &HashMap<UnitOffset<usize>, u64>,
) -> Vec<MemberInfo> {
    let mut members = Vec::new();

    let mut tree = match unit.entries_tree(Some(struct_entry.offset())) {
        Ok(t) => t,
        Err(_) => return members,
    };

    let root = match tree.root() {
        Ok(r) => r,
        Err(_) => return members,
    };

    let mut children = root.children();
    while let Ok(Some(child)) = children.next() {
        let entry = child.entry();
        if entry.tag() != gimli::DW_TAG_member {
            continue;
        }

        let name = get_name(dwarf, entry).unwrap_or_else(|| "<anon>".to_string());

        // Get member offset (DW_AT_data_member_location)
        let offset = match entry.attr_value(gimli::DW_AT_data_member_location) {
            Some(AttributeValue::Udata(v)) => v,
            Some(AttributeValue::Sdata(v)) => v as u64,
            _ => continue,
        };

        // Check for bitfield
        let is_bitfield = entry
            .attr_value(gimli::DW_AT_bit_size)
            .is_some()
            || entry
                .attr_value(gimli::DW_AT_bit_offset)
                .is_some();

        // Resolve type
        let (type_name, type_size) = match entry.attr_value(gimli::DW_AT_type) {
            Some(AttributeValue::UnitRef(type_off)) => {
                let tn = resolve_type_name(type_off, type_names, unit, dwarf);
                let ts = resolve_type_size(type_off, type_sizes, unit);
                (tn, ts)
            }
            _ => ("?".to_string(), None),
        };

        members.push(MemberInfo {
            name,
            type_name,
            offset,
            size: type_size.unwrap_or(0),
            is_bitfield,
        });
    }

    members
}

fn resolve_type_name(
    offset: UnitOffset<usize>,
    type_names: &HashMap<UnitOffset<usize>, String>,
    unit: &Unit<R, usize>,
    dwarf: &gimli::Dwarf<R>,
) -> String {
    if let Some(name) = type_names.get(&offset) {
        return name.clone();
    }

    // Follow DW_AT_type chain (for typedefs, const, volatile, etc.)
    let mut cursor = unit.entries_at_offset(offset).ok();
    if let Some(ref mut c) = cursor {
        if let Ok(Some(entry)) = c.next_dfs() {
            if let Some(AttributeValue::UnitRef(next)) = entry.attr_value(gimli::DW_AT_type) {
                return resolve_type_name(next, type_names, unit, dwarf);
            }
            if let Some(name) = get_name(dwarf, entry) {
                return name;
            }
        }
    }
    "?".to_string()
}

fn resolve_type_size(
    offset: UnitOffset<usize>,
    type_sizes: &HashMap<UnitOffset<usize>, u64>,
    unit: &Unit<R, usize>,
) -> Option<u64> {
    if let Some(&size) = type_sizes.get(&offset) {
        return Some(size);
    }

    // Follow DW_AT_type chain
    let mut cursor = unit.entries_at_offset(offset).ok();
    if let Some(ref mut c) = cursor {
        if let Ok(Some(entry)) = c.next_dfs() {
            if let Some(AttributeValue::UnitRef(next)) = entry.attr_value(gimli::DW_AT_type) {
                return resolve_type_size(next, type_sizes, unit);
            }
        }
    }
    None
}

fn get_name(dwarf: &gimli::Dwarf<R>, entry: &DebuggingInformationEntry<R, usize>) -> Option<String> {
    let attr = entry.attr_value(gimli::DW_AT_name)?;
    match attr {
        AttributeValue::DebugStrRef(offset) => {
            let s = dwarf.debug_str.get_str(offset).ok()?;
            Some(s.to_string().ok()?.to_string())
        }
        AttributeValue::String(s) => Some(s.to_string().ok()?.to_string()),
        _ => None,
    }
}

fn get_byte_size(entry: &DebuggingInformationEntry<R, usize>) -> Option<u64> {
    match entry.attr_value(gimli::DW_AT_byte_size)? {
        AttributeValue::Udata(v) => Some(v),
        AttributeValue::Sdata(v) => Some(v as u64),
        AttributeValue::Data1(v) => Some(v as u64),
        AttributeValue::Data2(v) => Some(v as u64),
        AttributeValue::Data4(v) => Some(v as u64),
        AttributeValue::Data8(v) => Some(v),
        _ => None,
    }
}

fn get_source_location(
    dwarf: &gimli::Dwarf<R>,
    unit: &Unit<R, usize>,
    entry: &DebuggingInformationEntry<R, usize>,
) -> (String, u64) {
    let file_idx = match entry.attr_value(gimli::DW_AT_decl_file) {
        Some(AttributeValue::FileIndex(idx)) => idx,
        Some(AttributeValue::Udata(idx)) => idx,
        Some(AttributeValue::Data1(idx)) => idx as u64,
        Some(AttributeValue::Data2(idx)) => idx as u64,
        _ => return ("<unknown>".to_string(), 0),
    };

    let line = match entry.attr_value(gimli::DW_AT_decl_line) {
        Some(AttributeValue::Udata(l)) => l,
        Some(AttributeValue::Data1(l)) => l as u64,
        Some(AttributeValue::Data2(l)) => l as u64,
        _ => 0,
    };

    let file_name =
        get_file_name(dwarf, unit, file_idx).unwrap_or_else(|| "<unknown>".to_string());
    (file_name, line)
}

fn get_file_name(
    dwarf: &gimli::Dwarf<R>,
    unit: &Unit<R, usize>,
    file_idx: u64,
) -> Option<String> {
    let line_program = unit.line_program.as_ref()?.clone();
    let header = line_program.header();

    let file = header.file(file_idx)?;
    let dir = if let Some(dir_attr) = file.directory(header) {
        dwarf
            .attr_string(unit, dir_attr)
            .ok()?
            .to_string()
            .ok()?
            .to_string()
    } else {
        String::new()
    };

    let file_name = dwarf
        .attr_string(unit, file.path_name())
        .ok()?
        .to_string()
        .ok()?
        .to_string();

    if dir.is_empty() {
        Some(file_name)
    } else {
        Some(format!("{}/{}", dir, file_name))
    }
}

fn analyze_structs(structs: &[StructInfo], max_align: u64) -> Vec<Issue> {
    let mut issues = Vec::new();
    let pack_pattern = regex::Regex::new(r"_(rec|pkt(_\w+)?|header)_t$").ok();

    for s in structs {
        let is_packed = infer_packed(s, max_align);

        if is_packed {
            for m in &s.members {
                if m.is_bitfield || m.size == 0 || m.size == 1 {
                    continue;
                }
                let natural_align = std::cmp::min(m.size, max_align);
                if m.offset % natural_align != 0 {
                    issues.push(Issue::MisalignedMember {
                        struct_name: s.name.clone(),
                        member_name: m.name.clone(),
                        type_name: m.type_name.clone(),
                        member_size: m.size,
                        offset: m.offset,
                        natural_alignment: natural_align,
                        decl_file: s.decl_file.clone(),
                        decl_line: s.decl_line,
                    });
                }
            }
        } else if let Some(ref pat) = pack_pattern {
            if pat.is_match(&s.name) {
                let sum_sizes: u64 = s.members.iter().map(|m| m.size).sum();
                let padding = s.size.saturating_sub(sum_sizes);
                if padding > 0 {
                    issues.push(Issue::NotPacked {
                        struct_name: s.name.clone(),
                        padding_bytes: padding,
                        pattern: pat.to_string(),
                        decl_file: s.decl_file.clone(),
                        decl_line: s.decl_line,
                    });
                }
            }
        }
    }
    issues
}

fn infer_packed(s: &StructInfo, max_align: u64) -> bool {
    let mut has_alignment_violation = false;
    let sum_sizes: u64 = s.members.iter().map(|m| m.size).sum();

    for m in &s.members {
        if m.is_bitfield || m.size <= 1 {
            continue;
        }
        let natural_align = std::cmp::min(m.size, max_align);
        if m.offset % natural_align != 0 {
            has_alignment_violation = true;
            break;
        }
    }

    if has_alignment_violation && s.size == sum_sizes {
        return true;
    }

    if s.size == sum_sizes && sum_sizes > 0 {
        let mut expected_offset: u64 = 0;
        let mut would_need_padding = false;
        for m in &s.members {
            if m.size > 1 {
                let natural_align = std::cmp::min(m.size, max_align);
                let aligned_offset = (expected_offset + natural_align - 1) & !(natural_align - 1);
                if aligned_offset != expected_offset {
                    would_need_padding = true;
                }
            }
            expected_offset += m.size;
        }
        if !would_need_padding && expected_offset > 0 {
            let max_member_align = s
                .members
                .iter()
                .map(|m| {
                    if m.size > 1 {
                        std::cmp::min(m.size, max_align)
                    } else {
                        1
                    }
                })
                .max()
                .unwrap_or(1);
            let padded_size = (expected_offset + max_member_align - 1) & !(max_member_align - 1);
            if padded_size != s.size {
                would_need_padding = true;
            }
        }
        if would_need_padding {
            return true;
        }
    }

    false
}
