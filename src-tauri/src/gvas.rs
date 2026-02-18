//! GVAS (Unreal Engine Game Version Archive Save) parser / writer.
//!
//! Converts between the binary `.sav` format used by Palworld and a
//! `serde_json::Value` representation that mirrors the structure produced by
//! PalworldSaveTools.
//!
//! The outer `.sav` container supports three compression schemes:
//!   - 0x32 / "PlZ" – double-zlib
//!   - 0x31 / "PlM" – Oodle (Mermaid) via the game's `oo2core` DLL
//!   - 0x30 / "CNK" – single-zlib with a 24-byte header (wrapper)
//!
//! Inside the decompressed data is the GVAS binary stream.

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::io::{self, Cursor, Read, Write};
use std::sync::LazyLock;

use crate::oodle;

/// Static empty vec used as default for `.unwrap_or_else(|| &EMPTY_VEC)` patterns.
static EMPTY_VEC: LazyLock<Vec<Value>> = LazyLock::new(Vec::new);

// ── SAV container ────────────────────────────────────────

/// Decompress a `.sav` file into raw GVAS bytes.
/// Returns `(gvas_bytes, save_type)`.
///
/// Supported formats:
///   - `0x32` / magic "PlZ" – double-zlib
///   - `0x31` / magic "PlM" – Oodle (requires `oo2core` DLL from Palworld)
///   - `0x30` / magic "CNK" – wrapper; re-reads inner header then decompresses
pub fn decompress_sav(data: &[u8]) -> Result<(Vec<u8>, u8), String> {
    if data.len() < 12 {
        return Err("SAV file too small".into());
    }
    let mut cur = Cursor::new(data);
    let mut uncompressed_len = cur.read_u32::<LittleEndian>().map_err(|e| e.to_string())? as usize;
    let mut compressed_len = cur.read_u32::<LittleEndian>().map_err(|e| e.to_string())? as usize;
    let mut magic = [0u8; 3];
    cur.read_exact(&mut magic).map_err(|e| e.to_string())?;
    let mut save_type = cur.read_u8().map_err(|e| e.to_string())?;

    let mut data_offset: usize = 12;

    // CNK wrapper: re-read inner header
    if &magic == b"CNK" {
        if data.len() < 24 {
            return Err("CNK file too small for inner header".into());
        }
        uncompressed_len = cur.read_u32::<LittleEndian>().map_err(|e| e.to_string())? as usize;
        compressed_len = cur.read_u32::<LittleEndian>().map_err(|e| e.to_string())? as usize;
        cur.read_exact(&mut magic).map_err(|e| e.to_string())?;
        save_type = cur.read_u8().map_err(|e| e.to_string())?;
        data_offset = 24;
    }

    let payload = &data[data_offset..];

    match save_type {
        0x32 => {
            // Double-zlib (PlZ type 50)
            let mut first = Vec::with_capacity(compressed_len);
            ZlibDecoder::new(payload)
                .read_to_end(&mut first)
                .map_err(|e| format!("zlib pass-1 decompress: {e}"))?;
            let mut gvas = Vec::with_capacity(uncompressed_len);
            ZlibDecoder::new(&first[..])
                .read_to_end(&mut gvas)
                .map_err(|e| format!("zlib pass-2 decompress: {e}"))?;
            Ok((gvas, save_type))
        }
        0x31 => {
            // Oodle / Mermaid (PlM type 49)
            let compressed_data = if compressed_len > 0 && compressed_len <= payload.len() {
                &payload[..compressed_len]
            } else {
                payload
            };
            let gvas = oodle::decompress(compressed_data, uncompressed_len)?;
            Ok((gvas, save_type))
        }
        0x30 => {
            // Single-zlib (CNK inner or standalone type 48)
            let mut gvas = Vec::with_capacity(uncompressed_len);
            ZlibDecoder::new(payload)
                .read_to_end(&mut gvas)
                .map_err(|e| format!("zlib decompress: {e}"))?;
            Ok((gvas, save_type))
        }
        _ => Err(format!("Unsupported save_type 0x{save_type:02X}")),
    }
}

/// Compress raw GVAS bytes back into `.sav` format.
///
/// **PLM (0x31) is automatically converted to PLZ (0x32)**, because
/// Oodle compression requires the proprietary SDK.  Palworld reads PLZ
/// files regardless of the original format.
pub fn compress_sav(gvas: &[u8], save_type: u8) -> Result<Vec<u8>, String> {
    // PLM → PLZ: we can decompress Oodle via the game DLL, but we cannot
    // recompress without the Oodle SDK.  PalworldSaveTools does the same.
    let effective = if save_type == 0x31 { 0x32 } else { save_type };

    match effective {
        0x32 => {
            // Double-zlib (PlZ type 50)
            let mut enc1 = ZlibEncoder::new(Vec::new(), Compression::default());
            enc1.write_all(gvas).map_err(|e| e.to_string())?;
            let compressed_once = enc1.finish().map_err(|e| e.to_string())?;
            let compressed_len = compressed_once.len() as u32;
            let mut enc2 = ZlibEncoder::new(Vec::new(), Compression::default());
            enc2.write_all(&compressed_once).map_err(|e| e.to_string())?;
            let compressed_twice = enc2.finish().map_err(|e| e.to_string())?;
            let mut out = Vec::with_capacity(12 + compressed_twice.len());
            out.write_u32::<LittleEndian>(gvas.len() as u32)
                .map_err(|e| e.to_string())?;
            out.write_u32::<LittleEndian>(compressed_len)
                .map_err(|e| e.to_string())?;
            out.extend_from_slice(b"PlZ");
            out.push(0x32);
            out.extend_from_slice(&compressed_twice);
            Ok(out)
        }
        0x30 => {
            // Single-zlib (CNK / type 48)
            let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
            enc.write_all(gvas).map_err(|e| e.to_string())?;
            let compressed = enc.finish().map_err(|e| e.to_string())?;
            let mut out = Vec::with_capacity(12 + compressed.len());
            out.write_u32::<LittleEndian>(gvas.len() as u32)
                .map_err(|e| e.to_string())?;
            out.write_u32::<LittleEndian>(compressed.len() as u32)
                .map_err(|e| e.to_string())?;
            out.extend_from_slice(b"PlZ");
            out.push(0x30);
            out.extend_from_slice(&compressed);
            Ok(out)
        }
        _ => Err(format!("Unsupported save_type 0x{effective:02X}")),
    }
}

// ── UUID helpers ─────────────────────────────────────────

/// Read 16 bytes as a UUID string with Unreal's byte-swizzle convention.
fn read_uuid(cur: &mut Cursor<&[u8]>) -> io::Result<String> {
    let mut raw = [0u8; 16];
    cur.read_exact(&mut raw)?;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        raw[3], raw[2], raw[1], raw[0],
        raw[7], raw[6],
        raw[5], raw[4],
        raw[11], raw[10],
        raw[9], raw[8],
        raw[15], raw[14], raw[13], raw[12],
    ))
}

fn write_uuid(w: &mut Vec<u8>, s: &str) -> Result<(), String> {
    let hex: String = s.replace('-', "");
    if hex.len() != 32 {
        return Err(format!("Invalid UUID: {s}"));
    }
    let bytes: Vec<u8> = (0..32)
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0))
        .collect();
    // Swizzle back to Unreal order
    w.extend_from_slice(&[bytes[3], bytes[2], bytes[1], bytes[0]]);
    w.extend_from_slice(&[bytes[7], bytes[6]]);
    w.extend_from_slice(&[bytes[5], bytes[4]]);
    w.extend_from_slice(&[bytes[11], bytes[10]]);
    w.extend_from_slice(&[bytes[9], bytes[8]]);
    w.extend_from_slice(&[bytes[15], bytes[14], bytes[13], bytes[12]]);
    Ok(())
}

// ── FString helpers ──────────────────────────────────────

fn read_fstring(cur: &mut Cursor<&[u8]>) -> io::Result<String> {
    let size = cur.read_i32::<LittleEndian>()?;
    if size == 0 {
        return Ok(String::new());
    }
    if size < 0 {
        // UTF-16-LE
        let count = (-size) as usize;
        let mut buf = vec![0u8; count * 2];
        cur.read_exact(&mut buf)?;
        // Strip null terminator (last 2 bytes)
        let chars: Vec<u16> = buf[..buf.len() - 2]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        Ok(String::from_utf16_lossy(&chars))
    } else {
        let count = size as usize;
        let mut buf = vec![0u8; count];
        cur.read_exact(&mut buf)?;
        // Strip null terminator
        if let Some(last) = buf.last() {
            if *last == 0 {
                buf.pop();
            }
        }
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }
}

fn write_fstring(w: &mut Vec<u8>, s: &str) -> Result<(), String> {
    if s.is_empty() {
        w.write_i32::<LittleEndian>(0).map_err(|e| e.to_string())?;
        return Ok(());
    }
    if s.is_ascii() {
        let len = (s.len() + 1) as i32; // +1 for null terminator
        w.write_i32::<LittleEndian>(len)
            .map_err(|e| e.to_string())?;
        w.extend_from_slice(s.as_bytes());
        w.push(0);
    } else {
        let utf16: Vec<u16> = s.encode_utf16().collect();
        let len = -((utf16.len() + 1) as i32); // negative for UTF-16
        w.write_i32::<LittleEndian>(len)
            .map_err(|e| e.to_string())?;
        for ch in &utf16 {
            w.write_u16::<LittleEndian>(*ch)
                .map_err(|e| e.to_string())?;
        }
        w.extend_from_slice(&[0, 0]); // null terminator
    }
    Ok(())
}

// ── Optional GUID ────────────────────────────────────────

fn read_optional_uuid(cur: &mut Cursor<&[u8]>) -> io::Result<Value> {
    let flag = cur.read_u8()?;
    if flag != 0 {
        let uuid = read_uuid(cur)?;
        Ok(Value::String(uuid))
    } else {
        Ok(Value::Null)
    }
}

fn write_optional_uuid(w: &mut Vec<u8>, v: &Value) -> Result<(), String> {
    match v {
        Value::Null => {
            w.push(0);
            Ok(())
        }
        Value::String(s) => {
            w.push(1);
            write_uuid(w, s)
        }
        _ => {
            w.push(0);
            Ok(())
        }
    }
}

// ── Known paths that should use skip-decode (raw passthrough) ──

fn is_skip_path(path: &str) -> bool {
    // We only need CharacterSaveParameterMap and GroupSaveDataMap for player
    // extraction.  Everything else inside worldSaveData is skipped as raw bytes
    // to avoid parsing structures we don't have full type hints for.
    let skip_patterns = [
        // Large blob properties
        "FoliageGridSaveDataMap",
        "MapObjectSpawnerInStageSaveData",
        "WorldLocation",
        "WorldRotation",
        "WorldScale3D",
        "EffectMap",
        // All other worldSaveData children we don't need
        "ItemContainerSaveData",
        "CharacterContainerSaveData",
        "DynamicItemSaveData",
        "MapObjectSaveData",
        "WorkSaveData",
        "BaseCampSaveData",
        "EnemyCampSaveData",
        "DungeonSaveData",
        "DungeonPointMarkerSaveData",
        "OilrigSaveData",
        "InvaderSaveData",
        "GameTimeSaveData",
        "WorkerDirectorSaveData",
        "GuildExtraSaveDataMap",
        "CharacterParameterStorageSaveData",
        "SupplySaveData",
        "InLockerCharacterInstanceIDArray",
    ];
    for pat in &skip_patterns {
        if path.ends_with(pat) {
            return true;
        }
    }
    false
}

