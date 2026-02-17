import { invoke } from "@tauri-apps/api/core";

export type Player = {
  id: string;
  name: string;
  originalId: string;
  isHost: boolean;
};

export type WorldInfo = {
  id: string;
  playerCount: number;
  displayName: string | null;
};

export async function getAccounts(): Promise<string[]> {
  return invoke<string[]>("get_accounts");
}

export async function getWorlds(accountId: string): Promise<string[]> {
  return invoke<string[]>("get_worlds", { accountId });
}

export async function getWorldsWithCounts(
  accountId: string,
): Promise<WorldInfo[]> {
  return invoke<WorldInfo[]>("get_worlds_with_counts", { accountId });
}

export async function getPlayers(
  accountId: string,
  worldId: string,
): Promise<Player[]> {
  return invoke<Player[]>("get_players", { accountId, worldId });
}

export async function setHostPlayer(
  accountId: string,
  worldId: string,
  playerId: string,
): Promise<Player[]> {
  return invoke<Player[]>("set_host_player", { accountId, worldId, playerId });
}

export async function setHostSlot(
  accountId: string,
  worldId: string,
  hostId: string,
): Promise<Player[]> {
  return invoke<Player[]>("set_host_slot", { accountId, worldId, hostId });
}

export async function setPlayerName(
  accountId: string,
  worldId: string,
  playerId: string,
  name: string,
): Promise<Player[]> {
  return invoke<Player[]>("set_player_name", {
    accountId,
    worldId,
    playerId,
    name,
  });
}

export async function resetPlayerNames(
  accountId: string,
  worldId: string,
): Promise<Player[]> {
  return invoke<Player[]>("reset_player_names", { accountId, worldId });
}

export async function swapPlayers(
  accountId: string,
  worldId: string,
  firstId: string,
  secondId: string,
): Promise<Player[]> {
  return invoke<Player[]>("swap_players", {
    accountId,
    worldId,
    firstId,
    secondId,
  });
}

export async function createBackup(
  accountId: string,
  worldId: string,
  players: Player[],
): Promise<string> {
  const playerIds = players.map((player) => player.id);
  return invoke<string>("create_backup", { accountId, worldId, playerIds });
}

export async function listBackups(
  accountId: string,
  worldId: string,
): Promise<string[]> {
  return invoke<string[]>("list_backups", { accountId, worldId });
}

export async function deleteBackup(
  accountId: string,
  worldId: string,
  backupName: string,
): Promise<string[]> {
  return invoke<string[]>("delete_backup", { accountId, worldId, backupName });
}

export async function deleteAllBackups(
  accountId: string,
  worldId: string,
): Promise<string[]> {
  return invoke<string[]>("delete_all_backups", { accountId, worldId });
}

export async function restoreBackup(
  accountId: string,
  worldId: string,
  backupName: string,
): Promise<Player[]> {
  return invoke<Player[]>("restore_backup", { accountId, worldId, backupName });
}

export async function rescanStorage(): Promise<void> {
  await invoke("rescan_storage");
}

export async function setWorldName(
  accountId: string,
  worldId: string,
  name: string,
): Promise<WorldInfo[]> {
  return invoke<WorldInfo[]>("set_world_name", { accountId, worldId, name });
}

export async function resetWorldName(
  accountId: string,
  worldId: string,
): Promise<WorldInfo[]> {
  return invoke<WorldInfo[]>("reset_world_name", { accountId, worldId });
}

// ── World Transfer ──────────────────────────────────

export type ValidatedFolder = {
  name: string;
  path: string;
};

export async function exportWorld(
  accountId: string,
  worldId: string,
  destPath: string,
): Promise<string> {
  return invoke<string>("export_world", { accountId, worldId, destPath });
}

export async function validateWorldFolder(
  folderPath: string,
): Promise<ValidatedFolder> {
  return invoke<ValidatedFolder>("validate_world_folder", { folderPath });
}

export async function checkWorldExists(
  accountId: string,
  worldName: string,
): Promise<boolean> {
  return invoke<boolean>("check_world_exists", { accountId, worldName });
}

export async function importWorld(
  accountId: string,
  folderPath: string,
  mode: string,
  newName?: string,
): Promise<WorldInfo[]> {
  return invoke<WorldInfo[]>("import_world", {
    accountId,
    folderPath,
    mode,
    newName: newName ?? null,
  });
}
