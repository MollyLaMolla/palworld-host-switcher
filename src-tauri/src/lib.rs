use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use tauri::{AppHandle, Emitter, Manager};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;

const DEFAULT_HOST_ID: &str = "00000000000000000000000000000001";
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Player {
  id: String,
  name: String,
  original_id: String,
  is_host: bool,
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

fn resolve_host_id(wc: &WorldConfig, player_ids: &[String]) -> Option<String> {
  if let Some(host_id) = &wc.host_id {
    let normalized = normalize_id(host_id);
    if player_ids.contains(&normalized) {
      return Some(normalized);
    }
  }
  let default_host = normalize_id(DEFAULT_HOST_ID);
  if player_ids.contains(&default_host) {
    return Some(default_host);
  }
  player_ids.first().cloned()
}

fn build_players(
  wc: &mut WorldConfig,
  player_ids: &[String],
  host_id: &str,
) -> Vec<Player> {
  player_ids
    .iter()
    .map(|id| {
      let name = wc
        .players
        .entry(id.clone())
        .or_insert_with(|| id.clone())
        .clone();
      let original_id = wc
        .original_names
        .entry(id.clone())
        .or_insert_with(|| id.clone())
        .clone();
      Player {
        id: id.clone(),
        name,
        original_id,
        is_host: id == host_id,
      }
    })
    .collect()
}

fn swap_files(players_dir: &Path, first_id: &str, second_id: &str) -> Result<(), String> {
  let first = players_dir.join(format!("{}.sav", normalize_id(first_id)));
  let second = players_dir.join(format!("{}.sav", normalize_id(second_id)));
  if !first.exists() || !second.exists() {
    return Err("Missing .sav files for swap.".to_string());
  }
  let stamp = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map_err(|err| err.to_string())?
    .as_millis();
  let temp = players_dir.join(format!("swap-{stamp}.tmp"));
  fs::rename(&first, &temp).map_err(|err| err.to_string())?;
  fs::rename(&second, &first).map_err(|err| err.to_string())?;
  fs::rename(&temp, &second).map_err(|err| err.to_string())?;
  Ok(())
}

fn backup_files(players_dir: &Path, ids: &[String], snapshot: &BackupSnapshot) -> Result<PathBuf, String> {
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
fn get_players(app: AppHandle, account_id: String, world_id: String) -> Result<Vec<Player>, String> {
  let dir = players_dir(&account_id, &world_id)?;
  let player_ids = list_player_ids(&dir);
  if player_ids.is_empty() {
    return Ok(Vec::new());
  }
  let mut wc = load_world_config(&dir);
  prune_world_config(&mut wc, &player_ids);
  let host_id = resolve_host_id(&wc, &player_ids).ok_or("Host not found.")?;
  wc.host_id = Some(host_id.clone());
  let players = build_players(&mut wc, &player_ids, &host_id);
  save_world_config(&dir, &wc)?;
  // Remember last-used account/world
  let mut ac = load_app_config(&app).unwrap_or_default();
  ac.account_id = Some(account_id);
  ac.world_id = Some(world_id);
  let _ = save_app_config(&app, &ac);
  Ok(players)
}

#[tauri::command]
fn set_host_player(
  app: AppHandle,
  account_id: String,
  world_id: String,
  player_id: String,
) -> Result<Vec<Player>, String> {
  let dir = players_dir(&account_id, &world_id)?;
  let player_ids = list_player_ids(&dir);
  let mut wc = load_world_config(&dir);
  let host_id = resolve_host_id(&wc, &player_ids).ok_or("Host not found.")?;
  let target_id = normalize_id(&player_id);
  if host_id == target_id {
    return get_players(app, account_id, world_id);
  }
  swap_files(&dir, &host_id, &target_id)?;
  // Swap display names
  let first_name = wc.players.entry(host_id.clone()).or_insert_with(|| host_id.clone()).clone();
  let second_name = wc.players.entry(target_id.clone()).or_insert_with(|| target_id.clone()).clone();
  wc.players.insert(host_id.clone(), second_name);
  wc.players.insert(target_id.clone(), first_name);
  // Swap original identities (they follow the data)
  let first_orig = wc.original_names.entry(host_id.clone()).or_insert_with(|| host_id.clone()).clone();
  let second_orig = wc.original_names.entry(target_id.clone()).or_insert_with(|| target_id.clone()).clone();
  wc.original_names.insert(host_id, second_orig);
  wc.original_names.insert(target_id, first_orig);
  save_world_config(&dir, &wc)?;
  get_players(app, account_id, world_id)
}

#[tauri::command]
fn swap_players(
  app: AppHandle,
  account_id: String,
  world_id: String,
  first_id: String,
  second_id: String,
) -> Result<Vec<Player>, String> {
  let dir = players_dir(&account_id, &world_id)?;
  let first = normalize_id(&first_id);
  let second = normalize_id(&second_id);
  swap_files(&dir, &first, &second)?;
  let mut wc = load_world_config(&dir);
  // Swap display names
  let first_name = wc.players.entry(first.clone()).or_insert_with(|| first.clone()).clone();
  let second_name = wc.players.entry(second.clone()).or_insert_with(|| second.clone()).clone();
  wc.players.insert(first.clone(), second_name);
  wc.players.insert(second.clone(), first_name);
  // Swap original identities (they follow the data)
  let first_orig = wc.original_names.entry(first.clone()).or_insert_with(|| first.clone()).clone();
  let second_orig = wc.original_names.entry(second.clone()).or_insert_with(|| second.clone()).clone();
  wc.original_names.insert(first, second_orig);
  wc.original_names.insert(second, first_orig);
  save_world_config(&dir, &wc)?;
  get_players(app, account_id, world_id)
}

#[tauri::command]
fn set_host_slot(
  app: AppHandle,
  account_id: String,
  world_id: String,
  host_id: String,
) -> Result<Vec<Player>, String> {
  let dir = players_dir(&account_id, &world_id)?;
  let mut wc = load_world_config(&dir);
  wc.host_id = Some(normalize_id(&host_id));
  save_world_config(&dir, &wc)?;
  get_players(app, account_id, world_id)
}

#[tauri::command]
fn set_player_name(
  app: AppHandle,
  account_id: String,
  world_id: String,
  player_id: String,
  name: String,
) -> Result<Vec<Player>, String> {
  let dir = players_dir(&account_id, &world_id)?;
  let mut wc = load_world_config(&dir);
  let normalized = normalize_id(&player_id);
  wc.original_names
    .entry(normalized.clone())
    .or_insert_with(|| normalized.clone());
  wc.players.insert(normalized, name);
  save_world_config(&dir, &wc)?;
  get_players(app, account_id, world_id)
}

#[tauri::command]
fn reset_player_names(
  app: AppHandle,
  account_id: String,
  world_id: String,
) -> Result<Vec<Player>, String> {
  let dir = players_dir(&account_id, &world_id)?;
  let player_ids = list_player_ids(&dir);
  let mut wc = load_world_config(&dir);
  for id in player_ids {
    // Restore display name to the original identity stored for this slot.
    let orig = wc.original_names.get(&id).cloned().unwrap_or_else(|| id.clone());
    wc.players.insert(id, orig);
  }
  save_world_config(&dir, &wc)?;
  get_players(app, account_id, world_id)
}

#[tauri::command]
fn create_backup(
  _app: AppHandle,
  account_id: String,
  world_id: String,
  player_ids: Vec<String>,
) -> Result<String, String> {
  let dir = players_dir(&account_id, &world_id)?;
  let wc = load_world_config(&dir);
  let snapshot = BackupSnapshot {
    host_id: wc.host_id.clone(),
    players: wc.players.clone(),
    original_names: wc.original_names.clone(),
    display_name: wc.display_name.clone(),
  };
  let backup_dir = backup_files(&dir, &player_ids, &snapshot)?;
  Ok(backup_dir.to_string_lossy().to_string())
}

#[tauri::command]
fn list_backups(account_id: String, world_id: String) -> Result<Vec<String>, String> {
  let dir = players_dir(&account_id, &world_id)?;
  Ok(list_backups_dir(&dir))
}

#[tauri::command]
fn restore_backup(
  app: AppHandle,
  account_id: String,
  world_id: String,
  backup_name: String,
) -> Result<Vec<Player>, String> {
  let dir = players_dir(&account_id, &world_id)?;
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
        let dest = dir.join(name);
        fs::copy(&file_path, dest).map_err(|err| err.to_string())?;
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

  get_players(app, account_id, world_id)
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

  // ── Build a set of old backup folders to SKIP ──────────────────────
  // Palworld stores game backups in backup/world/<timestamp> and backup/local/<timestamp>.
  // We keep only the most recent subfolder in each category to shrink the ZIP.
  let mut skip_dirs: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

  for sub in &["world", "local"] {
    let bdir = wdir.join("backup").join(sub);
    if bdir.is_dir() {
      if let Ok(rd) = fs::read_dir(&bdir) {
        let mut folders: Vec<PathBuf> = rd
          .filter_map(|e| e.ok())
          .filter(|e| e.path().is_dir())
          .map(|e| e.path())
          .collect();
        // Sort by folder name descending (timestamp format sorts lexicographically)
        folders.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
        // Skip everything except the first (most recent)
        for old in folders.iter().skip(1) {
          skip_dirs.insert(old.clone());
        }
      }
    }
  }

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
      set_host_slot,
      set_player_name,
      reset_player_names,
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
      extract_zip_to_temp
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