// ── Palworld-specific type hints for MapProperty key/value struct types ──

fn type_hint_for(path: &str) -> Option<&'static str> {
    // Key/value struct types for known MapProperty paths.
    // "" = generic struct (read properties until None)
    // "Guid" = read 16-byte Unreal GUID
    //
    // These hints were derived from PalworldSaveTools JSON output for a real
    // Level.sav.  When the key/value is StructProperty but the inner struct is
    // a plain Guid, specify "Guid"; otherwise "" means "generic property bag".
    match path {
        // CharacterSaveParameterMap: key=struct{PlayerUId,InstanceId}, value=struct{RawData}
        p if p.ends_with(".CharacterSaveParameterMap.Key") => Some(""),
        p if p.ends_with(".CharacterSaveParameterMap.Value") => Some(""),
        // GroupSaveDataMap: key=Guid, value=struct{GroupType,RawData,...}
        p if p.ends_with(".GroupSaveDataMap.Key") => Some("Guid"),
        p if p.ends_with(".GroupSaveDataMap.Value") => Some(""),
        // GuildExtraSaveDataMap: key=Guid
        p if p.ends_with(".GuildExtraSaveDataMap.Key") => Some("Guid"),
        p if p.ends_with(".GuildExtraSaveDataMap.Value") => Some(""),
        // SupplyInfos: key=Guid, value=struct
        p if p.ends_with(".SupplyInfos.Key") => Some("Guid"),
        p if p.ends_with(".SupplyInfos.Value") => Some(""),
        // RewardSaveDataMap: key=Guid
        p if p.ends_with(".RewardSaveDataMap.Key") => Some("Guid"),
        p if p.ends_with(".RewardSaveDataMap.Value") => Some(""),
        // SpawnerDataMapByLevelObjectInstanceId: key=Guid
        p if p.ends_with(".SpawnerDataMapByLevelObjectInstanceId.Key") => Some("Guid"),
        p if p.ends_with(".SpawnerDataMapByLevelObjectInstanceId.Value") => Some(""),
        // BaseCampSaveData: key=Guid
        p if p.ends_with(".BaseCampSaveData.Key") => Some("Guid"),
        p if p.ends_with(".BaseCampSaveData.Value") => Some(""),
        // InvaderSaveData: key=Guid
        p if p.ends_with(".InvaderSaveData.Key") => Some("Guid"),
        p if p.ends_with(".InvaderSaveData.Value") => Some(""),
        // Generic struct maps (key=struct property bag)
        p if p.ends_with(".ItemContainerSaveData.Key") => Some(""),
        p if p.ends_with(".ItemContainerSaveData.Value") => Some(""),
        p if p.ends_with(".CharacterContainerSaveData.Key") => Some(""),
        p if p.ends_with(".CharacterContainerSaveData.Value") => Some(""),
        p if p.ends_with(".DynamicItemSaveData.Key") => Some(""),
        p if p.ends_with(".DynamicItemSaveData.Value") => Some(""),
        p if p.ends_with(".FoliageGridSaveDataMap.Key") => Some(""),
        p if p.ends_with(".FoliageGridSaveDataMap.Value") => Some(""),
        p if p.ends_with(".MapObjectSpawnerInStageSaveData.Key") => Some(""),
        p if p.ends_with(".MapObjectSpawnerInStageSaveData.Value") => Some(""),
        p if p.ends_with(".InstanceDataMap.Key") => Some(""),
        p if p.ends_with(".InstanceDataMap.Value") => Some(""),
        // Catch-all for any map ending in "SaveData" or "Map"
        p if p.ends_with("SaveData.Key") => Some(""),
        p if p.ends_with("SaveData.Value") => Some(""),
        p if p.ends_with("Map.Key") => Some(""),
        p if p.ends_with("Map.Value") => Some(""),
        _ => None,
    }
}

// ── Custom property paths that need rawdata decode ──

fn is_group_rawdata_path(path: &str) -> bool {
    path.ends_with(".GroupSaveDataMap")
}

fn is_character_rawdata_path(path: &str) -> bool {
    path.ends_with("CharacterSaveParameterMap.Value.RawData")
}

// ── GVAS reader ─────────────────────────────────────────

struct GvasReader<'a> {
    cur: Cursor<&'a [u8]>,
}

