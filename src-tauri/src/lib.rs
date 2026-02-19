mod gvas;
mod oodle;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use tauri::{AppHandle, Emitter, Manager};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;

/// The host slot UUID in Palworld co-op, formatted for file names.
/// FGuid{1,0,0,0} → "00000001000000000000000000000000"
const DEFAULT_HOST_ID: &str = "00000001000000000000000000000000";
/// Legacy host ID format (some older saves may use this).
const LEGACY_HOST_ID: &str = "00000000000000000000000000000001";
/// Name of the per-world config file stored inside each world's Players folder.
/// Travels with the world files when shared between users.
const WORLD_CONFIG_FILE: &str = "host_switcher.json";

// ── Data structures ──────────────────────────────────────

/// Per-world configuration stored *inside* the world folder.
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[serde(default)]
struct WorldConfig {
  host_id: Option<String>,
  /// Display names: slot-id → friendly name
  players: HashMap<String, String>,
  /// Original identities: slot-id → original player id (tracks swaps)
  original_names: HashMap<String, String>,
  /// Custom display name for this world (shown in the app UI)
  display_name: Option<String>,
}

/// Lightweight global config (app data dir) – just remembers last session.
#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(default)]
struct AppConfig {
  account_id: Option<String>,
  world_id: Option<String>,
  // ── Legacy fields for migration only ──
  #[serde(default, skip_serializing_if = "Option::is_none")]
  host_id: Option<String>,
  #[serde(default, skip_serializing_if = "HashMap::is_empty")]
  players: HashMap<String, String>,
  #[serde(default, skip_serializing_if = "HashMap::is_empty")]
  original_names: HashMap<String, String>,
  #[serde(default, skip_serializing_if = "HashMap::is_empty")]
  worlds: HashMap<String, WorldConfig>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Player {
  id: String,
  name: String,
  original_id: String,
  is_host: bool,
  level: u32,
  pals_count: usize,
  last_online: String,
  guild_name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorldInfo {
  id: String,
  player_count: usize,
  display_name: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ValidatedFolder {
  name: String,
  path: String,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProgressPayload {
  percent: f64,
  message: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
struct BackupSnapshot {
  host_id: Option<String>,
  players: HashMap<String, String>,
  original_names: HashMap<String, String>,
  display_name: Option<String>,
}

impl Default for BackupSnapshot {
  fn default() -> Self {
    Self {
      host_id: None,
      players: HashMap::new(),
      original_names: HashMap::new(),
      display_name: None,
    }
  }
}

fn normalize_id(value: &str) -> String {
  value.trim().to_ascii_lowercase()
}

fn home_dir() -> Result<PathBuf, String> {
  if let Ok(profile) = std::env::var("USERPROFILE") {
    return Ok(PathBuf::from(profile));
  }
  if let Ok(home) = std::env::var("HOME") {
    return Ok(PathBuf::from(home));
  }
  Err("Cannot find home directory.".to_string())
}

fn save_games_root() -> Result<PathBuf, String> {
  let home = home_dir()?;
  Ok(
    home
      .join("AppData")
      .join("Local")
      .join("Pal")
      .join("Saved")
      .join("SaveGames"),
  )
}

fn players_dir(account_id: &str, world_id: &str) -> Result<PathBuf, String> {
  Ok(
    save_games_root()?
      .join(account_id)
      .join(world_id)
      .join("Players"),
  )
}

fn world_dir(account_id: &str, world_id: &str) -> Result<PathBuf, String> {
  Ok(save_games_root()?.join(account_id).join(world_id))
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
  let dir = app
    .path()
    .app_data_dir()
    .map_err(|err| err.to_string())?
    .join("palworld-host-switcher");
  fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
  Ok(dir.join("config.json"))
}

fn load_app_config(app: &AppHandle) -> Result<AppConfig, String> {
  let path = config_path(app)?;
  if !path.exists() {
    return Ok(AppConfig::default());
  }
  let raw = fs::read_to_string(&path).map_err(|err| err.to_string())?;
  serde_json::from_str(&raw).map_err(|err| err.to_string())
}

fn save_app_config(app: &AppHandle, config: &AppConfig) -> Result<(), String> {
  let path = config_path(app)?;
  let raw = serde_json::to_string_pretty(config).map_err(|err| err.to_string())?;
  fs::write(path, raw).map_err(|err| err.to_string())
}

// ── Per-world config (stored in the world's Players folder) ──

fn world_config_path(pdir: &Path) -> PathBuf {
  pdir.join(WORLD_CONFIG_FILE)
}

fn load_world_config(pdir: &Path) -> WorldConfig {
  let path = world_config_path(pdir);
  if !path.exists() {
    return WorldConfig::default();
  }
  match fs::read_to_string(&path) {
    Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
    Err(_) => WorldConfig::default(),
  }
}

fn save_world_config(pdir: &Path, wc: &WorldConfig) -> Result<(), String> {
  // Ensure directory exists (it should, but be safe)
  if !pdir.exists() {
    fs::create_dir_all(pdir).map_err(|err| err.to_string())?;
  }
  let path = world_config_path(pdir);
  let raw = serde_json::to_string_pretty(wc).map_err(|err| err.to_string())?;
  fs::write(path, raw).map_err(|err| err.to_string())
}

/// Prune stale player entries from WorldConfig that no longer have .sav files.
#[allow(dead_code)]
fn prune_world_config(wc: &mut WorldConfig, live_ids: &[String]) {
  wc.players.retain(|id, _| live_ids.contains(id));
  wc.original_names.retain(|id, _| live_ids.contains(id));
}

// ── Migration: move old app-level configs into world folders ──

fn migrate_legacy_config(app: &AppHandle) -> Result<(), String> {
  let mut config = load_app_config(app)?;
  let mut migrated = false;

  // 1. Migrate flat legacy fields (very old format)
  if !config.players.is_empty() || !config.original_names.is_empty() || config.host_id.is_some() {
    if let (Some(aid), Some(wid)) = (config.account_id.clone(), config.world_id.clone()) {
      if let Ok(pdir) = players_dir(&aid, &wid) {
        if pdir.exists() {
          let mut wc = load_world_config(&pdir);
          // Only migrate if the world config is empty (don't overwrite)
          if wc.players.is_empty() {
            wc.players = std::mem::take(&mut config.players);
          } else {
            config.players.clear();
          }
          if wc.original_names.is_empty() {
            wc.original_names = std::mem::take(&mut config.original_names);
          } else {
            config.original_names.clear();
          }
          if wc.host_id.is_none() {
            wc.host_id = config.host_id.take();
          } else {
            config.host_id = None;
          }
          let _ = save_world_config(&pdir, &wc);
          migrated = true;
        }
      }
    }
  }

  // 2. Migrate per-world map entries (previous session format)
  if !config.worlds.is_empty() {
    for (key, wc_old) in std::mem::take(&mut config.worlds) {
      // key format is "accountId/worldId"
      let parts: Vec<&str> = key.splitn(2, '/').collect();
      if parts.len() == 2 {
        if let Ok(pdir) = players_dir(parts[0], parts[1]) {
          if pdir.exists() {
            let mut wc = load_world_config(&pdir);
            // Merge: only fill in missing data
            if wc.players.is_empty() {
              wc.players = wc_old.players;
            }
            if wc.original_names.is_empty() {
              wc.original_names = wc_old.original_names;
            }
            if wc.host_id.is_none() {
              wc.host_id = wc_old.host_id;
            }
            let _ = save_world_config(&pdir, &wc);
          }
        }
      }
    }
    migrated = true;
  }

  if migrated {
    save_app_config(app, &config)?;
  }
  Ok(())
}

fn list_dirs(path: &Path) -> Vec<String> {
  fs::read_dir(path)
    .ok()
    .into_iter()
    .flatten()
    .filter_map(|entry| entry.ok())
    .filter(|entry| entry.file_type().map(|file| file.is_dir()).unwrap_or(false))
    .filter_map(|entry| entry.file_name().into_string().ok())
    .collect()
}

fn is_hex_id(value: &str) -> bool {
  value.len() == 32 && value.chars().all(|c| c.is_ascii_hexdigit())
}

/// Convert a GVAS UUID (with dashes) to a Palworld .sav filename (flat hex).
fn uuid_to_filename(uuid: &str) -> String {
  uuid.replace('-', "").to_ascii_lowercase()
}

/// Convert a flat-hex filename to a GVAS UUID (with dashes).
fn filename_to_uuid(filename: &str) -> String {
  let s = filename.to_ascii_lowercase();
  if s.len() != 32 {
    return s;
  }
  format!(
    "{}-{}-{}-{}-{}",
    &s[0..8],
    &s[8..12],
    &s[12..16],
    &s[16..20],
    &s[20..32]
  )
}

/// Check if a player ID (flat hex) is the host slot.
#[allow(dead_code)]
fn is_host_slot(id: &str) -> bool {
  let n = normalize_id(id);
  n == DEFAULT_HOST_ID || n == LEGACY_HOST_ID
}

fn list_player_ids(players_dir: &Path) -> Vec<String> {
  fs::read_dir(players_dir)
    .ok()
    .into_iter()
    .flatten()
    .filter_map(|entry| entry.ok())
    .filter(|entry| entry.file_type().map(|file| file.is_file()).unwrap_or(false))
    .filter_map(|entry| entry.file_name().into_string().ok())
    .filter_map(|name| name.strip_suffix(".sav").map(|id| id.to_string()))
    .map(|id| normalize_id(&id))
    .filter(|id| is_hex_id(id))
    .collect()
}

fn resolve_host_id(_wc: &WorldConfig, player_ids: &[String]) -> Option<String> {
  // Host is always the player in the well-known slot 0001.
  for &hid in &[DEFAULT_HOST_ID, LEGACY_HOST_ID] {
    let normalized = normalize_id(hid);
    if player_ids.contains(&normalized) {
      return Some(normalized);
    }
  }
  player_ids.first().cloned()
}

// ── Level.sav player extraction ──────────────────────────

/// Information extracted from Level.sav about a single player.
#[allow(dead_code)]
struct LevelPlayerInfo {
  uuid: String,      // GVAS UUID with dashes
  filename: String,   // flat hex for .sav filename
  name: String,
  level: u32,
  pals_count: usize,
  last_online: String,
  guild_name: String,
}

/// Read Level.sav and extract player info (name, level, pals, etc.).
fn extract_players_from_level(world_path: &Path) -> Result<Vec<LevelPlayerInfo>, String> {
  let level_sav = world_path.join("Level.sav");
  if !level_sav.exists() {
    return Err("Level.sav not found.".into());
  }
  let data = fs::read(&level_sav).map_err(|e| format!("Cannot read Level.sav: {e}"))?;
  let (json, _save_type) = gvas::sav_to_json(&data)?;

  let world_data = &json["properties"]["worldSaveData"]["value"];

  // ── 1. Extract guild info from GroupSaveDataMap ──
  // Maps: player_uuid → (player_name, last_online_ticks, guild_name)
  let mut guild_info: HashMap<String, (String, i64, String)> = HashMap::new();

  if let Some(gsm) = world_data.get("GroupSaveDataMap") {
    if let Some(entries) = gsm.get("value").and_then(|v| v.as_array()) {
      for entry in entries {
        let group_type = entry
          .pointer("/value/GroupType/value/value")
          .and_then(|v| v.as_str())
          .unwrap_or("");
        if group_type != "EPalGroupType::Guild" {
          continue;
        }
        let raw_data = entry.pointer("/value/RawData/value");
        if raw_data.is_none() {
          continue;
        }
        let rd = raw_data.unwrap();
        let g_name = rd["guild_name"].as_str().unwrap_or("").to_string();
        if let Some(players) = rd["players"].as_array() {
          for p in players {
            let puid = p["player_uid"].as_str().unwrap_or("").to_string();
            let last_online = p["player_info"]["last_online_real_time"]
              .as_i64()
              .unwrap_or(0);
            let pname = p["player_info"]["player_name"]
              .as_str()
              .unwrap_or("")
              .to_string();
            if !puid.is_empty() {
              guild_info.insert(puid, (pname, last_online, g_name.clone()));
            }
          }
        }
      }
    }
  }

  // ── 2. Extract character info from CharacterSaveParameterMap ──
  // Maps: player_uuid → level, counts pals per owner
  let mut player_levels: HashMap<String, u32> = HashMap::new();
  let mut player_names_cspm: HashMap<String, String> = HashMap::new();
  let mut pals_count: HashMap<String, usize> = HashMap::new();

  if let Some(cspm) = world_data.get("CharacterSaveParameterMap") {
    if let Some(entries) = cspm.get("value").and_then(|v| v.as_array()) {
      for entry in entries {
        // Key has PlayerUId and InstanceId
        let player_uid = entry
          .pointer("/key/PlayerUId/value")
          .and_then(|v| v.as_str())
          .unwrap_or("")
          .to_string();

        // Decoded RawData for the character
        let raw_data = entry.pointer("/value/RawData");
        if raw_data.is_none() {
          continue;
        }
        let rd = raw_data.unwrap();
        let save_param = &rd["value"]["object"]["SaveParameter"]["value"];

        let is_player = save_param
          .get("IsPlayer")
          .and_then(|v| v.get("value"))
          .and_then(|v| v.as_bool())
          .unwrap_or(false);

        if is_player {
          // Level is a ByteProperty: {"value": {"type":"None","value":55}}
          let level = save_param
            .get("Level")
            .and_then(|v| v.get("value"))
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;
          let nick = save_param
            .get("NickName")
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
          player_levels.insert(player_uid.clone(), level);
          if !nick.is_empty() {
            player_names_cspm.insert(player_uid, nick);
          }
        } else {
          // This is a pal – count under owner
          let owner = save_param
            .get("OwnerPlayerUId")
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
          if !owner.is_empty() && owner != "00000000-0000-0000-0000-000000000000" {
            *pals_count.entry(owner.to_string()).or_insert(0) += 1;
          }
        }
      }
    }
  }

  // ── 3. Get current game time for "last seen" calculation ──
  let current_ticks = world_data
    .pointer("/GameTimeSaveData/value/RealDateTimeTicks/value")
    .and_then(|v| v.as_u64())
    .unwrap_or(0);

  // ── 4. Build player list ──
  // Combine guild_info + cspm data
  let mut all_uuids: Vec<String> = Vec::new();
  for uuid in guild_info.keys() {
    if !all_uuids.contains(uuid) {
      all_uuids.push(uuid.clone());
    }
  }
  for uuid in player_levels.keys() {
    if !all_uuids.contains(uuid) {
      all_uuids.push(uuid.clone());
    }
  }

  let mut result = Vec::new();
  for uuid in &all_uuids {
    let filename = uuid_to_filename(uuid);
    let (guild_name_str, last_online_str, player_name) = if let Some((name, ticks, gname)) = guild_info.get(uuid) {
      let last_seen = format_last_seen(*ticks, current_ticks);
      (gname.clone(), last_seen, name.clone())
    } else {
      ("".to_string(), "Unknown".to_string(), "".to_string())
    };

    let name = if !player_name.is_empty() {
      player_name
    } else if let Some(nick) = player_names_cspm.get(uuid) {
      nick.clone()
    } else {
      filename.clone()
    };

    let level = player_levels.get(uuid).copied().unwrap_or(0);
    let pals = pals_count.get(uuid).copied().unwrap_or(0);

    result.push(LevelPlayerInfo {
      uuid: uuid.clone(),
      filename,
      name,
      level,
      pals_count: pals,
      last_online: last_online_str,
      guild_name: guild_name_str,
    });
  }

  Ok(result)
}

/// Format last_online ticks relative to current game ticks into human-readable text.
fn format_last_seen(last_online_ticks: i64, current_ticks: u64) -> String {
  if last_online_ticks <= 0 {
    return "Unknown".to_string();
  }
  let diff_ticks = current_ticks as i64 - last_online_ticks;
  if diff_ticks < 0 {
    return "Online now".to_string();
  }
  // 1 tick = 100 nanoseconds = 0.0000001 seconds
  let seconds = diff_ticks / 10_000_000;
  if seconds < 60 {
    return "Online now".to_string();
  }
  let minutes = seconds / 60;
  if minutes < 60 {
    return format!("{minutes} min ago");
  }
  let hours = minutes / 60;
  if hours < 24 {
    return format!("{hours}h ago");
  }
  let days = hours / 24;
  format!("{days}d ago")
}

/// Modify a single player .sav file, swapping internal PlayerUId references.
/// Read the InstanceId from a player .sav file (needed for InstanceId-based matching).
fn read_player_instance_id(sav_path: &Path) -> Result<String, String> {
  let data = fs::read(sav_path).map_err(|e| format!("read player sav: {e}"))?;
  let (json, _) = gvas::sav_to_json(&data)?;
  let inst = json
    .pointer("/properties/SaveData/value/IndividualId/value/InstanceId/value")
    .and_then(|v| v.as_str())
    .unwrap_or("")
    .to_string();
  if inst.is_empty() {
    return Err(format!("No InstanceId found in {:?}", sav_path));
  }
  Ok(inst)
}

fn modify_player_sav(sav_path: &Path, old_uid: &str, new_uid: &str) -> Result<(), String> {
  let data = fs::read(sav_path).map_err(|e| format!("read player sav: {e}"))?;
  let (mut json, save_type) = gvas::sav_to_json(&data)?;

  // Update PlayerUId
  if let Some(puid) = json.pointer_mut("/properties/SaveData/value/PlayerUId/value") {
    if puid.as_str() == Some(old_uid) {
      *puid = Value::String(new_uid.to_string());
    }
  }
  // Update IndividualId → PlayerUId
  if let Some(iid) = json.pointer_mut("/properties/SaveData/value/IndividualId/value/PlayerUId/value") {
    if iid.as_str() == Some(old_uid) {
      *iid = Value::String(new_uid.to_string());
    }
  }

  let sav_bytes = gvas::json_to_sav(&json, save_type)?;
  fs::write(sav_path, &sav_bytes).map_err(|e| format!("write player sav: {e}"))?;
  Ok(())
}

fn build_players(
  player_ids: &[String],
  host_id: &str,
  level_info: &[LevelPlayerInfo],
) -> Vec<Player> {
  player_ids
    .iter()
    .map(|id| {
      // Find matching info from Level.sav
      let info = level_info.iter().find(|li| li.filename == *id);
      let name = info.map(|i| i.name.clone()).unwrap_or_else(|| id.clone());
      let level = info.map(|i| i.level).unwrap_or(0);
      let pals_count = info.map(|i| i.pals_count).unwrap_or(0);
      let last_online = info.map(|i| i.last_online.clone()).unwrap_or_default();
      let guild_name = info.map(|i| i.guild_name.clone()).unwrap_or_default();
      Player {
        id: id.clone(),
        name,
        original_id: id.clone(),
        is_host: id == host_id,
        level,
        pals_count,
        last_online,
        guild_name,
      }
    })
    .collect()
}

/// Swap .sav files + modify Level.sav with GVAS-based UID swap.
/// Follows PalworldSaveTools fix_host_save logic:
///   1. Read InstanceIds from both player .sav files
///   2. Patch PlayerUId inside both player .sav files
///   3. In Level.sav CharacterSaveParameterMap: swap PlayerUId only for the
///      two entries matching by InstanceId (not all entries!)
///   4. In Level.sav GroupSaveDataMap: swap admin, player_uid, and
///      individual_character_handle_ids.guid matched by instance_id
///   5. Deep-swap OwnerPlayerUId/build_player_uid/etc across all Level.sav
///   6. Serialize Level.sav and write all files
///   7. Rename .sav files (swap filenames)
///
/// Emits granular swap-progress events when `progress` is provided.
fn swap_players_full(
  world_path: &Path,
  players_dir: &Path,
  first_id: &str,
  second_id: &str,
  progress: Option<(&AppHandle, f64, f64)>, // (app, base%, range%)
) -> Result<(), String> {
  // progress helper: emit (base + fraction * range)
  let emit = |frac: f64, msg: &str| {
    if let Some((app, base, range)) = &progress {
      let _ = app.emit("swap-progress", ProgressPayload {
        percent: base + frac * range,
        message: msg.to_string(),
      });
    }
  };

  let first = normalize_id(first_id);
  let second = normalize_id(second_id);

  let first_sav = players_dir.join(format!("{first}.sav"));
  let second_sav = players_dir.join(format!("{second}.sav"));
  if !first_sav.exists() || !second_sav.exists() {
    return Err("Missing .sav files for swap.".to_string());
  }

  let uuid_first = filename_to_uuid(&first);
  let uuid_second = filename_to_uuid(&second);

  // ── 0. Read InstanceIds from player .sav files (needed for CSPM / guild matching) ──
  emit(0.0, "Reading player saves…");
  let inst_first = read_player_instance_id(&first_sav)?;
  let inst_second = read_player_instance_id(&second_sav)?;

  // ── 1. Modify player .sav files (patch PlayerUId + IndividualId.PlayerUId) ──
  emit(0.05, "Patching player saves…");
  if let Err(e) = modify_player_sav(&first_sav, &uuid_first, &uuid_second) {
    eprintln!("[palhost] warn: could not modify {first}.sav internals: {e}");
  }
  if let Err(e) = modify_player_sav(&second_sav, &uuid_second, &uuid_first) {
    eprintln!("[palhost] warn: could not modify {second}.sav internals: {e}");
  }

  // ── 2. Level.sav: read ──
  emit(0.10, "Reading Level.sav…");
  let level_sav = world_path.join("Level.sav");
  if !level_sav.exists() {
    return Err("Level.sav not found.".into());
  }
  let data = fs::read(&level_sav).map_err(|e| format!("Cannot read Level.sav: {e}"))?;

  // ── 3. Level.sav: parse ──
  emit(0.15, "Parsing Level.sav…");
  let (mut json, save_type) = gvas::sav_to_json(&data)?;

  // ── 4. Level.sav: modify UIDs ──
  emit(0.40, "Swapping UIDs in Level.sav…");
  {
    let world_data = json
      .get_mut("properties")
      .and_then(|p| p.get_mut("worldSaveData"))
      .and_then(|w| w.get_mut("value"))
      .ok_or("Cannot navigate to worldSaveData")?;

    // 4a. CharacterSaveParameterMap: swap PlayerUId ONLY for the two entries
    //     that match by InstanceId (the player's own character entry).
    //     All other entries (pals, other players) are left untouched.
    if let Some(cspm) = world_data.get_mut("CharacterSaveParameterMap") {
      if let Some(entries) = cspm.get_mut("value").and_then(|v| v.as_array_mut()) {
        for entry in entries.iter_mut() {
          if let Some(key) = entry.get_mut("key") {
            let entry_inst = key
              .pointer("/InstanceId/value")
              .and_then(|v| v.as_str())
              .unwrap_or("");
            if entry_inst == inst_first {
              if let Some(puid) = key.pointer_mut("/PlayerUId/value") {
                *puid = Value::String(uuid_second.to_string());
              }
            } else if entry_inst == inst_second {
              if let Some(puid) = key.pointer_mut("/PlayerUId/value") {
                *puid = Value::String(uuid_first.to_string());
              }
            }
          }
        }
      }
    }

    // 4b. GroupSaveDataMap: swap admin_player_uid, player_uid in member list,
    //     and individual_character_handle_ids.guid matched by instance_id.
    if let Some(gsm) = world_data.get_mut("GroupSaveDataMap") {
      if let Some(entries) = gsm.get_mut("value").and_then(|v| v.as_array_mut()) {
        for entry in entries.iter_mut() {
          // Only process guilds
          let is_guild = entry
            .pointer("/value/GroupType/value/value")
            .and_then(|v| v.as_str())
            == Some("EPalGroupType::Guild");
          if !is_guild {
            continue;
          }

          let raw_data = entry.pointer_mut("/value/RawData/value");
          if let Some(rd) = raw_data {
            // Swap admin_player_uid
            if let Some(admin) = rd.get_mut("admin_player_uid") {
              if let Some(s) = admin.as_str().map(|s| s.to_string()) {
                if s == uuid_first {
                  *admin = Value::String(uuid_second.to_string());
                } else if s == uuid_second {
                  *admin = Value::String(uuid_first.to_string());
                }
              }
            }

            // Swap player_uid in players list
            if let Some(players) = rd.get_mut("players").and_then(|p| p.as_array_mut()) {
              for p in players.iter_mut() {
                if let Some(puid) = p.get_mut("player_uid") {
                  if let Some(s) = puid.as_str().map(|s| s.to_string()) {
                    if s == uuid_first {
                      *puid = Value::String(uuid_second.to_string());
                    } else if s == uuid_second {
                      *puid = Value::String(uuid_first.to_string());
                    }
                  }
                }
              }
            }

            // Swap guid in individual_character_handle_ids — matched by instance_id
            if let Some(handles) = rd.get_mut("individual_character_handle_ids").and_then(|h| h.as_array_mut()) {
              for h in handles.iter_mut() {
                let h_inst = h.get("instance_id")
                  .and_then(|v| v.as_str())
                  .unwrap_or("");
                if h_inst == inst_first {
                  if let Some(guid) = h.get_mut("guid") {
                    *guid = Value::String(uuid_second.to_string());
                  }
                } else if h_inst == inst_second {
                  if let Some(guid) = h.get_mut("guid") {
                    *guid = Value::String(uuid_first.to_string());
                  }
                }
              }
            }
          }
        }
      }
    }

    // 4c. Deep-swap ownership UIDs (OwnerPlayerUId, build_player_uid, etc.)
    //     across the entire worldSaveData. This is the same as PalworldSaveTools'
    //     deep_swap() function applied to the full Level.sav.
    gvas::deep_swap_uids(world_data, &uuid_first, &uuid_second);
  }

  // ── 5. Level.sav: serialize ──
  emit(0.50, "Serializing Level.sav…");
  let sav_bytes = gvas::json_to_sav(&json, save_type)?;

  // ── 6. Level.sav: write ──
  emit(0.75, "Writing Level.sav…");
  fs::write(&level_sav, &sav_bytes).map_err(|e| format!("Cannot write Level.sav: {e}"))?;

  // ── 7. Rename .sav files (swap filenames) ──
  emit(0.96, "Renaming files…");
  let stamp = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map_err(|err| err.to_string())?
    .as_millis();
  let temp = players_dir.join(format!("swap-{stamp}.tmp"));
  fs::rename(&first_sav, &temp).map_err(|err| err.to_string())?;
  fs::rename(&second_sav, &first_sav).map_err(|err| err.to_string())?;
  fs::rename(&temp, &second_sav).map_err(|err| err.to_string())?;

  emit(1.0, "Swap complete.");
  Ok(())
}

fn backup_files(players_dir: &Path, world_path: &Path, ids: &[String], snapshot: &BackupSnapshot) -> Result<PathBuf, String> {
  let stamp = chrono::Utc::now().format("%Y-%m-%d_%H-%M-%S").to_string();
  let backup_dir = players_dir.join("backup").join(stamp);
  fs::create_dir_all(&backup_dir).map_err(|err| err.to_string())?;
  for id in ids {
    let src = players_dir.join(format!("{}.sav", normalize_id(id)));
    if src.exists() {
      let dest = backup_dir.join(format!("{}.sav", normalize_id(id)));
      fs::copy(&src, &dest).map_err(|err| err.to_string())?;
    }
  }
  // Backup Level.sav
  let level_sav = world_path.join("Level.sav");
  if level_sav.exists() {
    let dest = backup_dir.join("Level.sav");
    fs::copy(&level_sav, &dest).map_err(|err| err.to_string())?;
  }
  // Save config snapshot with names mapping
  let snapshot_json = serde_json::to_string_pretty(snapshot).map_err(|err| err.to_string())?;
  fs::write(backup_dir.join("config_snapshot.json"), snapshot_json).map_err(|err| err.to_string())?;
  Ok(backup_dir)
}

fn list_backups_dir(players_dir: &Path) -> Vec<String> {
  let backup_root = players_dir.join("backup");
  if !backup_root.exists() {
    return Vec::new();
  }
  let mut items = list_dirs(&backup_root);
  items.sort_by(|a, b| b.cmp(a));
  items
}

#[tauri::command]
fn get_accounts() -> Result<Vec<String>, String> {
  Ok(list_dirs(&save_games_root()?))
}

#[tauri::command]
fn get_worlds(account_id: String) -> Result<Vec<String>, String> {
  Ok(list_dirs(&save_games_root()?.join(account_id)))
}

#[tauri::command]
fn get_worlds_with_counts(account_id: String) -> Result<Vec<WorldInfo>, String> {
  let root = save_games_root()?.join(&account_id);
  let world_ids = list_dirs(&root);
  let result = world_ids
    .into_iter()
    .map(|wid| {
      let pdir = root.join(&wid).join("Players");
      let count = list_player_ids(&pdir).len();
      let wc = load_world_config(&pdir);
      WorldInfo { id: wid, player_count: count, display_name: wc.display_name }
    })
    .collect();
  Ok(result)
}

#[tauri::command]
fn set_world_name(account_id: String, world_id: String, name: String) -> Result<Vec<WorldInfo>, String> {
  let pdir = players_dir(&account_id, &world_id)?;
  let mut wc = load_world_config(&pdir);
  let trimmed = name.trim().to_string();
  if trimmed.is_empty() {
    wc.display_name = None;
  } else {
    wc.display_name = Some(trimmed);
  }
  save_world_config(&pdir, &wc)?;
  get_worlds_with_counts(account_id)
}

#[tauri::command]
fn reset_world_name(account_id: String, world_id: String) -> Result<Vec<WorldInfo>, String> {
  let pdir = players_dir(&account_id, &world_id)?;
  let mut wc = load_world_config(&pdir);
  wc.display_name = None;
  save_world_config(&pdir, &wc)?;
  get_worlds_with_counts(account_id)
}

#[tauri::command]
async fn get_players(app: AppHandle, account_id: String, world_id: String) -> Result<Vec<Player>, String> {
  let a = app.clone();
  tauri::async_runtime::spawn_blocking(move || {
    get_players_sync(&a, &account_id, &world_id)
  })
  .await
  .map_err(|e| format!("Task error: {e}"))?
}

fn get_players_sync(app: &AppHandle, account_id: &str, world_id: &str) -> Result<Vec<Player>, String> {
  let dir = players_dir(account_id, world_id)?;
  let wpath = world_dir(account_id, world_id)?;
  let player_ids = list_player_ids(&dir);
  if player_ids.is_empty() {
    return Ok(Vec::new());
  }
  let wc = load_world_config(&dir);
  let host_id = resolve_host_id(&wc, &player_ids).ok_or("Host not found.")?;

  // Read player info from Level.sav
  let level_info = match extract_players_from_level(&wpath) {
    Ok(info) => info,
    Err(e) => {
      eprintln!("[palhost] Failed to parse Level.sav: {e}");
      Vec::new()
    }
  };

  let players = build_players(&player_ids, &host_id, &level_info);

  // Remember last-used account/world
  let mut ac = load_app_config(app).unwrap_or_default();
  ac.account_id = Some(account_id.to_string());
  ac.world_id = Some(world_id.to_string());
  let _ = save_app_config(app, &ac);

  Ok(players)
}

#[tauri::command]
async fn set_host_player(
  app: AppHandle,
  account_id: String,
  world_id: String,
  player_id: String,
) -> Result<Vec<Player>, String> {
  let a = app.clone();
  tauri::async_runtime::spawn_blocking(move || {
    set_host_player_sync(&a, &account_id, &world_id, &player_id)
  })
  .await
  .map_err(|e| format!("Task error: {e}"))?
}

fn set_host_player_sync(
  app: &AppHandle,
  account_id: &str,
  world_id: &str,
  player_id: &str,
) -> Result<Vec<Player>, String> {
  let dir = players_dir(account_id, world_id)?;
  let wpath = world_dir(account_id, world_id)?;
  let player_ids = list_player_ids(&dir);
  let wc = load_world_config(&dir);
  let host_id = resolve_host_id(&wc, &player_ids).ok_or("Host not found.")?;
  let target_id = normalize_id(player_id);
  if host_id == target_id {
    return get_players_sync(app, account_id, world_id);
  }
  swap_players_full(&wpath, &dir, &host_id, &target_id, Some((app, 0.0, 90.0)))?;
  let _ = app.emit("swap-progress", ProgressPayload { percent: 95.0, message: "Reloading players…".into() });
  get_players_sync(app, account_id, world_id)
}

#[tauri::command]
async fn swap_players(
  app: AppHandle,
  account_id: String,
  world_id: String,
  first_id: String,
  second_id: String,
) -> Result<Vec<Player>, String> {
  let a = app.clone();
  tauri::async_runtime::spawn_blocking(move || {
    swap_players_sync(&a, &account_id, &world_id, &first_id, &second_id)
  })
  .await
  .map_err(|e| format!("Task error: {e}"))?
}

fn swap_players_sync(
  app: &AppHandle,
  account_id: &str,
  world_id: &str,
  first_id: &str,
  second_id: &str,
) -> Result<Vec<Player>, String> {
  let dir = players_dir(account_id, world_id)?;
  let wpath = world_dir(account_id, world_id)?;
  let first = normalize_id(first_id);
  let second = normalize_id(second_id);
  swap_players_full(&wpath, &dir, &first, &second, Some((app, 0.0, 90.0)))?;
  let _ = app.emit("swap-progress", ProgressPayload { percent: 95.0, message: "Reloading players…".into() });
  get_players_sync(app, account_id, world_id)
}



#[tauri::command]
fn create_backup(
  _app: AppHandle,
  account_id: String,
  world_id: String,
  player_ids: Vec<String>,
) -> Result<String, String> {
  let dir = players_dir(&account_id, &world_id)?;
  let wpath = world_dir(&account_id, &world_id)?;
  let wc = load_world_config(&dir);
  let snapshot = BackupSnapshot {
    host_id: wc.host_id.clone(),
    players: wc.players.clone(),
    original_names: wc.original_names.clone(),
    display_name: wc.display_name.clone(),
  };
  let backup_dir = backup_files(&dir, &wpath, &player_ids, &snapshot)?;
  Ok(backup_dir.to_string_lossy().to_string())
}

#[tauri::command]
fn list_backups(account_id: String, world_id: String) -> Result<Vec<String>, String> {
  let dir = players_dir(&account_id, &world_id)?;
  Ok(list_backups_dir(&dir))
}

#[tauri::command]
async fn restore_backup(
  app: AppHandle,
  account_id: String,
  world_id: String,
  backup_name: String,
) -> Result<Vec<Player>, String> {
  let a = app.clone();
  tauri::async_runtime::spawn_blocking(move || {
    restore_backup_sync(&a, &account_id, &world_id, &backup_name)
  })
  .await
  .map_err(|e| format!("Task error: {e}"))?
}

fn restore_backup_sync(
  app: &AppHandle,
  account_id: &str,
  world_id: &str,
  backup_name: &str,
) -> Result<Vec<Player>, String> {
  let dir = players_dir(account_id, world_id)?;
  let wpath = world_dir(account_id, world_id)?;
  let backup_dir = dir.join("backup").join(backup_name);
  if !backup_dir.exists() {
    return Err("Backup not found.".to_string());
  }

  // Restore .sav files
  let entries = fs::read_dir(&backup_dir).map_err(|err| err.to_string())?;
  for entry in entries.flatten() {
    let file_path = entry.path();
    if let Some(name) = file_path.file_name().and_then(|value| value.to_str()) {
      if name.ends_with(".sav") {
        if name == "Level.sav" {
          // Restore Level.sav to world root
          let dest = wpath.join(name);
          fs::copy(&file_path, dest).map_err(|err| err.to_string())?;
        } else {
          // Restore player .sav to Players dir
          let dest = dir.join(name);
          fs::copy(&file_path, dest).map_err(|err| err.to_string())?;
        }
      }
    }
  }

  // Restore config snapshot into world-local config
  let snapshot_path = backup_dir.join("config_snapshot.json");
  if snapshot_path.exists() {
    let raw = fs::read_to_string(&snapshot_path).map_err(|err| err.to_string())?;
    if let Ok(snapshot) = serde_json::from_str::<BackupSnapshot>(&raw) {
      let mut wc = load_world_config(&dir);
      wc.players = snapshot.players;
      wc.original_names = snapshot.original_names;
      wc.host_id = snapshot.host_id;
      wc.display_name = snapshot.display_name;
      save_world_config(&dir, &wc)?;
    }
  }

  get_players_sync(app, account_id, world_id)
}

#[tauri::command]
fn delete_backup(account_id: String, world_id: String, backup_name: String) -> Result<Vec<String>, String> {
  let dir = players_dir(&account_id, &world_id)?;
  let backup_dir = dir.join("backup").join(&backup_name);
  if backup_dir.exists() {
    fs::remove_dir_all(&backup_dir).map_err(|err| err.to_string())?;
  }
  Ok(list_backups_dir(&dir))
}

#[tauri::command]
fn delete_all_backups(account_id: String, world_id: String) -> Result<Vec<String>, String> {
  let dir = players_dir(&account_id, &world_id)?;
  let backup_root = dir.join("backup");
  if backup_root.exists() {
    fs::remove_dir_all(&backup_root).map_err(|err| err.to_string())?;
  }
  Ok(Vec::new())
}

// ── World transfer ────────────────────────────────────────

/// Export a world folder as a ZIP file (runs on background thread).
#[tauri::command]
async fn export_world(app: AppHandle, account_id: String, world_id: String, dest_path: String) -> Result<String, String> {
  let app2 = app.clone();
  tauri::async_runtime::spawn_blocking(move || {
    export_world_sync(&app2, &account_id, &world_id, &dest_path)
  })
  .await
  .map_err(|e| format!("Task error: {e}"))?
}

fn export_world_sync(app: &AppHandle, account_id: &str, world_id: &str, dest_path: &str) -> Result<String, String> {
  let wdir = world_dir(account_id, world_id)?;
  if !wdir.exists() {
    return Err("World folder does not exist.".to_string());
  }

  let dest = PathBuf::from(dest_path);

  // Ensure destination directory exists
  if let Some(parent) = dest.parent() {
    if !parent.exists() {
      fs::create_dir_all(parent).map_err(|e| format!("Cannot create destination folder: {e}"))?;
    }
  }

  // ── Skip ALL backup directories for P2P export ──────────────────────
  // Skip <worldDir>/backup/ (Palworld game backups: backup/world/ and backup/local/)
  // and <worldDir>/Players/backup/ (PalHost swap backups).
  // Backups are unnecessary for P2P transfer and can be 100MB+ each.
  let skip_dirs: Vec<PathBuf> = vec![
    wdir.join("backup"),
    wdir.join("Players").join("backup"),
  ];

  // Count total files for progress (excluding skipped backup dirs)
  let entries: Vec<_> = WalkDir::new(&wdir)
    .into_iter()
    .filter_map(|e| e.ok())
    .filter(|e| {
      let p = e.path();
      !skip_dirs.iter().any(|sk| p.starts_with(sk))
    })
    .collect();
  let total = entries.iter().filter(|e| e.path().is_file()).count().max(1);
  let mut done = 0usize;
  let mut last_pct = 0u32;

  let _ = app.emit("export-progress", ProgressPayload { percent: 0.0, message: "Starting export…".to_string() });

  let file = fs::File::create(&dest)
    .map_err(|e| format!("Cannot create ZIP file: {e}"))?;
  let mut zip = zip::ZipWriter::new(file);
  let options = SimpleFileOptions::default()
    .compression_method(zip::CompressionMethod::Deflated)
    .unix_permissions(0o644);

  // Walk the world directory and add all files
  for entry in &entries {
    let abs_path = entry.path();
    let rel_path = abs_path.strip_prefix(&wdir).map_err(|e| e.to_string())?;

    // Use world_id as the root folder name inside the ZIP
    let archive_path = PathBuf::from(world_id).join(rel_path);
    let archive_name = archive_path.to_string_lossy().replace('\\', "/");

    if abs_path.is_dir() {
      zip.add_directory(&archive_name, options)
        .map_err(|e| format!("Error adding folder to ZIP: {e}"))?;
    } else {
      zip.start_file(&archive_name, options)
        .map_err(|e| format!("Error adding file to ZIP: {e}"))?;
      let mut f = fs::File::open(abs_path)
        .map_err(|e| format!("Cannot read {}: {e}", abs_path.display()))?;
      let mut buf = Vec::new();
      f.read_to_end(&mut buf)
        .map_err(|e| format!("File read error: {e}"))?;
      zip.write_all(&buf)
        .map_err(|e| format!("ZIP write error: {e}"))?;
      done += 1;
      let pct = (done as f64 / total as f64 * 100.0).min(100.0) as u32;
      // Throttle: emit only when percentage changes by at least 2%
      if pct >= last_pct + 2 || done == total {
        last_pct = pct;
        let _ = app.emit("export-progress", ProgressPayload { percent: pct as f64, message: format!("Compressing… {done}/{total}") });
      }
    }
  }

  zip.finish().map_err(|e| format!("Error finalizing ZIP: {e}"))?;
  let _ = app.emit("export-progress", ProgressPayload { percent: 100.0, message: "Export complete.".to_string() });
  Ok(dest.to_string_lossy().to_string())
}

/// Validate a folder to check if it looks like a valid Palworld world.
/// Returns the folder name (world ID).
#[tauri::command]
fn validate_world_folder(folder_path: String) -> Result<ValidatedFolder, String> {
  let src = PathBuf::from(&folder_path);
  if !src.exists() || !src.is_dir() {
    return Err("The path is not a valid folder.".to_string());
  }

  // Helper: check if a directory looks like a valid Palworld world
  let is_valid_world = |dir: &Path| -> bool {
    let players_sub = dir.join("Players");
    let has_players = players_sub.exists() && players_sub.is_dir();
    let has_sav = fs::read_dir(dir)
      .ok()
      .into_iter()
      .flatten()
      .filter_map(|e| e.ok())
      .any(|e| {
        e.path()
          .extension()
          .map(|ext| ext == "sav")
          .unwrap_or(false)
      });
    has_players || has_sav
  };

  // First, check the folder itself
  if is_valid_world(&src) {
    let folder_name = src
      .file_name()
      .and_then(|n| n.to_str())
      .ok_or("Invalid folder name.")?
      .to_string();
    return Ok(ValidatedFolder { name: folder_name, path: folder_path });
  }

  // Fallback: check for a subfolder with the same name (common after ZIP extraction)
  let folder_name = src
    .file_name()
    .and_then(|n| n.to_str())
    .ok_or("Invalid folder name.")?
    .to_string();
  let nested = src.join(&folder_name);
  if nested.exists() && nested.is_dir() && is_valid_world(&nested) {
    return Ok(ValidatedFolder {
      name: folder_name,
      path: nested.to_string_lossy().to_string(),
    });
  }

  // Also check any single subfolder (in case name differs)
  let sub_entries: Vec<_> = fs::read_dir(&src)
    .ok()
    .into_iter()
    .flatten()
    .filter_map(|e| e.ok())
    .filter(|e| e.path().is_dir())
    .collect();
  if sub_entries.len() == 1 {
    let sub = &sub_entries[0];
    let sub_path = sub.path();
    if is_valid_world(&sub_path) {
      let sub_name = sub_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&folder_name)
        .to_string();
      return Ok(ValidatedFolder {
        name: sub_name,
        path: sub_path.to_string_lossy().to_string(),
      });
    }
  }

  Err("The folder does not appear to be a valid Palworld world (missing Players/ folder and .sav files).".to_string())
}

/// Check if a world folder already exists for the given account.
#[tauri::command]
fn check_world_exists(account_id: String, world_name: String) -> Result<bool, String> {
  if account_id.trim().is_empty() || world_name.trim().is_empty() {
    return Ok(false);
  }
  let target = save_games_root()?.join(&account_id).join(&world_name);
  Ok(target.exists())
}

/// Import a world folder into the account's save directory (runs on background thread).
/// mode: "replace" | "new"
/// new_name is used only when mode == "new"
#[tauri::command]
async fn import_world(
  app: AppHandle,
  account_id: String,
  folder_path: String,
  mode: String,
  new_name: Option<String>,
) -> Result<Vec<WorldInfo>, String> {
  let app2 = app.clone();
  tauri::async_runtime::spawn_blocking(move || {
    import_world_sync(&app2, &account_id, &folder_path, &mode, new_name.as_deref())
  })
  .await
  .map_err(|e| format!("Task error: {e}"))?
}

fn import_world_sync(
  app: &AppHandle,
  account_id: &str,
  folder_path: &str,
  mode: &str,
  new_name: Option<&str>,
) -> Result<Vec<WorldInfo>, String> {
  let src = PathBuf::from(folder_path);
  if !src.exists() || !src.is_dir() {
    return Err("Source folder does not exist.".to_string());
  }

  let folder_name = src
    .file_name()
    .and_then(|n| n.to_str())
    .ok_or("Invalid source folder name.")?
    .to_string();

  let target_name = match mode {
    "new" => {
      let n = new_name.unwrap_or(&folder_name).to_string();
      if n.trim().is_empty() {
        return Err("World name cannot be empty.".to_string());
      }
      n
    }
    _ => folder_name.clone(),
  };

  let account_root = save_games_root()?.join(account_id);
  if !account_root.exists() {
    return Err("Account folder does not exist.".to_string());
  }
  let target = account_root.join(&target_name);

  if mode == "new" && target.exists() {
    return Err(format!("A world named '{}' already exists.", target_name));
  }

  if mode == "replace" {
    if target.exists() {
      // Remove everything EXCEPT backup/world and backup/local
      remove_dir_except_backups(&target)
        .map_err(|e| format!("Cannot clean existing world: {e}"))?;
    }
  }

  // ── Build skip-set for old backups in the SOURCE ──────────────────
  // Keep only the most recent backup subfolder in each category
  // so we don't bloat the destination with tons of old backup folders.
  let mut skip_src_dirs: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

  for sub in &["world", "local"] {
    let bdir = src.join("backup").join(sub);
    if bdir.is_dir() {
      if let Ok(rd) = fs::read_dir(&bdir) {
        let mut folders: Vec<PathBuf> = rd
          .filter_map(|e| e.ok())
          .filter(|e| e.path().is_dir())
          .map(|e| e.path())
          .collect();
        // Sort descending by name (timestamp format sorts lexicographically)
        folders.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
        // Skip everything except the most recent
        for old in folders.iter().skip(1) {
          skip_src_dirs.insert(old.clone());
        }
      }
    }
  }

  // Count total files for progress (excluding skipped backup dirs)
  let total_files = WalkDir::new(&src)
    .into_iter()
    .filter_map(|e| e.ok())
    .filter(|e| {
      let p = e.path();
      !skip_src_dirs.iter().any(|sk| p.starts_with(sk))
    })
    .filter(|e| e.path().is_file())
    .count()
    .max(1);
  let counter = std::sync::atomic::AtomicUsize::new(0);
  let mut last_pct = 0u32;

  let _ = app.emit("import-progress", ProgressPayload { percent: 0.0, message: "Starting import…".to_string() });

  // Recursively copy src into target, merging backups and skipping old ones
  copy_dir_recursive_merge(&src, &target, app, &counter, total_files, &mut last_pct, &skip_src_dirs)?;

  let _ = app.emit("import-progress", ProgressPayload { percent: 100.0, message: "Import complete.".to_string() });

  // Return updated world list
  get_worlds_with_counts(account_id.to_string())
}

/// Remove all contents of a world directory EXCEPT backup/world and backup/local.
/// This preserves existing game backups while replacing everything else.
fn remove_dir_except_backups(dir: &Path) -> std::io::Result<()> {
  for entry in fs::read_dir(dir)? {
    let entry = entry?;
    let path = entry.path();
    let name = entry.file_name();

    if name == "backup" && path.is_dir() {
      // Inside the backup folder, remove everything except "world" and "local"
      for bentry in fs::read_dir(&path)? {
        let bentry = bentry?;
        let bname = bentry.file_name();
        if bname != "world" && bname != "local" {
          if bentry.path().is_dir() {
            fs::remove_dir_all(bentry.path())?;
          } else {
            fs::remove_file(bentry.path())?;
          }
        }
      }
    } else if path.is_dir() {
      fs::remove_dir_all(&path)?;
    } else {
      fs::remove_file(&path)?;
    }
  }
  Ok(())
}

/// Recursively copy src to dest, merging backup directories and skipping old backup folders.
fn copy_dir_recursive_merge(
  src: &Path,
  dest: &Path,
  app: &AppHandle,
  counter: &std::sync::atomic::AtomicUsize,
  total: usize,
  last_pct: &mut u32,
  skip_dirs: &std::collections::HashSet<PathBuf>,
) -> Result<(), String> {
  if !dest.exists() {
    fs::create_dir_all(dest).map_err(|e| format!("Cannot create {}: {e}", dest.display()))?;
  }
  for entry in fs::read_dir(src).map_err(|e| format!("Cannot read {}: {e}", src.display()))? {
    let entry = entry.map_err(|e| e.to_string())?;
    let path = entry.path();

    // Skip old backup folders from the source
    if skip_dirs.iter().any(|sk| path == *sk || path.starts_with(sk)) {
      continue;
    }

    let dest_path = dest.join(entry.file_name());
    if path.is_dir() {
      // For backup subdirs that already exist at destination, don't clear them — just merge
      copy_dir_recursive_merge(&path, &dest_path, app, counter, total, last_pct, skip_dirs)?;
    } else {
      fs::copy(&path, &dest_path)
        .map_err(|e| format!("Cannot copy {}: {e}", path.display()))?;
      let done = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
      let pct = (done as f64 / total as f64 * 100.0).min(100.0) as u32;
      if pct >= *last_pct + 2 || done == total {
        *last_pct = pct;
        let _ = app.emit("import-progress", ProgressPayload { percent: pct as f64, message: format!("Copying… {done}/{total}") });
      }
    }
  }
  Ok(())
}

#[tauri::command]
fn is_palworld_running() -> bool {
  use std::os::windows::process::CommandExt;
  const CREATE_NO_WINDOW: u32 = 0x08000000;

  if let Ok(output) = StdCommand::new("tasklist")
    .args(["/FI", "IMAGENAME eq Palworld-Win64-Shipping.exe", "/NH", "/FO", "CSV"])
    .creation_flags(CREATE_NO_WINDOW)
    .output()
  {
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.contains("Palworld-Win64-Shipping.exe")
  } else {
    false
  }
}

#[tauri::command]
fn rescan_storage() -> Result<(), String> {
  Ok(())
}

// ── P2P Transfer helper commands ──────────────────────────

/// Export a world to a temporary ZIP file for P2P sharing.
/// Returns the full path to the temp ZIP.
#[tauri::command]
async fn export_world_to_temp(app: AppHandle, account_id: String, world_id: String) -> Result<String, String> {
  let temp_path = std::env::temp_dir()
    .join(format!("palhost_share_{}.zip", &world_id))
    .to_string_lossy()
    .to_string();
  let tp = temp_path.clone();
  let app2 = app.clone();
  tauri::async_runtime::spawn_blocking(move || {
    export_world_sync(&app2, &account_id, &world_id, &tp)
  })
  .await
  .map_err(|e| format!("Task error: {e}"))?
}

/// Get the file size in bytes.
#[tauri::command]
fn get_file_size(path: String) -> Result<u64, String> {
  let meta = fs::metadata(&path).map_err(|e| format!("Cannot read: {e}"))?;
  Ok(meta.len())
}

/// Read a binary chunk from a file. Returns Vec<u8> → ArrayBuffer on JS side.
#[tauri::command]
fn read_file_chunk(path: String, offset: u64, length: u64) -> Result<Vec<u8>, String> {
  let mut f = fs::File::open(&path).map_err(|e| format!("Cannot open: {e}"))?;
  f.seek(std::io::SeekFrom::Start(offset)).map_err(|e| format!("Seek error: {e}"))?;
  let mut buf = vec![0u8; length as usize];
  let n = f.read(&mut buf).map_err(|e| format!("Read error: {e}"))?;
  buf.truncate(n);
  Ok(buf)
}

/// Decode a base64 string and append it to a file (creates if needed).
#[tauri::command]
fn append_file_chunk_b64(path: String, data_b64: String) -> Result<(), String> {
  let data = base64_decode(&data_b64)
    .map_err(|_| "Invalid base64 data".to_string())?;
  let mut f = fs::OpenOptions::new()
    .create(true)
    .append(true)
    .open(&path)
    .map_err(|e| format!("Cannot open: {e}"))?;
  f.write_all(&data).map_err(|e| format!("Write error: {e}"))?;
  Ok(())
}

/// Get a path in the system temp directory for receiving P2P files.
#[tauri::command]
fn get_temp_path(filename: String) -> String {
  std::env::temp_dir()
    .join(&filename)
    .to_string_lossy()
    .to_string()
}

/// Delete a temporary file.
#[tauri::command]
fn delete_temp_file(path: String) -> Result<(), String> {
  let p = Path::new(&path);
  if p.exists() {
    if p.is_dir() {
      fs::remove_dir_all(p).map_err(|e| format!("Cannot delete: {e}"))?;
    } else {
      fs::remove_file(p).map_err(|e| format!("Cannot delete: {e}"))?;
    }
  }
  Ok(())
}

/// Extract a ZIP file to a temp directory and return the extracted folder path.
#[tauri::command]
fn extract_zip_to_temp(zip_path: String) -> Result<String, String> {
  let zip_file = fs::File::open(&zip_path)
    .map_err(|e| format!("Cannot open ZIP: {e}"))?;
  let mut archive = zip::ZipArchive::new(zip_file)
    .map_err(|e| format!("Invalid ZIP: {e}"))?;

  let extract_dir = std::env::temp_dir().join("palhost_p2p_extract");
  // Clean previous extraction
  if extract_dir.exists() {
    let _ = fs::remove_dir_all(&extract_dir);
  }
  fs::create_dir_all(&extract_dir)
    .map_err(|e| format!("Cannot create temp dir: {e}"))?;

  for i in 0..archive.len() {
    let mut file = archive.by_index(i)
      .map_err(|e| format!("ZIP read error: {e}"))?;
    let out_path = extract_dir.join(file.mangled_name());

    if file.is_dir() {
      fs::create_dir_all(&out_path)
        .map_err(|e| format!("Cannot create dir: {e}"))?;
    } else {
      if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)
          .map_err(|e| format!("Cannot create parent: {e}"))?;
      }
      let mut out_file = fs::File::create(&out_path)
        .map_err(|e| format!("Cannot create file: {e}"))?;
      std::io::copy(&mut file, &mut out_file)
        .map_err(|e| format!("Extract error: {e}"))?;
    }
  }

  // Find the world folder inside (should be the first directory)
  let mut world_folder = extract_dir.clone();
  if let Ok(entries) = fs::read_dir(&extract_dir) {
    for entry in entries.flatten() {
      if entry.path().is_dir() {
        world_folder = entry.path();
        break;
      }
    }
  }

  Ok(world_folder.to_string_lossy().to_string())
}

/// Simple base64 decoder (no extra crate needed).
fn base64_decode(input: &str) -> Result<Vec<u8>, ()> {
  let table: [u8; 128] = {
    let mut t = [255u8; 128];
    for (i, &c) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".iter().enumerate() {
      t[c as usize] = i as u8;
    }
    t
  };
  let input = input.as_bytes();
  let mut out = Vec::with_capacity(input.len() * 3 / 4);
  let mut buf = 0u32;
  let mut bits = 0u32;
  for &b in input {
    if b == b'=' || b == b'\n' || b == b'\r' || b == b' ' { continue; }
    let val = if (b as usize) < 128 { table[b as usize] } else { 255 };
    if val == 255 { return Err(()); }
    buf = (buf << 6) | val as u32;
    bits += 6;
    if bits >= 8 {
      bits -= 8;
      out.push((buf >> bits) as u8);
      buf &= (1 << bits) - 1;
    }
  }
  Ok(out)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
    .setup(|app| {
      if cfg!(debug_assertions) {
        app.handle().plugin(
          tauri_plugin_log::Builder::default()
            .level(log::LevelFilter::Info)
            .filter(|metadata| {
              // Suppress noisy tao event-loop warnings on Windows
              !metadata.target().starts_with("tao::")
            })
            .build(),
        )?;
      }
      app.handle().plugin(tauri_plugin_dialog::init())?;
      // Migrate old app-level config data into per-world files
      let _ = migrate_legacy_config(app.handle());
      Ok(())
    })
    .invoke_handler(tauri::generate_handler![
      get_accounts,
      get_worlds,
      get_worlds_with_counts,
      get_players,
      set_host_player,
      swap_players,
      create_backup,
      list_backups,
      restore_backup,
      delete_backup,
      delete_all_backups,
      export_world,
      validate_world_folder,
      check_world_exists,
      import_world,
      set_world_name,
      reset_world_name,
      is_palworld_running,
      rescan_storage,
      export_world_to_temp,
      get_file_size,
      read_file_chunk,
      append_file_chunk_b64,
      get_temp_path,
      delete_temp_file,
      extract_zip_to_temp,
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::path::Path;

  /// Integration test: perform swap on original save files and compare with
  /// PalworldSaveTools "correct" output.
  ///
  /// Swaps player 00000000000000000000000000000001 ↔ BAAB90A2000000000000000000000000.
  #[test]
  fn test_swap_matches_palworld_save_tools() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR"))
      .parent().unwrap()
      .join("examples").join("json example");
    let original = examples.join("original").join("E310B8F24E41312E1A141FBBAEB1645A");
    let correct  = examples.join("correct").join("E310B8F24E41312E1A141FBBAEB1645A");

    if !original.join("Level.sav").exists() {
      eprintln!("Skipping: original Level.sav not found");
      return;
    }
    if !correct.join("Level.sav").exists() {
      eprintln!("Skipping: correct Level.sav not found");
      return;
    }

    // Copy original to a temp directory
    let tmp = std::env::temp_dir().join("palhost_swap_test");
    if tmp.exists() {
      fs::remove_dir_all(&tmp).unwrap();
    }
    fs::create_dir_all(tmp.join("Players")).unwrap();

    // Copy Level.sav
    fs::copy(original.join("Level.sav"), tmp.join("Level.sav")).unwrap();
    // Copy player .sav files
    for entry in fs::read_dir(original.join("Players")).unwrap() {
      let entry = entry.unwrap();
      let name = entry.file_name().to_string_lossy().to_string();
      if name.ends_with(".sav") {
        fs::copy(entry.path(), tmp.join("Players").join(&name)).unwrap();
      }
    }

    let players_dir = tmp.join("Players");

    // Run our swap
    let result = swap_players_full(
      &tmp,
      &players_dir,
      "00000000000000000000000000000001",
      "BAAB90A2000000000000000000000000",
      None,
    );
    assert!(result.is_ok(), "swap_players_full failed: {:?}", result.err());

    // ── Compare Level.sav ──
    let our_data = fs::read(tmp.join("Level.sav")).unwrap();
    let (our_json, _) = gvas::sav_to_json(&our_data).expect("parse our Level.sav");

    // Load correct Level.json (PST output)
    let correct_json: Value = serde_json::from_str(
      &fs::read_to_string(correct.join("Level.json")).expect("read correct Level.json")
    ).expect("parse correct Level.json");

    let our_wsd = &our_json["properties"]["worldSaveData"]["value"];
    let cor_wsd = &correct_json["properties"]["worldSaveData"]["value"];

    // Compare CSPM key.PlayerUId — should match for ALL entries
    let our_cspm = our_wsd["CharacterSaveParameterMap"]["value"].as_array().unwrap();
    let cor_cspm = cor_wsd["CharacterSaveParameterMap"]["value"].as_array().unwrap();
    assert_eq!(our_cspm.len(), cor_cspm.len(), "CSPM entry count mismatch");

    let mut cspm_key_diffs = 0;
    let mut cspm_key_diff_details = Vec::new();
    for (i, (ours, cors)) in our_cspm.iter().zip(cor_cspm.iter()).enumerate() {
      let our_puid = ours.pointer("/key/PlayerUId/value").and_then(|v| v.as_str()).unwrap_or("");
      let cor_puid = cors.pointer("/key/PlayerUId/value").and_then(|v| v.as_str()).unwrap_or("");
      if our_puid != cor_puid {
        cspm_key_diffs += 1;
        if cspm_key_diff_details.len() < 10 {
          cspm_key_diff_details.push(format!(
            "idx {i}: ours={our_puid} expected={cor_puid}"
          ));
        }
      }
    }
    assert_eq!(
      cspm_key_diffs, 0,
      "CSPM key.PlayerUId mismatches: {cspm_key_diffs}\nFirst diffs: {cspm_key_diff_details:?}"
    );

    // Compare OwnerPlayerUId across all CSPM entries
    let mut owner_diffs = 0;
    let mut owner_diff_details = Vec::new();
    for (i, (ours, cors)) in our_cspm.iter().zip(cor_cspm.iter()).enumerate() {
      let our_owner = ours.pointer("/value/RawData/value/object/SaveParameter/value/OwnerPlayerUId/value")
        .and_then(|v| v.as_str()).unwrap_or("");
      let cor_owner = cors.pointer("/value/RawData/value/object/SaveParameter/value/OwnerPlayerUId/value")
        .and_then(|v| v.as_str()).unwrap_or("");
      if our_owner != cor_owner {
        owner_diffs += 1;
        if owner_diff_details.len() < 10 {
          owner_diff_details.push(format!(
            "idx {i}: ours={our_owner} expected={cor_owner}"
          ));
        }
      }
    }
    assert_eq!(
      owner_diffs, 0,
      "OwnerPlayerUId mismatches: {owner_diffs}\nFirst diffs: {owner_diff_details:?}"
    );

    // Compare GroupSaveDataMap guild info
    let our_gsm = our_wsd["GroupSaveDataMap"]["value"].as_array().unwrap();
    let cor_gsm = cor_wsd["GroupSaveDataMap"]["value"].as_array().unwrap();
    for (i, (ours, cors)) in our_gsm.iter().zip(cor_gsm.iter()).enumerate() {
      let our_rd = &ours["value"]["RawData"]["value"];
      let cor_rd = &cors["value"]["RawData"]["value"];

      let our_admin = our_rd["admin_player_uid"].as_str().unwrap_or("");
      let cor_admin = cor_rd["admin_player_uid"].as_str().unwrap_or("");
      assert_eq!(our_admin, cor_admin, "Guild {i} admin_player_uid mismatch");

      // Compare player_uid list
      if let (Some(our_players), Some(cor_players)) =
        (our_rd["players"].as_array(), cor_rd["players"].as_array())
      {
        for (j, (op, cp)) in our_players.iter().zip(cor_players.iter()).enumerate() {
          let our_puid = op["player_uid"].as_str().unwrap_or("");
          let cor_puid = cp["player_uid"].as_str().unwrap_or("");
          assert_eq!(our_puid, cor_puid, "Guild {i} player {j} uid mismatch");
        }
      }

      // Compare individual_character_handle_ids guid
      if let (Some(our_handles), Some(cor_handles)) = (
        our_rd["individual_character_handle_ids"].as_array(),
        cor_rd["individual_character_handle_ids"].as_array(),
      ) {
        let mut handle_diffs = 0;
        for (oh, ch) in our_handles.iter().zip(cor_handles.iter()) {
          if oh["guid"].as_str() != ch["guid"].as_str() {
            handle_diffs += 1;
          }
        }
        assert_eq!(handle_diffs, 0, "Guild {i}: {handle_diffs} handle guid mismatches");
      }
    }

    // Compare player .sav files
    let our_host_sav = tmp.join("Players").join("00000000000000000000000000000001.sav");
    let our_baa_sav_upper = tmp.join("Players").join("BAAB90A2000000000000000000000000.sav");
    let our_baa_sav_lower = tmp.join("Players").join("baab90a2000000000000000000000000.sav");
    let our_baa_sav = if our_baa_sav_upper.exists() { our_baa_sav_upper } else { our_baa_sav_lower };

    let our_host_data = fs::read(&our_host_sav).unwrap();
    let (our_host_json, _) = gvas::sav_to_json(&our_host_data).expect("parse our host.sav");
    let our_baa_data = fs::read(&our_baa_sav).unwrap();
    let (our_baa_json, _) = gvas::sav_to_json(&our_baa_data).expect("parse our baa.sav");

    // After PST-style swap:
    // host.sav now contains baa's original data with PlayerUId changed to baa UUID
    // baa.sav now contains host's original data with PlayerUId changed to host UUID
    // (because: patch UIDs first, then rename files)
    // After PST-style swap:
    // 1. host.sav has PlayerUId patched HOST→BAA, baa.sav has PlayerUId patched BAA→HOST
    // 2. Files are renamed (swapped): host.sav↔baa.sav
    // Result: HOST.sav file = old baa data with PlayerUId=HOST, BAA.sav file = old host data with PlayerUId=BAA
    let our_host_puid = our_host_json.pointer("/properties/SaveData/value/PlayerUId/value")
      .and_then(|v| v.as_str()).unwrap_or("");
    assert_eq!(our_host_puid, "00000000-0000-0000-0000-000000000001",
      "host.sav PlayerUId should stay HOST UUID (baa data renamed into host slot)");

    let our_baa_puid = our_baa_json.pointer("/properties/SaveData/value/PlayerUId/value")
      .and_then(|v| v.as_str()).unwrap_or("");
    assert_eq!(our_baa_puid, "baab90a2-0000-0000-0000-000000000000",
      "baa.sav PlayerUId should stay BAA UUID (host data renamed into baa slot)");

    // Compare InstanceIds with correct output
    let cor_host_json: Value = serde_json::from_str(
      &fs::read_to_string(correct.join("Players").join("00000000000000000000000000000001.json")).unwrap()
    ).unwrap();

    let our_host_inst = our_host_json.pointer("/properties/SaveData/value/IndividualId/value/InstanceId/value")
      .and_then(|v| v.as_str()).unwrap_or("");
    let cor_host_inst = cor_host_json.pointer("/properties/SaveData/value/IndividualId/value/InstanceId/value")
      .and_then(|v| v.as_str()).unwrap_or("");
    assert_eq!(our_host_inst, cor_host_inst, "host.sav InstanceId mismatch");

    eprintln!("All swap comparisons passed!");

    // Cleanup
    let _ = fs::remove_dir_all(&tmp);
  }
}