impl<'a> GvasReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            cur: Cursor::new(data),
        }
    }

    fn position(&self) -> u64 {
        self.cur.position()
    }

    fn read_header(&mut self) -> Result<Value, String> {
        let magic = self.cur.read_i32::<LittleEndian>().map_err(|e| e.to_string())?;
        if magic != 0x53415647 {
            return Err(format!("Bad GVAS magic: 0x{magic:08X}"));
        }
        let save_game_version = self.cur.read_i32::<LittleEndian>().map_err(|e| e.to_string())?;
        let pkg_ver_ue4 = self.cur.read_i32::<LittleEndian>().map_err(|e| e.to_string())?;
        let pkg_ver_ue5 = self.cur.read_i32::<LittleEndian>().map_err(|e| e.to_string())?;
        let ev_major = self.cur.read_u16::<LittleEndian>().map_err(|e| e.to_string())?;
        let ev_minor = self.cur.read_u16::<LittleEndian>().map_err(|e| e.to_string())?;
        let ev_patch = self.cur.read_u16::<LittleEndian>().map_err(|e| e.to_string())?;
        let ev_changelist = self.cur.read_u32::<LittleEndian>().map_err(|e| e.to_string())?;
        let ev_branch = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
        let cv_format = self.cur.read_i32::<LittleEndian>().map_err(|e| e.to_string())?;
        // Custom versions array
        let cv_count = self.cur.read_u32::<LittleEndian>().map_err(|e| e.to_string())?;
        let mut custom_versions = Vec::new();
        for _ in 0..cv_count {
            let guid = read_uuid(&mut self.cur).map_err(|e| e.to_string())?;
            let ver = self.cur.read_i32::<LittleEndian>().map_err(|e| e.to_string())?;
            custom_versions.push(json!([guid, ver]));
        }
        let save_game_class_name = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
        Ok(json!({
            "magic": magic,
            "save_game_version": save_game_version,
            "package_file_version_ue4": pkg_ver_ue4,
            "package_file_version_ue5": pkg_ver_ue5,
            "engine_version_major": ev_major,
            "engine_version_minor": ev_minor,
            "engine_version_patch": ev_patch,
            "engine_version_changelist": ev_changelist,
            "engine_version_branch": ev_branch,
            "custom_version_format": cv_format,
            "custom_versions": custom_versions,
            "save_game_class_name": save_game_class_name,
        }))
    }

    fn read_properties(&mut self, path: &str) -> Result<Map<String, Value>, String> {
        let mut props = Map::new();
        loop {
            let name = read_fstring(&mut self.cur).map_err(|e| format!("read prop name at {path}: {e}"))?;
            if name == "None" || name.is_empty() {
                break;
            }
            let type_name = read_fstring(&mut self.cur).map_err(|e| format!("read prop type for {path}.{name}: {e}"))?;
            let size = self.cur.read_u64::<LittleEndian>().map_err(|e| format!("read prop size for {path}.{name}: {e}"))? as usize;
            let prop_path = format!("{path}.{name}");
            let value = self.read_property(&type_name, size, &prop_path)
                .map_err(|e| format!("property {prop_path} ({type_name}, size={size}): {e}"))?;
            props.insert(name, value);
        }
        Ok(props)
    }

    fn read_property(&mut self, type_name: &str, size: usize, path: &str) -> Result<Value, String> {
        // Skip-decode for large blob properties
        if is_skip_path(path) {
            return self.read_skip_property(type_name, size, path);
        }

        // Custom decode for GroupSaveDataMap (reads as MapProperty then decodes group rawdata)
        if is_group_rawdata_path(path) {
            return self.read_group_map_property(size, path);
        }

        match type_name {
            "IntProperty" => self.read_int_property(),
            "UInt16Property" => self.read_uint16_property(),
            "UInt32Property" => self.read_uint32_property(),
            "UInt64Property" => self.read_uint64_property(),
            "Int64Property" => self.read_int64_property(),
            "FixedPoint64Property" => self.read_fixedpoint64_property(),
            "FloatProperty" => self.read_float_property(),
            "DoubleProperty" => self.read_double_property(),
            "StrProperty" => self.read_str_property(),
            "NameProperty" => self.read_name_property(),
            "TextProperty" => self.read_text_property(size),
            "BoolProperty" => self.read_bool_property(),
            "EnumProperty" => self.read_enum_property(),
            "ByteProperty" => self.read_byte_property(size),
            "StructProperty" => self.read_struct_property(size, path),
            "ArrayProperty" => self.read_array_property(size, path),
            "MapProperty" => self.read_map_property(size, path),
            "SetProperty" => self.read_set_property(size, path),
            "SoftObjectProperty" => self.read_soft_object_property(),
            "ObjectProperty" => self.read_object_property(),
            _ => {
                // Unknown type: skip bytes
                let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
                let mut raw = vec![0u8; size];
                self.cur.read_exact(&mut raw).map_err(|e| e.to_string())?;
                Ok(json!({
                    "id": id,
                    "value": base64_encode(&raw),
                    "type": type_name,
                    "custom_type": "unknown_skip"
                }))
            }
        }
    }

    fn read_skip_property(&mut self, type_name: &str, size: usize, _path: &str) -> Result<Value, String> {
        match type_name {
            "ArrayProperty" => {
                let array_type = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
                let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
                let mut raw = vec![0u8; size];
                self.cur.read_exact(&mut raw).map_err(|e| e.to_string())?;
                Ok(json!({
                    "skip_type": "ArrayProperty",
                    "array_type": array_type,
                    "id": id,
                    "value": base64_encode(&raw),
                    "type": "ArrayProperty"
                }))
            }
            "MapProperty" => {
                let key_type = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
                let value_type = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
                let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
                let mut raw = vec![0u8; size];
                self.cur.read_exact(&mut raw).map_err(|e| e.to_string())?;
                Ok(json!({
                    "skip_type": "MapProperty",
                    "key_type": key_type,
                    "value_type": value_type,
                    "id": id,
                    "value": base64_encode(&raw),
                    "type": "MapProperty"
                }))
            }
            "StructProperty" => {
                let struct_type = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
                let struct_id = read_uuid(&mut self.cur).map_err(|e| e.to_string())?;
                let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
                let mut raw = vec![0u8; size];
                self.cur.read_exact(&mut raw).map_err(|e| e.to_string())?;
                Ok(json!({
                    "skip_type": "StructProperty",
                    "struct_type": struct_type,
                    "struct_id": struct_id,
                    "id": id,
                    "value": base64_encode(&raw),
                    "type": "StructProperty"
                }))
            }
            "SetProperty" => {
                let set_type = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
                let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
                let mut raw = vec![0u8; size];
                self.cur.read_exact(&mut raw).map_err(|e| e.to_string())?;
                Ok(json!({
                    "skip_type": "SetProperty",
                    "set_type": set_type,
                    "id": id,
                    "value": base64_encode(&raw),
                    "type": "SetProperty"
                }))
            }
            _ => {
                // Generic skip: read header + raw body
                let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
                let mut raw = vec![0u8; size];
                self.cur.read_exact(&mut raw).map_err(|e| e.to_string())?;
                Ok(json!({
                    "skip_type": type_name,
                    "id": id,
                    "value": base64_encode(&raw),
                    "type": type_name
                }))
            }
        }
    }

    // ── Simple property types ──

    fn read_int_property(&mut self) -> Result<Value, String> {
        let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
        let v = self.cur.read_i32::<LittleEndian>().map_err(|e| e.to_string())?;
        Ok(json!({"id": id, "value": v, "type": "IntProperty"}))
    }

    fn read_uint16_property(&mut self) -> Result<Value, String> {
        let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
        let v = self.cur.read_u16::<LittleEndian>().map_err(|e| e.to_string())?;
        Ok(json!({"id": id, "value": v, "type": "UInt16Property"}))
    }

    fn read_uint32_property(&mut self) -> Result<Value, String> {
        let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
        let v = self.cur.read_u32::<LittleEndian>().map_err(|e| e.to_string())?;
        Ok(json!({"id": id, "value": v, "type": "UInt32Property"}))
    }

    fn read_uint64_property(&mut self) -> Result<Value, String> {
        let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
        let v = self.cur.read_u64::<LittleEndian>().map_err(|e| e.to_string())?;
        Ok(json!({"id": id, "value": v, "type": "UInt64Property"}))
    }

    fn read_int64_property(&mut self) -> Result<Value, String> {
        let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
        let v = self.cur.read_i64::<LittleEndian>().map_err(|e| e.to_string())?;
        Ok(json!({"id": id, "value": v, "type": "Int64Property"}))
    }

    fn read_fixedpoint64_property(&mut self) -> Result<Value, String> {
        let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
        let v = self.cur.read_i32::<LittleEndian>().map_err(|e| e.to_string())?;
        Ok(json!({"id": id, "value": v, "type": "FixedPoint64Property"}))
    }

    fn read_float_property(&mut self) -> Result<Value, String> {
        let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
        let v = self.cur.read_f32::<LittleEndian>().map_err(|e| e.to_string())?;
        Ok(json!({"id": id, "value": v, "type": "FloatProperty"}))
    }

    fn read_double_property(&mut self) -> Result<Value, String> {
        let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
        let v = self.cur.read_f64::<LittleEndian>().map_err(|e| e.to_string())?;
        Ok(json!({"id": id, "value": v, "type": "DoubleProperty"}))
    }

    fn read_str_property(&mut self) -> Result<Value, String> {
        let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
        let v = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
        Ok(json!({"id": id, "value": v, "type": "StrProperty"}))
    }

    fn read_name_property(&mut self) -> Result<Value, String> {
        let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
        let v = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
        Ok(json!({"id": id, "value": v, "type": "NameProperty"}))
    }

    fn read_text_property(&mut self, size: usize) -> Result<Value, String> {
        let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
        // TextProperty is complex; store as raw bytes
        let mut raw = vec![0u8; size];
        self.cur.read_exact(&mut raw).map_err(|e| e.to_string())?;
        Ok(json!({"id": id, "value": base64_encode(&raw), "type": "TextProperty", "custom_type": "raw_text"}))
    }

    fn read_bool_property(&mut self) -> Result<Value, String> {
        // BoolProperty: value byte BEFORE optional_guid (unique among all types)
        let v = self.cur.read_u8().map_err(|e| e.to_string())?;
        let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
        Ok(json!({"id": id, "value": v != 0, "type": "BoolProperty"}))
    }

    fn read_enum_property(&mut self) -> Result<Value, String> {
        let enum_type = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
        let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
        let enum_value = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
        Ok(json!({
            "id": id,
            "value": {"type": enum_type, "value": enum_value},
            "type": "EnumProperty"
        }))
    }

    fn read_byte_property(&mut self, _size: usize) -> Result<Value, String> {
        let enum_type = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
        let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
        if enum_type == "None" {
            let v = self.cur.read_u8().map_err(|e| e.to_string())?;
            Ok(json!({
                "id": id,
                "value": {"type": enum_type, "value": v},
                "type": "ByteProperty"
            }))
        } else {
            let v = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
            Ok(json!({
                "id": id,
                "value": {"type": enum_type, "value": v},
                "type": "ByteProperty"
            }))
        }
    }

    fn read_soft_object_property(&mut self) -> Result<Value, String> {
        let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
        let v = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
        let sub_path = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
        Ok(json!({"id": id, "value": {"path": v, "sub_path": sub_path}, "type": "SoftObjectProperty"}))
    }

    fn read_object_property(&mut self) -> Result<Value, String> {
        let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
        let v = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
        Ok(json!({"id": id, "value": v, "type": "ObjectProperty"}))
    }

    // ── Struct property ──

    fn read_struct_property(&mut self, size: usize, path: &str) -> Result<Value, String> {
        let struct_type = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
        let struct_id = read_uuid(&mut self.cur).map_err(|e| e.to_string())?;
        let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
        let value = self.read_struct_value(&struct_type, size, path)?;
        Ok(json!({
            "struct_type": struct_type,
            "struct_id": struct_id,
            "id": id,
            "value": value,
            "type": "StructProperty"
        }))
    }

    fn read_struct_value(&mut self, struct_type: &str, _size: usize, path: &str) -> Result<Value, String> {
        match struct_type {
            "Vector" | "Rotator" => {
                let x = self.cur.read_f64::<LittleEndian>().map_err(|e| format!("{struct_type} x at {path}: {e}"))?;
                let y = self.cur.read_f64::<LittleEndian>().map_err(|e| format!("{struct_type} y at {path}: {e}"))?;
                let z = self.cur.read_f64::<LittleEndian>().map_err(|e| format!("{struct_type} z at {path}: {e}"))?;
                Ok(json!({"x": x, "y": y, "z": z}))
            }
            "Quat" => {
                let x = self.cur.read_f64::<LittleEndian>().map_err(|e| e.to_string())?;
                let y = self.cur.read_f64::<LittleEndian>().map_err(|e| e.to_string())?;
                let z = self.cur.read_f64::<LittleEndian>().map_err(|e| e.to_string())?;
                let w = self.cur.read_f64::<LittleEndian>().map_err(|e| e.to_string())?;
                Ok(json!({"x": x, "y": y, "z": z, "w": w}))
            }
            "DateTime" => {
                let v = self.cur.read_u64::<LittleEndian>().map_err(|e| e.to_string())?;
                Ok(json!(v))
            }
            "Guid" => {
                let uuid = read_uuid(&mut self.cur).map_err(|e| e.to_string())?;
                Ok(json!(uuid))
            }
            "LinearColor" => {
                let r = self.cur.read_f32::<LittleEndian>().map_err(|e| e.to_string())?;
                let g = self.cur.read_f32::<LittleEndian>().map_err(|e| e.to_string())?;
                let b = self.cur.read_f32::<LittleEndian>().map_err(|e| e.to_string())?;
                let a = self.cur.read_f32::<LittleEndian>().map_err(|e| e.to_string())?;
                Ok(json!({"r": r, "g": g, "b": b, "a": a}))
            }
            // ── Additional fixed-size UE struct types ──
            "IntVector" => {
                let x = self.cur.read_i32::<LittleEndian>().map_err(|e| e.to_string())?;
                let y = self.cur.read_i32::<LittleEndian>().map_err(|e| e.to_string())?;
                let z = self.cur.read_i32::<LittleEndian>().map_err(|e| e.to_string())?;
                Ok(json!({"x": x, "y": y, "z": z}))
            }
            "IntPoint" => {
                let x = self.cur.read_i32::<LittleEndian>().map_err(|e| e.to_string())?;
                let y = self.cur.read_i32::<LittleEndian>().map_err(|e| e.to_string())?;
                Ok(json!({"x": x, "y": y}))
            }
            "Vector2D" => {
                let x = self.cur.read_f64::<LittleEndian>().map_err(|e| e.to_string())?;
                let y = self.cur.read_f64::<LittleEndian>().map_err(|e| e.to_string())?;
                Ok(json!({"x": x, "y": y}))
            }
            "Vector4" | "Plane" => {
                let x = self.cur.read_f64::<LittleEndian>().map_err(|e| e.to_string())?;
                let y = self.cur.read_f64::<LittleEndian>().map_err(|e| e.to_string())?;
                let z = self.cur.read_f64::<LittleEndian>().map_err(|e| e.to_string())?;
                let w = self.cur.read_f64::<LittleEndian>().map_err(|e| e.to_string())?;
                Ok(json!({"x": x, "y": y, "z": z, "w": w}))
            }
            "Color" => {
                let b = self.cur.read_u8().map_err(|e| e.to_string())?;
                let g = self.cur.read_u8().map_err(|e| e.to_string())?;
                let r = self.cur.read_u8().map_err(|e| e.to_string())?;
                let a = self.cur.read_u8().map_err(|e| e.to_string())?;
                Ok(json!({"r": r, "g": g, "b": b, "a": a}))
            }
            "Timespan" => {
                let v = self.cur.read_i64::<LittleEndian>().map_err(|e| e.to_string())?;
                Ok(json!(v))
            }
            "Vector2f" | "Vector2D_f" => {
                let x = self.cur.read_f32::<LittleEndian>().map_err(|e| e.to_string())?;
                let y = self.cur.read_f32::<LittleEndian>().map_err(|e| e.to_string())?;
                Ok(json!({"x": x, "y": y}))
            }
            "Vector3f" => {
                let x = self.cur.read_f32::<LittleEndian>().map_err(|e| e.to_string())?;
                let y = self.cur.read_f32::<LittleEndian>().map_err(|e| e.to_string())?;
                let z = self.cur.read_f32::<LittleEndian>().map_err(|e| e.to_string())?;
                Ok(json!({"x": x, "y": y, "z": z}))
            }
            "Box" => {
                // FBox: min (3×f64) + max (3×f64) + valid (u8)
                let min_x = self.cur.read_f64::<LittleEndian>().map_err(|e| e.to_string())?;
                let min_y = self.cur.read_f64::<LittleEndian>().map_err(|e| e.to_string())?;
                let min_z = self.cur.read_f64::<LittleEndian>().map_err(|e| e.to_string())?;
                let max_x = self.cur.read_f64::<LittleEndian>().map_err(|e| e.to_string())?;
                let max_y = self.cur.read_f64::<LittleEndian>().map_err(|e| e.to_string())?;
                let max_z = self.cur.read_f64::<LittleEndian>().map_err(|e| e.to_string())?;
                let valid = self.cur.read_u8().map_err(|e| e.to_string())?;
                Ok(json!({"min": {"x": min_x, "y": min_y, "z": min_z}, "max": {"x": max_x, "y": max_y, "z": max_z}, "valid": valid != 0}))
            }
            _ => {
                // Generic struct: read nested properties
                let props = self.read_properties(path)?;
                Ok(Value::Object(props))
            }
        }
    }

    // ── Array property ──

    fn read_array_property(&mut self, size: usize, path: &str) -> Result<Value, String> {
        let array_type = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
        let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;

        // Custom decode for Character RawData
        if is_character_rawdata_path(path) && array_type == "ByteProperty" {
            let inner = self.read_character_rawdata(size)?;
            return Ok(json!({
                "array_type": array_type,
                "id": id,
                "value": inner,
                "type": "ArrayProperty",
                "custom_type": "character_rawdata"
            }));
        }

        let data_size = size.saturating_sub(4); // subtract count u32
        let inner = self.read_array_value(&array_type, data_size, path)?;

        Ok(json!({
            "array_type": array_type,
            "id": id,
            "value": inner,
            "type": "ArrayProperty"
        }))
    }

    fn read_array_value(&mut self, array_type: &str, size: usize, path: &str) -> Result<Value, String> {
        let count = self.cur.read_u32::<LittleEndian>().map_err(|e| e.to_string())? as usize;

        if array_type == "StructProperty" {
            let prop_name = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
            let prop_type = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
            let _element_size = self.cur.read_u64::<LittleEndian>().map_err(|e| e.to_string())?;
            let type_name = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
            let arr_id = read_uuid(&mut self.cur).map_err(|e| e.to_string())?;
            let has_guid = self.cur.read_u8().map_err(|e| e.to_string())?;
            // If has_guid flag is set, skip the 16-byte property GUID
            if has_guid != 0 {
                let mut _guid = [0u8; 16];
                self.cur.read_exact(&mut _guid).map_err(|e| e.to_string())?;
            }

            let mut values = Vec::with_capacity(count);
            for _i in 0..count {
                let sv = self.read_struct_value(&type_name, 0, path)?;
                values.push(sv);
            }

            return Ok(json!({
                "prop_name": prop_name,
                "prop_type": prop_type,
                "type_name": type_name,
                "id": arr_id,
                "values": values
            }));
        }

        // Non-struct arrays
        let mut values = Vec::with_capacity(count);
        match array_type {
            "EnumProperty" | "NameProperty" | "StrProperty" => {
                for _ in 0..count {
                    let s = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
                    values.push(json!(s));
                }
            }
            "Guid" => {
                for _ in 0..count {
                    let u = read_uuid(&mut self.cur).map_err(|e| e.to_string())?;
                    values.push(json!(u));
                }
            }
            "SoftObjectProperty" => {
                for _ in 0..count {
                    let p = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
                    let sp = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
                    values.push(json!({"path": p, "sub_path": sp}));
                }
            }
            "ObjectProperty" => {
                for _ in 0..count {
                    let s = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
                    values.push(json!(s));
                }
            }
            "ByteProperty" => {
                // Raw byte array
                if size == count + 4 {
                    // Exactly count bytes
                    let mut raw = vec![0u8; count];
                    self.cur.read_exact(&mut raw).map_err(|e| e.to_string())?;
                    return Ok(json!({"values": raw}));
                }
                // Otherwise individual bytes
                for _ in 0..count {
                    let b = self.cur.read_u8().map_err(|e| e.to_string())?;
                    values.push(json!(b));
                }
            }
            "IntProperty" => {
                for _ in 0..count {
                    let v = self.cur.read_i32::<LittleEndian>().map_err(|e| e.to_string())?;
                    values.push(json!(v));
                }
            }
            "UInt32Property" => {
                for _ in 0..count {
                    let v = self.cur.read_u32::<LittleEndian>().map_err(|e| e.to_string())?;
                    values.push(json!(v));
                }
            }
            "Int64Property" => {
                for _ in 0..count {
                    let v = self.cur.read_i64::<LittleEndian>().map_err(|e| e.to_string())?;
                    values.push(json!(v));
                }
            }
            "UInt64Property" => {
                for _ in 0..count {
                    let v = self.cur.read_u64::<LittleEndian>().map_err(|e| e.to_string())?;
                    values.push(json!(v));
                }
            }
            "FloatProperty" => {
                for _ in 0..count {
                    let v = self.cur.read_f32::<LittleEndian>().map_err(|e| e.to_string())?;
                    values.push(json!(v));
                }
            }
            "BoolProperty" => {
                for _ in 0..count {
                    let v = self.cur.read_u8().map_err(|e| e.to_string())?;
                    values.push(json!(v != 0));
                }
            }
            _ => {
                // Unknown array element type — read remaining as raw
                if count > 0 && size >= 4 {
                    let remaining = size - 4;
                    let mut raw = vec![0u8; remaining];
                    self.cur.read_exact(&mut raw).map_err(|e| e.to_string())?;
                    return Ok(json!({"values": base64_encode(&raw), "raw": true}));
                }
            }
        }
        Ok(json!({"values": values}))
    }

    // ── Map property ──

    fn read_map_property(&mut self, _size: usize, path: &str) -> Result<Value, String> {
        let key_type = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
        let value_type = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
        let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
        let _unknown = self.cur.read_u32::<LittleEndian>().map_err(|e| e.to_string())?;
        let count = self.cur.read_u32::<LittleEndian>().map_err(|e| e.to_string())? as usize;

        let key_struct_hint = type_hint_for(&format!("{path}.Key")).unwrap_or("");
        let val_struct_hint = type_hint_for(&format!("{path}.Value")).unwrap_or("");

        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let key = self.read_map_value(&key_type, key_struct_hint, &format!("{path}.Key"))?;
            let val = self.read_map_value(&value_type, val_struct_hint, &format!("{path}.Value"))?;
            entries.push(json!({"key": key, "value": val}));
        }

        Ok(json!({
            "key_type": key_type,
            "value_type": value_type,
            "key_struct_type": if key_type == "StructProperty" { Some(key_struct_hint) } else { None::<&str> },
            "value_struct_type": if value_type == "StructProperty" { Some(val_struct_hint) } else { None::<&str> },
            "id": id,
            "value": entries,
            "type": "MapProperty"
        }))
    }

    fn read_map_value(&mut self, type_name: &str, struct_hint: &str, path: &str) -> Result<Value, String> {
        match type_name {
            "StructProperty" => self.read_struct_value(struct_hint, 0, path),
            "EnumProperty" | "NameProperty" | "StrProperty" => {
                let s = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
                Ok(json!(s))
            }
            "IntProperty" => {
                let v = self.cur.read_i32::<LittleEndian>().map_err(|e| e.to_string())?;
                Ok(json!(v))
            }
            "Int64Property" => {
                let v = self.cur.read_i64::<LittleEndian>().map_err(|e| e.to_string())?;
                Ok(json!(v))
            }
            "UInt32Property" => {
                let v = self.cur.read_u32::<LittleEndian>().map_err(|e| e.to_string())?;
                Ok(json!(v))
            }
            "BoolProperty" => {
                let v = self.cur.read_u8().map_err(|e| e.to_string())?;
                Ok(json!(v != 0))
            }
            "ObjectProperty" => {
                let s = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
                Ok(json!(s))
            }
            "Guid" => {
                let u = read_uuid(&mut self.cur).map_err(|e| e.to_string())?;
                Ok(json!(u))
            }
            _ => {
                // Best-effort: try as struct properties
                let props = self.read_properties(path)?;
                Ok(Value::Object(props))
            }
        }
    }

    // ── Set property ──

    fn read_set_property(&mut self, _size: usize, path: &str) -> Result<Value, String> {
        let set_type = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
        let id = read_optional_uuid(&mut self.cur).map_err(|e| e.to_string())?;
        let _unknown = self.cur.read_u32::<LittleEndian>().map_err(|e| e.to_string())?;
        let count = self.cur.read_u32::<LittleEndian>().map_err(|e| e.to_string())? as usize;

        let mut entries = Vec::with_capacity(count);
        match set_type.as_str() {
            "StructProperty" => {
                for _ in 0..count {
                    let props = self.read_properties(path)?;
                    entries.push(Value::Object(props));
                }
            }
            "NameProperty" | "StrProperty" | "EnumProperty" | "ObjectProperty" => {
                for _ in 0..count {
                    let s = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
                    entries.push(json!(s));
                }
            }
            "IntProperty" => {
                for _ in 0..count {
                    let v = self.cur.read_i32::<LittleEndian>().map_err(|e| e.to_string())?;
                    entries.push(json!(v));
                }
            }
            "UInt32Property" => {
                for _ in 0..count {
                    let v = self.cur.read_u32::<LittleEndian>().map_err(|e| e.to_string())?;
                    entries.push(json!(v));
                }
            }
            "Int64Property" => {
                for _ in 0..count {
                    let v = self.cur.read_i64::<LittleEndian>().map_err(|e| e.to_string())?;
                    entries.push(json!(v));
                }
            }
            "UInt64Property" => {
                for _ in 0..count {
                    let v = self.cur.read_u64::<LittleEndian>().map_err(|e| e.to_string())?;
                    entries.push(json!(v));
                }
            }
            "Guid" => {
                for _ in 0..count {
                    let u = read_uuid(&mut self.cur).map_err(|e| e.to_string())?;
                    entries.push(json!(u));
                }
            }
            "SoftObjectProperty" => {
                for _ in 0..count {
                    let p = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
                    let sp = read_fstring(&mut self.cur).map_err(|e| e.to_string())?;
                    entries.push(json!({"path": p, "sub_path": sp}));
                }
            }
            _ => {
                // Fallback: treat as property bags
                for _ in 0..count {
                    let props = self.read_properties(path)?;
                    entries.push(Value::Object(props));
                }
            }
        }

        Ok(json!({
            "set_type": set_type,
            "id": id,
            "value": entries,
            "type": "SetProperty"
        }))
    }

    // ── Custom: GroupSaveDataMap ──
    // Reads the MapProperty normally, then decodes the RawData in each guild entry.

    fn read_group_map_property(&mut self, size: usize, path: &str) -> Result<Value, String> {
        let mut result = self.read_map_property(size, path)?;

        // Decode group RawData for each entry
        if let Some(entries) = result.get_mut("value").and_then(|v| v.as_array_mut()) {
            for entry in entries.iter_mut() {
                let group_type = entry
                    .pointer("/value/GroupType/value/value")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                if let Some(raw_data) = entry.pointer("/value/RawData") {
                    if let Some(raw_array) = raw_data
                        .pointer("/value/values")
                        .and_then(|v| v.as_array())
                    {
                        // Convert JSON byte array to actual bytes
                        let bytes: Vec<u8> = raw_array
                            .iter()
                            .filter_map(|v| v.as_u64().map(|n| n as u8))
                            .collect();
                        if !bytes.is_empty() {
                            if let Ok(decoded) = decode_group_rawdata(&bytes, &group_type) {
                                // Replace RawData.value with decoded struct
                                if let Some(rd) = entry.pointer_mut("/value/RawData/value") {
                                    *rd = decoded;
                                }
                            }
                        }
                    }
                }
            }
        }

        result["custom_type"] = json!("group_rawdata_map");
        Ok(result)
    }

    // ── Custom: Character RawData ──

    fn read_character_rawdata(&mut self, _size: usize) -> Result<Value, String> {
        let count = self.cur.read_u32::<LittleEndian>().map_err(|e| e.to_string())? as usize;
        // Read the raw byte array
        let mut raw = vec![0u8; count];
        self.cur.read_exact(&mut raw).map_err(|e| e.to_string())?;

        // Decode character rawdata
        let decoded = decode_character_rawdata(&raw)?;
        Ok(decoded)
    }

    fn read_trailer(&mut self) -> Result<Vec<u8>, String> {
        let mut trailer = Vec::new();
        self.cur.read_to_end(&mut trailer).map_err(|e| e.to_string())?;
        Ok(trailer)
    }
}

// ── Group RawData decoder ───────────────────────────────

fn decode_group_rawdata(data: &[u8], group_type: &str) -> Result<Value, String> {
    let mut cur = Cursor::new(data as &[u8]);

    let group_id = read_uuid(&mut cur).map_err(|e| e.to_string())?;
    let group_name = read_fstring(&mut cur).map_err(|e| e.to_string())?;

    // individual_character_handle_ids
    let handle_count = cur.read_u32::<LittleEndian>().map_err(|e| e.to_string())? as usize;
    let mut handles = Vec::with_capacity(handle_count);
    for _ in 0..handle_count {
        let guid = read_uuid(&mut cur).map_err(|e| e.to_string())?;
        let instance_id = read_uuid(&mut cur).map_err(|e| e.to_string())?;
        handles.push(json!({"guid": guid, "instance_id": instance_id}));
    }

    let mut result = json!({
        "group_id": group_id,
        "group_name": group_name,
        "individual_character_handle_ids": handles,
    });

    let is_guild = group_type == "EPalGroupType::Guild";
    let is_indep = group_type == "EPalGroupType::IndependentGuild";
    let is_org = group_type == "EPalGroupType::Organization";

    if is_guild || is_indep || is_org {
        let org_type = cur.read_u8().map_err(|e| e.to_string())?;
        result["org_type"] = json!(org_type);
    }

    if is_org {
        let mut trail = [0u8; 12];
        cur.read_exact(&mut trail).map_err(|e| e.to_string())?;
        result["trailing_bytes"] = json!(trail.to_vec());
        return Ok(result);
    }

    if is_guild {
        // Guild-specific fields
        let mut leading = [0u8; 4];
        cur.read_exact(&mut leading).map_err(|e| e.to_string())?;
        result["leading_bytes"] = json!(leading.to_vec());

        // base_ids
        let base_count = cur.read_u32::<LittleEndian>().map_err(|e| e.to_string())? as usize;
        let mut base_ids = Vec::with_capacity(base_count);
        for _ in 0..base_count {
            base_ids.push(json!(read_uuid(&mut cur).map_err(|e| e.to_string())?));
        }
        result["base_ids"] = json!(base_ids);

        let unknown_1 = cur.read_i32::<LittleEndian>().map_err(|e| e.to_string())?;
        result["unknown_1"] = json!(unknown_1);

        let base_camp_level = cur.read_i32::<LittleEndian>().map_err(|e| e.to_string())?;
        result["base_camp_level"] = json!(base_camp_level);

        // map_object_instance_ids_base_camp_points
        let moibc_count = cur.read_u32::<LittleEndian>().map_err(|e| e.to_string())? as usize;
        let mut moibc = Vec::with_capacity(moibc_count);
        for _ in 0..moibc_count {
            moibc.push(json!(read_uuid(&mut cur).map_err(|e| e.to_string())?));
        }
        result["map_object_instance_ids_base_camp_points"] = json!(moibc);

        let guild_name = read_fstring(&mut cur).map_err(|e| e.to_string())?;
        result["guild_name"] = json!(guild_name);

        let last_modifier = read_uuid(&mut cur).map_err(|e| e.to_string())?;
        result["last_guild_name_modifier_player_uid"] = json!(last_modifier);

        let mut unknown_2 = [0u8; 4];
        cur.read_exact(&mut unknown_2).map_err(|e| e.to_string())?;
        result["unknown_2"] = json!(unknown_2.to_vec());

        let admin = read_uuid(&mut cur).map_err(|e| e.to_string())?;
        result["admin_player_uid"] = json!(admin);

        // Players array
        let player_count = cur.read_u32::<LittleEndian>().map_err(|e| e.to_string())? as usize;
        let mut players = Vec::with_capacity(player_count);
        for _ in 0..player_count {
            let player_uid = read_uuid(&mut cur).map_err(|e| e.to_string())?;
            let last_online = cur.read_i64::<LittleEndian>().map_err(|e| e.to_string())?;
            let player_name = read_fstring(&mut cur).map_err(|e| e.to_string())?;
            players.push(json!({
                "player_uid": player_uid,
                "player_info": {
                    "last_online_real_time": last_online,
                    "player_name": player_name
                }
            }));
        }
        result["players"] = json!(players);

        // Trailing bytes - read whatever remains
        let pos = cur.position() as usize;
        let remaining = &data[pos..];
        result["trailing_bytes"] = json!(remaining.to_vec());
    }

    if is_indep {
        let base_camp_level = cur.read_i32::<LittleEndian>().map_err(|e| e.to_string())?;
        result["base_camp_level"] = json!(base_camp_level);

        let moibc_count = cur.read_u32::<LittleEndian>().map_err(|e| e.to_string())? as usize;
        let mut moibc = Vec::with_capacity(moibc_count);
        for _ in 0..moibc_count {
            moibc.push(json!(read_uuid(&mut cur).map_err(|e| e.to_string())?));
        }
        result["map_object_instance_ids_base_camp_points"] = json!(moibc);

        let guild_name = read_fstring(&mut cur).map_err(|e| e.to_string())?;
        result["guild_name"] = json!(guild_name);

        let player_uid = read_uuid(&mut cur).map_err(|e| e.to_string())?;
        result["player_uid"] = json!(player_uid);

        let guild_name_2 = read_fstring(&mut cur).map_err(|e| e.to_string())?;
        result["guild_name_2"] = json!(guild_name_2);

        let last_online = cur.read_i64::<LittleEndian>().map_err(|e| e.to_string())?;
        let player_name = read_fstring(&mut cur).map_err(|e| e.to_string())?;
        result["player_info"] = json!({
            "last_online_real_time": last_online,
            "player_name": player_name
        });
    }

    Ok(result)
}

// ── Character RawData decoder ───────────────────────────

fn decode_character_rawdata(data: &[u8]) -> Result<Value, String> {
    // The character rawdata is: object_properties + 4 unknown bytes + group_id(16) + 4 trailing bytes
    // But the object properties are variable length (terminated by "None" FString).
    // We parse the properties, then read the remaining fixed fields.
    let data_ref: &[u8] = data;
    let mut reader = GvasReader::new(data_ref);
    let props = reader.read_properties("")?;
    let pos = reader.position() as usize;
    let remaining = &data[pos..];

    if remaining.len() >= 24 {
        let mut cur = Cursor::new(remaining as &[u8]);
        let mut unknown = [0u8; 4];
        cur.read_exact(&mut unknown).map_err(|e| e.to_string())?;
        let group_id = read_uuid(&mut cur).map_err(|e| e.to_string())?;
        let mut trail = [0u8; 4];
        cur.read_exact(&mut trail).map_err(|e| e.to_string())?;
        Ok(json!({
            "object": {"SaveParameter": { "struct_type": "PalIndividualCharacterSaveParameter", "value": props }},
            "unknown_bytes": unknown.to_vec(),
            "group_id": group_id,
            "trailing_bytes": trail.to_vec()
        }))
    } else {
        Ok(json!({
            "object": {"SaveParameter": { "struct_type": "PalIndividualCharacterSaveParameter", "value": props }},
            "unknown_bytes": [],
            "group_id": "00000000-0000-0000-0000-000000000000",
            "trailing_bytes": remaining.to_vec()
        }))
    }
}

// ── Base64 helper (we use this for large raw data skip blobs) ──

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        out.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let mut table = [255u8; 128];
    for (i, &c) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
        .iter()
        .enumerate()
    {
        table[c as usize] = i as u8;
    }
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &b in s.as_bytes() {
        if b == b'=' || b == b'\n' || b == b'\r' || b == b' ' {
            continue;
        }
        if (b as usize) >= 128 || table[b as usize] == 255 {
            return Err("Invalid base64".into());
        }
        buf = (buf << 6) | table[b as usize] as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

// ── GVAS writer ─────────────────────────────────────────

struct GvasWriter {
    buf: Vec<u8>,
}

impl GvasWriter {
    fn new() -> Self {
        Self {
            buf: Vec::with_capacity(1024 * 1024),
        }
    }

    fn write_header(&mut self, header: &Value) -> Result<(), String> {
        let h = header.as_object().ok_or("header must be object")?;
        self.buf
            .write_i32::<LittleEndian>(h["magic"].as_i64().unwrap_or(0x53415647) as i32)
            .map_err(|e| e.to_string())?;
        self.buf
            .write_i32::<LittleEndian>(h["save_game_version"].as_i64().unwrap_or(3) as i32)
            .map_err(|e| e.to_string())?;
        self.buf
            .write_i32::<LittleEndian>(h["package_file_version_ue4"].as_i64().unwrap_or(0) as i32)
            .map_err(|e| e.to_string())?;
        self.buf
            .write_i32::<LittleEndian>(h["package_file_version_ue5"].as_i64().unwrap_or(0) as i32)
            .map_err(|e| e.to_string())?;
        self.buf
            .write_u16::<LittleEndian>(h["engine_version_major"].as_u64().unwrap_or(0) as u16)
            .map_err(|e| e.to_string())?;
        self.buf
            .write_u16::<LittleEndian>(h["engine_version_minor"].as_u64().unwrap_or(0) as u16)
            .map_err(|e| e.to_string())?;
        self.buf
            .write_u16::<LittleEndian>(h["engine_version_patch"].as_u64().unwrap_or(0) as u16)
            .map_err(|e| e.to_string())?;
        self.buf
            .write_u32::<LittleEndian>(h["engine_version_changelist"].as_u64().unwrap_or(0) as u32)
            .map_err(|e| e.to_string())?;
        write_fstring(
            &mut self.buf,
            h["engine_version_branch"].as_str().unwrap_or(""),
        )?;
        self.buf
            .write_i32::<LittleEndian>(h["custom_version_format"].as_i64().unwrap_or(3) as i32)
            .map_err(|e| e.to_string())?;

        let cvs = h["custom_versions"].as_array().ok_or("custom_versions")?;
        self.buf
            .write_u32::<LittleEndian>(cvs.len() as u32)
            .map_err(|e| e.to_string())?;
        for cv in cvs {
            let arr = cv.as_array().ok_or("custom_version entry")?;
            write_uuid(&mut self.buf, arr[0].as_str().unwrap_or(""))?;
            self.buf
                .write_i32::<LittleEndian>(arr[1].as_i64().unwrap_or(0) as i32)
                .map_err(|e| e.to_string())?;
        }

        write_fstring(
            &mut self.buf,
            h["save_game_class_name"].as_str().unwrap_or(""),
        )?;
        Ok(())
    }

    fn write_properties(&mut self, props: &Map<String, Value>) -> Result<(), String> {
        for (name, val) in props {
            let type_name = val["type"].as_str().unwrap_or("StructProperty");
            write_fstring(&mut self.buf, name)?;
            write_fstring(&mut self.buf, type_name)?;
            // Write property body to temp buffer; property_inner returns the
            // "data size" (value-only bytes, excluding type-specific metadata)
            let mut body_writer = GvasWriter::new();
            let data_size = body_writer.write_property_inner(type_name, val)?;
            let body = body_writer.buf;
            self.buf
                .write_u64::<LittleEndian>(data_size as u64)
                .map_err(|e| e.to_string())?;
            self.buf.extend_from_slice(&body);
        }
        // Terminator
        write_fstring(&mut self.buf, "None")?;
        Ok(())
    }

    /// Write a property body (metadata + value data) and return the "data size"
    /// (the number of value-data bytes, excluding type-specific metadata).
    /// In the GVAS wire format the size field counts ONLY value bytes.
    fn write_property_inner(&mut self, type_name: &str, val: &Value) -> Result<usize, String> {
        // Check for skip-decoded property
        if val.get("skip_type").is_some() {
            return self.write_skip_property(type_name, val);
        }

        // Check for custom types
        if let Some(ct) = val.get("custom_type").and_then(|v| v.as_str()) {
            match ct {
                "group_rawdata_map" => return self.write_group_map_property_sized(val),
                "character_rawdata" => {
                    // Write array header then encoded rawdata
                    let array_type = val["array_type"].as_str().unwrap_or("ByteProperty");
                    write_fstring(&mut self.buf, array_type)?;
                    write_optional_uuid(&mut self.buf, &val["id"])?;
                    let start = self.buf.len();
                    let encoded = encode_character_rawdata(&val["value"])?;
                    self.buf
                        .write_u32::<LittleEndian>(encoded.len() as u32)
                        .map_err(|e| e.to_string())?;
                    self.buf.extend_from_slice(&encoded);
                    return Ok(self.buf.len() - start);
                }
                "raw_text" | "unknown_skip" => {
                    write_optional_uuid(&mut self.buf, &val["id"])?;
                    let raw = base64_decode(val["value"].as_str().unwrap_or(""))?;
                    let size = raw.len();
                    self.buf.extend_from_slice(&raw);
                    return Ok(size);
                }
                _ => {}
            }
        }

        match type_name {
            "IntProperty" => {
                write_optional_uuid(&mut self.buf, &val["id"])?;
                self.buf
                    .write_i32::<LittleEndian>(val["value"].as_i64().unwrap_or(0) as i32)
                    .map_err(|e| e.to_string())?;
                Ok(4)
            }
            "UInt16Property" => {
                write_optional_uuid(&mut self.buf, &val["id"])?;
                self.buf
                    .write_u16::<LittleEndian>(val["value"].as_u64().unwrap_or(0) as u16)
                    .map_err(|e| e.to_string())?;
                Ok(2)
            }
            "UInt32Property" => {
                write_optional_uuid(&mut self.buf, &val["id"])?;
                self.buf
                    .write_u32::<LittleEndian>(val["value"].as_u64().unwrap_or(0) as u32)
                    .map_err(|e| e.to_string())?;
                Ok(4)
            }
            "UInt64Property" => {
                write_optional_uuid(&mut self.buf, &val["id"])?;
                self.buf
                    .write_u64::<LittleEndian>(val["value"].as_u64().unwrap_or(0))
                    .map_err(|e| e.to_string())?;
                Ok(8)
            }
            "Int64Property" => {
                write_optional_uuid(&mut self.buf, &val["id"])?;
                self.buf
                    .write_i64::<LittleEndian>(val["value"].as_i64().unwrap_or(0))
                    .map_err(|e| e.to_string())?;
                Ok(8)
            }
            "FixedPoint64Property" => {
                write_optional_uuid(&mut self.buf, &val["id"])?;
                self.buf
                    .write_i32::<LittleEndian>(val["value"].as_i64().unwrap_or(0) as i32)
                    .map_err(|e| e.to_string())?;
                Ok(4)
            }
            "FloatProperty" => {
                write_optional_uuid(&mut self.buf, &val["id"])?;
                self.buf
                    .write_f32::<LittleEndian>(val["value"].as_f64().unwrap_or(0.0) as f32)
                    .map_err(|e| e.to_string())?;
                Ok(4)
            }
            "DoubleProperty" => {
                write_optional_uuid(&mut self.buf, &val["id"])?;
                self.buf
                    .write_f64::<LittleEndian>(val["value"].as_f64().unwrap_or(0.0))
                    .map_err(|e| e.to_string())?;
                Ok(8)
            }
            "StrProperty" | "NameProperty" => {
                write_optional_uuid(&mut self.buf, &val["id"])?;
                let start = self.buf.len();
                write_fstring(&mut self.buf, val["value"].as_str().unwrap_or(""))?;
                Ok(self.buf.len() - start)
            }
            "BoolProperty" => {
                // BoolProperty: value byte BEFORE optional_guid; size = 0
                let bval = val["value"].as_bool().unwrap_or(false);
                self.buf.push(if bval { 1 } else { 0 });
                write_optional_uuid(&mut self.buf, &val["id"])?;
                Ok(0)
            }
            "EnumProperty" => {
                write_fstring(
                    &mut self.buf,
                    val["value"]["type"].as_str().unwrap_or(""),
                )?;
                write_optional_uuid(&mut self.buf, &val["id"])?;
                let start = self.buf.len();
                write_fstring(
                    &mut self.buf,
                    val["value"]["value"].as_str().unwrap_or(""),
                )?;
                Ok(self.buf.len() - start)
            }
            "ByteProperty" => {
                let enum_type = val["value"]["type"].as_str().unwrap_or("None");
                write_fstring(&mut self.buf, enum_type)?;
                write_optional_uuid(&mut self.buf, &val["id"])?;
                let start = self.buf.len();
                if enum_type == "None" {
                    self.buf.push(val["value"]["value"].as_u64().unwrap_or(0) as u8);
                } else {
                    write_fstring(
                        &mut self.buf,
                        val["value"]["value"].as_str().unwrap_or(""),
                    )?;
                }
                Ok(self.buf.len() - start)
            }
            "SoftObjectProperty" => {
                write_optional_uuid(&mut self.buf, &val["id"])?;
                let start = self.buf.len();
                write_fstring(
                    &mut self.buf,
                    val["value"]["path"].as_str().unwrap_or(""),
                )?;
                write_fstring(
                    &mut self.buf,
                    val["value"]["sub_path"].as_str().unwrap_or(""),
                )?;
                Ok(self.buf.len() - start)
            }
            "ObjectProperty" => {
                write_optional_uuid(&mut self.buf, &val["id"])?;
                let start = self.buf.len();
                write_fstring(&mut self.buf, val["value"].as_str().unwrap_or(""))?;
                Ok(self.buf.len() - start)
            }
            "StructProperty" => {
                let struct_type = val["struct_type"].as_str().unwrap_or("");
                write_fstring(&mut self.buf, struct_type)?;
                write_uuid(&mut self.buf, val["struct_id"].as_str().unwrap_or("00000000-0000-0000-0000-000000000000"))?;
                write_optional_uuid(&mut self.buf, &val["id"])?;
                let start = self.buf.len();
                self.write_struct_value(struct_type, &val["value"])?;
                Ok(self.buf.len() - start)
            }
            "ArrayProperty" => {
                let array_type = val["array_type"].as_str().unwrap_or("");
                write_fstring(&mut self.buf, array_type)?;
                write_optional_uuid(&mut self.buf, &val["id"])?;
                let start = self.buf.len();
                self.write_array_value(array_type, &val["value"])?;
                Ok(self.buf.len() - start)
            }
            "MapProperty" => {
                self.write_map_property_body_sized(val)
            }
            "SetProperty" => {
                let set_type = val["set_type"].as_str().unwrap_or("");
                write_fstring(&mut self.buf, set_type)?;
                write_optional_uuid(&mut self.buf, &val["id"])?;
                let start = self.buf.len();
                self.buf.write_u32::<LittleEndian>(0).map_err(|e| e.to_string())?; // unknown
                let entries = val["value"].as_array().unwrap_or_else(|| &EMPTY_VEC);
                self.buf
                    .write_u32::<LittleEndian>(entries.len() as u32)
                    .map_err(|e| e.to_string())?;
                for entry in entries {
                    match set_type {
                        "StructProperty" | "" => {
                            if let Some(obj) = entry.as_object() {
                                self.write_properties(obj)?;
                            }
                        }
                        "NameProperty" | "StrProperty" | "EnumProperty" | "ObjectProperty" => {
                            write_fstring(&mut self.buf, entry.as_str().unwrap_or(""))?;
                        }
                        "IntProperty" => {
                            self.buf.write_i32::<LittleEndian>(entry.as_i64().unwrap_or(0) as i32).map_err(|e| e.to_string())?;
                        }
                        "UInt32Property" => {
                            self.buf.write_u32::<LittleEndian>(entry.as_u64().unwrap_or(0) as u32).map_err(|e| e.to_string())?;
                        }
                        "Int64Property" => {
                            self.buf.write_i64::<LittleEndian>(entry.as_i64().unwrap_or(0)).map_err(|e| e.to_string())?;
                        }
                        "UInt64Property" => {
                            self.buf.write_u64::<LittleEndian>(entry.as_u64().unwrap_or(0)).map_err(|e| e.to_string())?;
                        }
                        "Guid" => {
                            write_uuid(&mut self.buf, entry.as_str().unwrap_or("00000000-0000-0000-0000-000000000000"))?;
                        }
                        "SoftObjectProperty" => {
                            write_fstring(&mut self.buf, entry["path"].as_str().unwrap_or(""))?;
                            write_fstring(&mut self.buf, entry["sub_path"].as_str().unwrap_or(""))?;
                        }
                        _ => {
                            if let Some(obj) = entry.as_object() {
                                self.write_properties(obj)?;
                            }
                        }
                    }
                }
                Ok(self.buf.len() - start)
            }
            "TextProperty" => {
                write_optional_uuid(&mut self.buf, &val["id"])?;
                let raw = base64_decode(val["value"].as_str().unwrap_or(""))?;
                let size = raw.len();
                self.buf.extend_from_slice(&raw);
                Ok(size)
            }
            _ => {
                // Unknown: write stored raw data
                write_optional_uuid(&mut self.buf, &val["id"])?;
                if let Some(raw_b64) = val["value"].as_str() {
                    let raw = base64_decode(raw_b64)?;
                    let size = raw.len();
                    self.buf.extend_from_slice(&raw);
                    Ok(size)
                } else {
                    Ok(0)
                }
            }
        }
    }

    fn write_skip_property(&mut self, type_name: &str, val: &Value) -> Result<usize, String> {
        let raw = base64_decode(val["value"].as_str().unwrap_or(""))?;
        let data_size = raw.len();
        match type_name {
            "ArrayProperty" => {
                write_fstring(&mut self.buf, val["array_type"].as_str().unwrap_or(""))?;
                write_optional_uuid(&mut self.buf, &val["id"])?;
                self.buf.extend_from_slice(&raw);
            }
            "MapProperty" => {
                write_fstring(&mut self.buf, val["key_type"].as_str().unwrap_or(""))?;
                write_fstring(&mut self.buf, val["value_type"].as_str().unwrap_or(""))?;
                write_optional_uuid(&mut self.buf, &val["id"])?;
                self.buf.extend_from_slice(&raw);
            }
            "StructProperty" => {
                write_fstring(&mut self.buf, val["struct_type"].as_str().unwrap_or(""))?;
                write_uuid(
                    &mut self.buf,
                    val["struct_id"]
                        .as_str()
                        .unwrap_or("00000000-0000-0000-0000-000000000000"),
                )?;
                write_optional_uuid(&mut self.buf, &val["id"])?;
                self.buf.extend_from_slice(&raw);
            }
            "SetProperty" => {
                write_fstring(&mut self.buf, val["set_type"].as_str().unwrap_or(""))?;
                write_optional_uuid(&mut self.buf, &val["id"])?;
                self.buf.extend_from_slice(&raw);
            }
            _ => {
                write_optional_uuid(&mut self.buf, &val["id"])?;
                self.buf.extend_from_slice(&raw);
            }
        }
        Ok(data_size)
    }

    fn write_struct_value(&mut self, struct_type: &str, val: &Value) -> Result<(), String> {
        match struct_type {
            "Vector" | "Rotator" => {
                self.buf
                    .write_f64::<LittleEndian>(val["x"].as_f64().unwrap_or(0.0))
                    .map_err(|e| e.to_string())?;
                self.buf
                    .write_f64::<LittleEndian>(val["y"].as_f64().unwrap_or(0.0))
                    .map_err(|e| e.to_string())?;
                self.buf
                    .write_f64::<LittleEndian>(val["z"].as_f64().unwrap_or(0.0))
                    .map_err(|e| e.to_string())?;
            }
            "Quat" => {
                self.buf
                    .write_f64::<LittleEndian>(val["x"].as_f64().unwrap_or(0.0))
                    .map_err(|e| e.to_string())?;
                self.buf
                    .write_f64::<LittleEndian>(val["y"].as_f64().unwrap_or(0.0))
                    .map_err(|e| e.to_string())?;
                self.buf
                    .write_f64::<LittleEndian>(val["z"].as_f64().unwrap_or(0.0))
                    .map_err(|e| e.to_string())?;
                self.buf
                    .write_f64::<LittleEndian>(val["w"].as_f64().unwrap_or(0.0))
                    .map_err(|e| e.to_string())?;
            }
            "DateTime" => {
                self.buf
                    .write_u64::<LittleEndian>(val.as_u64().unwrap_or(0))
                    .map_err(|e| e.to_string())?;
            }
            "Guid" => {
                write_uuid(
                    &mut self.buf,
                    val.as_str()
                        .unwrap_or("00000000-0000-0000-0000-000000000000"),
                )?;
            }
            "LinearColor" => {
                self.buf
                    .write_f32::<LittleEndian>(val["r"].as_f64().unwrap_or(0.0) as f32)
                    .map_err(|e| e.to_string())?;
                self.buf
                    .write_f32::<LittleEndian>(val["g"].as_f64().unwrap_or(0.0) as f32)
                    .map_err(|e| e.to_string())?;
                self.buf
                    .write_f32::<LittleEndian>(val["b"].as_f64().unwrap_or(0.0) as f32)
                    .map_err(|e| e.to_string())?;
                self.buf
                    .write_f32::<LittleEndian>(val["a"].as_f64().unwrap_or(0.0) as f32)
                    .map_err(|e| e.to_string())?;
            }
            _ => {
                // Generic struct — write nested properties
                if let Some(obj) = val.as_object() {
                    self.write_properties(obj)?;
                }
            }
        }
        Ok(())
    }

    fn write_array_value(&mut self, array_type: &str, val: &Value) -> Result<(), String> {
        if array_type == "StructProperty" {
            // Struct array has complex header
            let values = val["values"].as_array().unwrap_or_else(|| &EMPTY_VEC);
            let count = values.len() as u32;
            self.buf
                .write_u32::<LittleEndian>(count)
                .map_err(|e| e.to_string())?;

            write_fstring(&mut self.buf, val["prop_name"].as_str().unwrap_or(""))?;
            write_fstring(&mut self.buf, val["prop_type"].as_str().unwrap_or("StructProperty"))?;

            let type_name = val["type_name"].as_str().unwrap_or("");

            // Write elements to temp buffer to get total_size
            let mut elem_buf = GvasWriter::new();
            for elem in values {
                elem_buf.write_struct_value(type_name, elem)?;
            }
            let element_data = elem_buf.buf;

            self.buf
                .write_u64::<LittleEndian>(element_data.len() as u64)
                .map_err(|e| e.to_string())?;
            write_fstring(&mut self.buf, type_name)?;
            write_uuid(
                &mut self.buf,
                val["id"]
                    .as_str()
                    .unwrap_or("00000000-0000-0000-0000-000000000000"),
            )?;
            self.buf.push(0); // padding byte
            self.buf.extend_from_slice(&element_data);
            return Ok(());
        }

        let values = val["values"].as_array();
        match values {
            Some(arr) => {
                self.buf
                    .write_u32::<LittleEndian>(arr.len() as u32)
                    .map_err(|e| e.to_string())?;
                match array_type {
                    "EnumProperty" | "NameProperty" | "StrProperty" | "ObjectProperty" => {
                        for v in arr {
                            write_fstring(&mut self.buf, v.as_str().unwrap_or(""))?;
                        }
                    }
                    "Guid" => {
                        for v in arr {
                            write_uuid(
                                &mut self.buf,
                                v.as_str().unwrap_or("00000000-0000-0000-0000-000000000000"),
                            )?;
                        }
                    }
                    "SoftObjectProperty" => {
                        for v in arr {
                            write_fstring(&mut self.buf, v["path"].as_str().unwrap_or(""))?;
                            write_fstring(&mut self.buf, v["sub_path"].as_str().unwrap_or(""))?;
                        }
                    }
                    "ByteProperty" => {
                        // Check if it's a raw byte array (stored as integers)
                        for v in arr {
                            self.buf.push(v.as_u64().unwrap_or(0) as u8);
                        }
                    }
                    "IntProperty" => {
                        for v in arr {
                            self.buf
                                .write_i32::<LittleEndian>(v.as_i64().unwrap_or(0) as i32)
                                .map_err(|e| e.to_string())?;
                        }
                    }
                    "UInt32Property" => {
                        for v in arr {
                            self.buf
                                .write_u32::<LittleEndian>(v.as_u64().unwrap_or(0) as u32)
                                .map_err(|e| e.to_string())?;
                        }
                    }
                    "Int64Property" => {
                        for v in arr {
                            self.buf
                                .write_i64::<LittleEndian>(v.as_i64().unwrap_or(0))
                                .map_err(|e| e.to_string())?;
                        }
                    }
                    "UInt64Property" => {
                        for v in arr {
                            self.buf
                                .write_u64::<LittleEndian>(v.as_u64().unwrap_or(0))
                                .map_err(|e| e.to_string())?;
                        }
                    }
                    "FloatProperty" => {
                        for v in arr {
                            self.buf
                                .write_f32::<LittleEndian>(v.as_f64().unwrap_or(0.0) as f32)
                                .map_err(|e| e.to_string())?;
                        }
                    }
                    "BoolProperty" => {
                        for v in arr {
                            self.buf.push(if v.as_bool().unwrap_or(false) { 1 } else { 0 });
                        }
                    }
                    _ => {
                        // Raw data stored as base64
                        if val.get("raw").is_some() {
                            if let Some(b64) = val["values"].as_str() {
                                let raw = base64_decode(b64)?;
                                self.buf.extend_from_slice(&raw);
                            }
                        }
                    }
                }
            }
            None => {
                // Could be byte array stored directly
                if let Some(b64) = val["values"].as_str() {
                    // Base64 encoded raw data
                    let raw = base64_decode(b64)?;
                    self.buf
                        .write_u32::<LittleEndian>(raw.len() as u32)
                        .map_err(|e| e.to_string())?;
                    self.buf.extend_from_slice(&raw);
                } else {
                    self.buf.write_u32::<LittleEndian>(0).map_err(|e| e.to_string())?;
                }
            }
        }
        Ok(())
    }

    fn write_map_property_body_sized(&mut self, val: &Value) -> Result<usize, String> {
        let key_type = val["key_type"].as_str().unwrap_or("");
        let value_type = val["value_type"].as_str().unwrap_or("");
        write_fstring(&mut self.buf, key_type)?;
        write_fstring(&mut self.buf, value_type)?;
        write_optional_uuid(&mut self.buf, &val["id"])?;
        let start = self.buf.len();
        self.buf.write_u32::<LittleEndian>(0).map_err(|e| e.to_string())?; // unknown
        let entries = val["value"].as_array().unwrap_or_else(|| &EMPTY_VEC);
        self.buf
            .write_u32::<LittleEndian>(entries.len() as u32)
            .map_err(|e| e.to_string())?;

        let key_struct = val["key_struct_type"].as_str().unwrap_or("Guid");
        let val_struct = val["value_struct_type"].as_str().unwrap_or("StructProperty");

        for entry in entries {
            self.write_map_single_value(key_type, key_struct, &entry["key"])?;
            self.write_map_single_value(value_type, val_struct, &entry["value"])?;
        }
        Ok(self.buf.len() - start)
    }

    fn write_map_single_value(
        &mut self,
        type_name: &str,
        struct_hint: &str,
        val: &Value,
    ) -> Result<(), String> {
        match type_name {
            "StructProperty" => self.write_struct_value(struct_hint, val),
            "EnumProperty" | "NameProperty" | "StrProperty" | "ObjectProperty" => {
                write_fstring(&mut self.buf, val.as_str().unwrap_or(""))
            }
            "IntProperty" => self
                .buf
                .write_i32::<LittleEndian>(val.as_i64().unwrap_or(0) as i32)
                .map_err(|e| e.to_string()),
            "Int64Property" => self
                .buf
                .write_i64::<LittleEndian>(val.as_i64().unwrap_or(0))
                .map_err(|e| e.to_string()),
            "UInt32Property" => self
                .buf
                .write_u32::<LittleEndian>(val.as_u64().unwrap_or(0) as u32)
                .map_err(|e| e.to_string()),
            "BoolProperty" => {
                self.buf
                    .push(if val.as_bool().unwrap_or(false) { 1 } else { 0 });
                Ok(())
            }
            "Guid" => write_uuid(
                &mut self.buf,
                val.as_str()
                    .unwrap_or("00000000-0000-0000-0000-000000000000"),
            ),
            _ => {
                if let Some(obj) = val.as_object() {
                    self.write_properties(obj)
                } else {
                    Ok(())
                }
            }
        }
    }

    // ── Custom: GroupSaveDataMap writer ──

    fn write_group_map_property_sized(&mut self, val: &Value) -> Result<usize, String> {
        // Re-encode group RawData back to bytes, then write as regular MapProperty
        let mut map_val = val.clone();

        if let Some(entries) = map_val.get_mut("value").and_then(|v| v.as_array_mut()) {
            for entry in entries.iter_mut() {
                let group_type = entry
                    .pointer("/value/GroupType/value/value")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                if let Some(raw_val) = entry.pointer("/value/RawData/value").cloned() {
                    if raw_val.is_object() && raw_val.get("group_id").is_some() {
                        // This is decoded group rawdata — re-encode to bytes
                        if let Ok(bytes) = encode_group_rawdata(&raw_val, &group_type) {
                            let byte_arr: Vec<Value> = bytes.iter().map(|&b| json!(b)).collect();
                            if let Some(rd) = entry.pointer_mut("/value/RawData/value") {
                                *rd = json!({"values": byte_arr});
                            }
                        }
                    }
                }
            }
        }

        // Now write as regular MapProperty
        self.write_map_property_body_sized(&map_val)
    }
}

// ── Group rawdata encoder ──

fn encode_group_rawdata(val: &Value, group_type: &str) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();

    write_uuid(&mut buf, val["group_id"].as_str().unwrap_or("00000000-0000-0000-0000-000000000000"))?;
    write_fstring(&mut buf, val["group_name"].as_str().unwrap_or(""))?;

    let handles = val["individual_character_handle_ids"]
        .as_array()
        .unwrap_or_else(|| &EMPTY_VEC);
    buf.write_u32::<LittleEndian>(handles.len() as u32)
        .map_err(|e| e.to_string())?;
    for h in handles {
        write_uuid(
            &mut buf,
            h["guid"]
                .as_str()
                .unwrap_or("00000000-0000-0000-0000-000000000000"),
        )?;
        write_uuid(
            &mut buf,
            h["instance_id"]
                .as_str()
                .unwrap_or("00000000-0000-0000-0000-000000000000"),
        )?;
    }

    let is_guild = group_type == "EPalGroupType::Guild";
    let is_indep = group_type == "EPalGroupType::IndependentGuild";
    let is_org = group_type == "EPalGroupType::Organization";

    if is_guild || is_indep || is_org {
        buf.push(val["org_type"].as_u64().unwrap_or(0) as u8);
    }

    if is_org {
        let trail = val["trailing_bytes"].as_array().unwrap_or_else(|| &EMPTY_VEC);
        for b in trail {
            buf.push(b.as_u64().unwrap_or(0) as u8);
        }
        return Ok(buf);
    }

    if is_guild {
        let leading = val["leading_bytes"].as_array().unwrap_or_else(|| &EMPTY_VEC);
        for b in leading {
            buf.push(b.as_u64().unwrap_or(0) as u8);
        }

        let base_ids = val["base_ids"].as_array().unwrap_or_else(|| &EMPTY_VEC);
        buf.write_u32::<LittleEndian>(base_ids.len() as u32)
            .map_err(|e| e.to_string())?;
        for id in base_ids {
            write_uuid(
                &mut buf,
                id.as_str()
                    .unwrap_or("00000000-0000-0000-0000-000000000000"),
            )?;
        }

        buf.write_i32::<LittleEndian>(val["unknown_1"].as_i64().unwrap_or(0) as i32)
            .map_err(|e| e.to_string())?;
        buf.write_i32::<LittleEndian>(val["base_camp_level"].as_i64().unwrap_or(0) as i32)
            .map_err(|e| e.to_string())?;

        let moibc = val["map_object_instance_ids_base_camp_points"]
            .as_array()
            .unwrap_or_else(|| &EMPTY_VEC);
        buf.write_u32::<LittleEndian>(moibc.len() as u32)
            .map_err(|e| e.to_string())?;
        for id in moibc {
            write_uuid(
                &mut buf,
                id.as_str()
                    .unwrap_or("00000000-0000-0000-0000-000000000000"),
            )?;
        }

        write_fstring(&mut buf, val["guild_name"].as_str().unwrap_or(""))?;
        write_uuid(
            &mut buf,
            val["last_guild_name_modifier_player_uid"]
                .as_str()
                .unwrap_or("00000000-0000-0000-0000-000000000000"),
        )?;

        let unk2 = val["unknown_2"].as_array().unwrap_or_else(|| &EMPTY_VEC);
        for b in unk2 {
            buf.push(b.as_u64().unwrap_or(0) as u8);
        }

        write_uuid(
            &mut buf,
            val["admin_player_uid"]
                .as_str()
                .unwrap_or("00000000-0000-0000-0000-000000000000"),
        )?;

        let players = val["players"].as_array().unwrap_or_else(|| &EMPTY_VEC);
        buf.write_u32::<LittleEndian>(players.len() as u32)
            .map_err(|e| e.to_string())?;
        for p in players {
            write_uuid(
                &mut buf,
                p["player_uid"]
                    .as_str()
                    .unwrap_or("00000000-0000-0000-0000-000000000000"),
            )?;
            buf.write_i64::<LittleEndian>(
                p["player_info"]["last_online_real_time"]
                    .as_i64()
                    .unwrap_or(0),
            )
            .map_err(|e| e.to_string())?;
            write_fstring(
                &mut buf,
                p["player_info"]["player_name"].as_str().unwrap_or(""),
            )?;
        }

        let trail = val["trailing_bytes"].as_array().unwrap_or_else(|| &EMPTY_VEC);
        for b in trail {
            buf.push(b.as_u64().unwrap_or(0) as u8);
        }
    }

    if is_indep {
        buf.write_i32::<LittleEndian>(val["base_camp_level"].as_i64().unwrap_or(0) as i32)
            .map_err(|e| e.to_string())?;

        let moibc = val["map_object_instance_ids_base_camp_points"]
            .as_array()
            .unwrap_or_else(|| &EMPTY_VEC);
        buf.write_u32::<LittleEndian>(moibc.len() as u32)
            .map_err(|e| e.to_string())?;
        for id in moibc {
            write_uuid(
                &mut buf,
                id.as_str()
                    .unwrap_or("00000000-0000-0000-0000-000000000000"),
            )?;
        }

        write_fstring(&mut buf, val["guild_name"].as_str().unwrap_or(""))?;
        write_uuid(
            &mut buf,
            val["player_uid"]
                .as_str()
                .unwrap_or("00000000-0000-0000-0000-000000000000"),
        )?;
        write_fstring(&mut buf, val["guild_name_2"].as_str().unwrap_or(""))?;

        buf.write_i64::<LittleEndian>(
            val["player_info"]["last_online_real_time"]
                .as_i64()
                .unwrap_or(0),
        )
        .map_err(|e| e.to_string())?;
        write_fstring(
            &mut buf,
            val["player_info"]["player_name"].as_str().unwrap_or(""),
        )?;
    }

    Ok(buf)
}

// ── Character rawdata encoder ──

fn encode_character_rawdata(val: &Value) -> Result<Vec<u8>, String> {
    let sp_value = &val["object"]["SaveParameter"]["value"];

    let mut writer = GvasWriter::new();
    if let Some(obj) = sp_value.as_object() {
        writer.write_properties(obj)?;
    }

    let unknown = val["unknown_bytes"].as_array().unwrap_or_else(|| &EMPTY_VEC);
    for b in unknown {
        writer.buf.push(b.as_u64().unwrap_or(0) as u8);
    }

    write_uuid(
        &mut writer.buf,
        val["group_id"]
            .as_str()
            .unwrap_or("00000000-0000-0000-0000-000000000000"),
    )?;

    let trail = val["trailing_bytes"].as_array().unwrap_or_else(|| &EMPTY_VEC);
    for b in trail {
        writer.buf.push(b.as_u64().unwrap_or(0) as u8);
    }

    Ok(writer.buf)
}

// ── Public API ──────────────────────────────────────────

/// Parse a `.sav` file into a JSON-compatible structure.
pub fn sav_to_json(data: &[u8]) -> Result<(Value, u8), String> {
    let (gvas, save_type) = decompress_sav(data)?;
    let mut reader = GvasReader::new(&gvas);
    let header = reader.read_header()?;
    let properties = reader.read_properties("")?;
    let trailer = reader.read_trailer()?;

    Ok((
        json!({
            "header": header,
            "properties": Value::Object(properties),
            "trailer": base64_encode(&trailer),
        }),
        save_type,
    ))
}

/// Serialize a JSON structure back to `.sav` binary format.
pub fn json_to_sav(json: &Value, save_type: u8) -> Result<Vec<u8>, String> {
    let mut writer = GvasWriter::new();
    writer.write_header(&json["header"])?;
    let props = json["properties"]
        .as_object()
        .ok_or("properties must be object")?;
    writer.write_properties(props)?;
    // Trailer
    let trailer = base64_decode(json["trailer"].as_str().unwrap_or("AAAAAA=="))?;
    writer.buf.extend_from_slice(&trailer);
    compress_sav(&writer.buf, save_type)
}

// ── Deep UID swap ───────────────────────────────────────

/// Recursively walk the JSON tree and swap every occurrence of `old_uid` ↔ `new_uid`
/// in ownership-related fields.
pub fn deep_swap_uids(data: &mut Value, old_uid: &str, new_uid: &str) {
    let swap_keys: HashSet<&str> = [
        "OwnerPlayerUId",
        "owner_player_uid",
        "build_player_uid",
        "private_lock_player_uid",
    ]
    .into_iter()
    .collect();

    deep_swap_recursive(data, old_uid, new_uid, &swap_keys);
}

fn deep_swap_recursive(data: &mut Value, old_uid: &str, new_uid: &str, keys: &HashSet<&str>) {
    match data {
        Value::Object(map) => {
            for key in keys.iter() {
                if let Some(v) = map.get_mut(*key) {
                    // Could be {"value": "uuid"} (StructProperty) or just "uuid" (string)
                    if let Some(inner) = v.as_object_mut() {
                        if let Some(val_str) = inner.get("value").and_then(|s| s.as_str()) {
                            if val_str == old_uid {
                                inner.insert("value".to_string(), json!(new_uid));
                            } else if val_str == new_uid {
                                inner.insert("value".to_string(), json!(old_uid));
                            }
                        }
                    } else if let Some(s) = v.as_str() {
                        if s == old_uid {
                            *v = json!(new_uid);
                        } else if s == new_uid {
                            *v = json!(old_uid);
                        }
                    }
                }
            }
            for (_, v) in map.iter_mut() {
                deep_swap_recursive(v, old_uid, new_uid, keys);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                deep_swap_recursive(v, old_uid, new_uid, keys);
            }
        }
        _ => {}
    }
}

/// Extract value with nested .value lookups (like PalworldSaveTools' extract_value).
#[allow(dead_code)]
pub fn extract_value(data: &Value, key: &str) -> Option<Value> {
    let mut v = data.get(key)?;
    // Drill into {"value": ...} wrappers
    while let Some(inner) = v.as_object().and_then(|o| o.get("value")) {
        v = inner;
    }
    Some(v.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decompress_level_sav() {
        let sav_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent().unwrap()
            .join("examples").join("json example").join("Level.sav");
        if !sav_path.exists() {
            eprintln!("Skipping: {:?} not found", sav_path);
            return;
        }
        let data = std::fs::read(&sav_path).expect("read Level.sav");
        match decompress_sav(&data) {
            Ok((gvas, save_type)) => {
                assert_eq!(save_type, 0x31, "Expected save_type 0x31 (PLM/Oodle)");
                assert!(gvas.len() >= 4, "GVAS too small");
                assert_eq!(&gvas[..4], &[0x47, 0x56, 0x41, 0x53], "GVAS magic mismatch");
                eprintln!("Decompressed Level.sav: {} bytes", gvas.len());
            }
            Err(e) if e.contains("oo2core") || e.contains("Oodle") => {
                eprintln!("Skipping: Oodle DLL not available ({e})");
            }
            Err(e) => panic!("decompress_sav failed: {e}"),
        }
    }

    #[test]
    fn test_parse_level_sav_to_json() {
        let sav_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent().unwrap()
            .join("examples").join("json example").join("Level.sav");
        if !sav_path.exists() {
            eprintln!("Skipping: {:?} not found", sav_path);
            return;
        }
        let data = std::fs::read(&sav_path).expect("read Level.sav");
        match sav_to_json(&data) {
            Ok((json, save_type)) => {
                assert_eq!(save_type, 0x31);
                let props = json.get("properties").expect("no properties in JSON");
                let wsd = props.get("worldSaveData").expect("no worldSaveData");
                let wsd_val = wsd.get("value").expect("no value in worldSaveData");
                assert!(wsd_val.get("CharacterSaveParameterMap").is_some(),
                    "Missing CharacterSaveParameterMap");
                assert!(wsd_val.get("GroupSaveDataMap").is_some(),
                    "Missing GroupSaveDataMap");
                eprintln!("sav_to_json succeeded, save_type=0x{:02X}", save_type);
            }
            Err(e) if e.contains("oo2core") || e.contains("Oodle") => {
                eprintln!("Skipping: Oodle DLL not available ({e})");
            }
            Err(e) => panic!("sav_to_json failed: {e}"),
        }
    }

    #[test]
    fn test_roundtrip_level_sav() {
        let sav_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent().unwrap()
            .join("examples").join("json example").join("Level.sav");
        if !sav_path.exists() {
            eprintln!("Skipping: {:?} not found", sav_path);
            return;
        }
        let data = std::fs::read(&sav_path).expect("read Level.sav");
        let (json, save_type) = sav_to_json(&data).expect("sav_to_json");
        eprintln!("Parsed OK, now writing back...");
        let sav_bytes = json_to_sav(&json, save_type).expect("json_to_sav");
        eprintln!("Written {} bytes, now re-parsing...", sav_bytes.len());
        let (json2, _save_type2) = sav_to_json(&sav_bytes).expect("re-parse failed");
        let wsd2 = json2.pointer("/properties/worldSaveData/value").expect("no worldSaveData on re-parse");
        assert!(wsd2.get("CharacterSaveParameterMap").is_some());
        assert!(wsd2.get("GroupSaveDataMap").is_some());
        eprintln!("Round-trip OK!");
    }

    #[test]
    fn test_plz_roundtrip() {
        // Test that compress→decompress roundtrips for PLZ
        let original = b"GVAS\x00\x00\x00\x00test data for roundtrip";
        let compressed = compress_sav(original, 0x32).expect("compress_sav PLZ");
        let (decompressed, st) = decompress_sav(&compressed).expect("decompress_sav PLZ");
        assert_eq!(st, 0x32);
        assert_eq!(&decompressed, original);
    }
}
