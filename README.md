# 🎮 Palworld Host Switcher

A lightweight desktop application for managing **Palworld** dedicated/co-op save files.
Swap host ownership, rename players, transfer worlds between machines, share worlds peer-to-peer, and create backups — all from a clean, modern UI without touching raw game files.

![Tauri](https://img.shields.io/badge/Tauri-v2.10-blue?logo=tauri)
![React](https://img.shields.io/badge/React-19-61dafb?logo=react)
![TypeScript](https://img.shields.io/badge/TypeScript-5.9-3178c6?logo=typescript)
![Rust](https://img.shields.io/badge/Rust-1.77+-orange?logo=rust)
![Platform](https://img.shields.io/badge/Platform-Windows-0078d4?logo=windows)

---

## ✨ Features

### Player Management

- **Auto-detect accounts & worlds** — scans `%LOCALAPPDATA%\Pal\Saved\SaveGames` automatically
- **Player list** with host indicator (crown icon), player level, pal count, guild name, and last-seen timestamp
- **Drag & drop player swap** — pointer-based reorder in Edit Mode to swap `.sav` files between slots
- **Net-diff swap optimization** — uses a greedy permutation algorithm to compute the minimum number of swaps required to reach the desired arrangement; cycles of length 1 are skipped, and each multi-element cycle uses only `len − 1` swaps
- **Rename players** — assign friendly display names (stored in a JSON sidecar, never touches game files)
- **Reset names** — bulk-reset all player names back to their original slot IDs
- **Automatic host detection** — host is always slot `000…001`; no manual selection required

### World Management

- **World display names** — give each world a custom name shown in the UI (folder name stays unchanged)
- **World transfer (Export)** — export an entire world as a `.zip` archive to share with others; only the most recent backup is included to keep file size small
- **World transfer (Import)** — import a world from a folder or ZIP via file browser or drag & drop from the OS
- **Conflict detection** — importing a world that already exists lets you choose to replace or create a copy with a new name
- **Smart import merge** — when replacing an existing world, backups from both the existing and imported world are preserved and merged together

### GVAS Save Parser (Pure Rust)

The application includes a **complete, zero-dependency GVAS parser** (Unreal Engine 4/5 binary save format) implemented entirely in Rust (~2 400 lines). This is used to read and modify `Level.sav` and individual player `.sav` files.

- **Full round-trip** — parse `.sav` → JSON → `.sav` with bit-perfect output
- **All GVAS property types** — `BoolProperty`, `IntProperty`, `Int64Property`, `FloatProperty`, `DoubleProperty`, `StrProperty`, `NameProperty`, `TextProperty`, `EnumProperty`, `StructProperty`, `ArrayProperty`, `MapProperty`, `SetProperty`, `SoftObjectProperty`, `ObjectProperty`
- **Struct sub-types** — `Guid`, `DateTime`, `Timespan`, `Vector`, `Quat`, `LinearColor`, `Rotator`, and generic key-value structs
- **Compression** — transparent handling of PLZ (double-zlib) and PLM (Oodle) compressed saves
- **Player extraction from Level.sav** — reads `CharacterSaveParameterMap` and `GroupSaveDataMap` to extract player name, level, pal count, guild name, last-online ticks, and ownership UIDs
- **Deep UID swap** — bidirectional UUID swap across `CharacterSaveParameterMap`, `GroupSaveDataMap`, ownership fields, and the full JSON tree via `deep_swap_uids`
- **Player .sav patching** — `modify_player_sav` rewrites the `PlayerUId` and `IndividualId.PlayerUId` inside each player's personal save file
- **4 unit tests** — `test_plz_roundtrip`, `test_decompress_level_sav`, `test_parse_level_sav_to_json`, `test_roundtrip_level_sav`

### P2P World Transfer

- **Peer-to-peer sharing** — share worlds directly between two PCs over the internet using WebRTC (no server upload needed)
- **Sender flow** — click **Share**, get a 6-character code, and share it with the receiver
- **Receiver flow** — enter the code, choose where to save the ZIP, and the world transfers directly from the sender's PC
- **Progress tracking** — real-time progress bar and status messages for both sender and receiver
- **Auto-import** — received worlds are automatically extracted and presented for import into your game
- **Metered.ca TURN relay** — integrated [Metered.ca](https://www.metered.ca/) TURN support for reliable connections across NAT/CGNAT/mobile networks (free tier: 50 GB/month)
- **TURN test button** — one-click connectivity test that verifies relay candidates are reachable before transferring
- **ICE debug logging** — connection state, ICE gathering state, and signaling state changes are logged to the activity console
- **Copy API key** — one-click copy button to easily share your Metered API key with friends
- **Credential persistence** — Metered domain and API key are saved in localStorage and restored automatically on next launch

> **Important:** P2P transfers require a TURN relay to work across different networks. Each user must create a free [Metered.ca](https://www.metered.ca/signup) account and enter their credentials in the app before sharing or receiving worlds. See [P2P TURN Configuration](#-p2p-turn-configuration) below.

### Backup System

- **Create backups** — snapshot all player `.sav` files plus the full config state (host, names, display name)
- **Restore backups** — restore any previous snapshot, overwriting current files
- **Delete backups** — remove individual backups or wipe all at once
- **Auto-backup** — optionally create a backup before every destructive operation (host swap, player swap)

### UI / UX

- **Dark theme** — modern dark interface with accent colors
- **Splash screen** — instant dark splash loader prevents white flash on startup
- **Game-running safety lock** — detects if Palworld is running and blocks all operations with a full-screen overlay to prevent save corruption
- **Non-blocking heavy operations** — all save parsing, swap, and restore commands run on a background thread via `spawn_blocking` to keep the UI responsive
- **Real-time swap progress** — granular progress events emitted from Rust at ~10 stages per swap (read → parse → modify UIDs → serialize → write Level.sav → patch player .savs → rename files); displayed as a percentage and per-step checklist in the overlay
- **Scanning overlay** — spinner + message while Level.sav is being read (no fake progress bar)
- **Resizable sidebar** — drag the divider to resize the sidebar
- **Activity log** — collapsible console showing timestamped operation history
- **Toast notifications** — non-blocking success/error/info popups
- **Progress bars** — real-time progress for export, import, and P2P transfer operations
- **Search** — filter players by name or ID
- **Rescan** — manually re-scan the save folder to pick up external changes (added/deleted worlds)

---

## 🏗️ Tech Stack

| Layer           | Technology                | Purpose                                               |
| --------------- | ------------------------- | ----------------------------------------------------- |
| **Frontend**    | React 19 + TypeScript 5.9 | UI components, state management                       |
| **Bundler**     | Vite 7                    | Dev server, HMR, production build                     |
| **Backend**     | Rust (Tauri v2)           | File system operations, GVAS parsing, ZIP, IPC        |
| **Desktop**     | Tauri v2.10               | Native window, installer generation, OS drag & drop   |
| **Save parser** | Custom GVAS module (Rust) | Unreal Engine .sav read/write, UID swaps, Level.sav   |
| **Compression** | zlib (flate2) + Oodle FFI | PLZ double-zlib and PLM Oodle decompression           |
| **Dialogs**     | `tauri-plugin-dialog`     | Native file/folder pickers, confirm dialogs           |
| **ZIP**         | `zip` crate (Rust)        | World export/import as `.zip`                         |
| **File walk**   | `walkdir` crate           | Recursive directory traversal for export              |
| **P2P**         | PeerJS (WebRTC)           | Direct peer-to-peer world transfer between PCs        |
| **TURN relay**  | Metered.ca API            | TURN credentials for NAT traversal (free 50 GB/month) |
| **Logging**     | `tauri-plugin-log`        | Structured log output with target-based filtering     |

---

## 📁 Project Structure

```
palworld-host-switcher/
├── src/                          # Frontend (React + TypeScript)
│   ├── App.tsx                   # Main component — all UI logic (~2 500 lines)
│   ├── App.css                   # Complete stylesheet, dark theme (~1 900 lines)
│   ├── index.css                 # Base/reset styles
│   ├── main.tsx                  # React entry point
│   └── services/
│       ├── palworldService.ts    # Tauri IPC invoke wrappers
│       └── p2pService.ts         # WebRTC P2P file transfer (PeerJS)
├── src-tauri/                    # Rust backend
│   ├── src/
│   │   ├── lib.rs                # Tauri commands & file logic (~1 660 lines)
│   │   ├── gvas.rs               # GVAS save parser / serializer (~2 430 lines)
│   │   ├── oodle.rs              # Oodle DLL FFI wrapper for PLM saves
│   │   └── main.rs               # Binary entry point
│   ├── tauri.conf.json           # App config (window, bundle, permissions)
│   ├── Cargo.toml                # Rust dependencies
│   ├── build.rs                  # Cargo build script
│   ├── icons/                    # Generated app icons (all sizes)
│   └── capabilities/             # Tauri v2 permission capabilities
├── public/                       # Static assets
├── index.html                    # Entry point (includes inline splash loader)
├── package.json                  # Node dependencies & scripts
├── vite.config.ts                # Vite configuration
├── tsconfig.json                 # TypeScript config (project references)
├── tsconfig.app.json             # TypeScript config for app sources
├── tsconfig.node.json            # TypeScript config for Vite/node
├── eslint.config.js              # ESLint flat config
└── .github/
    └── copilot-instructions.md   # AI assistant instructions
```

---

## 📦 Data Storage

The app operates on Palworld's save game directory:

```
%LOCALAPPDATA%\Pal\Saved\SaveGames\
  └── <SteamAccountID>/
      └── <WorldID>/
          ├── Players/
          │   ├── 00000000000000000000000000000001.sav
          │   ├── <PlayerID>.sav
          │   ├── host_switcher.json    ← app config (per-world)
          │   └── backup/
          │       └── 2026-02-17_14-30-00/
          │           ├── *.sav          ← backed up player files
          │           └── config_snapshot.json
          ├── LevelMeta.sav
          └── Level.sav
```

### `host_switcher.json` (per-world config)

Stored **inside the world folder**, so it travels with the world when exported/shared:

```json
{
  "host_id": "00000000000000000000000000000001",
  "players": {
    "00000000000000000000000000000001": "Alex",
    "612decda000000000000000000000000": "Sam"
  },
  "original_names": {
    "00000000000000000000000000000001": "00000000000000000000000000000001",
    "612decda000000000000000000000000": "612decda000000000000000000000000"
  },
  "display_name": "My Main World"
}
```

---

## � Download

Pre-built Windows installer (no build tools required):

**[⬇️ Download Palworld Host Switcher](https://drive.google.com/drive/u/0/folders/1iUfwsw6elrihbwZLED2J0P3P0r_QxtYa)**

Download the `.msi` or `_setup.exe` file, run the installer, and launch from the Start Menu.

---

## 🚀 Getting Started (Development)

### Prerequisites

- **Node.js** ≥ 18 — [Download](https://nodejs.org/)
- **Rust** ≥ 1.77 — [Install via rustup](https://rustup.rs/)
- **Visual Studio Build Tools** (Windows) — C++ build tools required by Tauri ([Setup guide](https://tauri.app/start/prerequisites/))

### Clone the Repository

```bash
git clone https://github.com/MollyLaMolla/palworld-host-switcher.git
cd palworld-host-switcher
```

### Install Dependencies

```bash
npm install
```

### Run in Development Mode

```bash
npx tauri dev
```

This starts both the Vite dev server (with HMR) and the Tauri native window.

---

## 🔌 P2P TURN Configuration

P2P world transfers require a **TURN relay** to work reliably across different networks (NAT, CGNAT, mobile data, etc.). The app uses [Metered.ca](https://www.metered.ca/) as TURN provider.

### Setup (required for each user)

1. **Create a free account** at [metered.ca/signup](https://www.metered.ca/signup) — the free plan includes **50 GB/month**
2. In your Metered dashboard, copy your **App Domain** (e.g. `yourapp.metered.live`) and **API Key**
3. Open the app → expand **P2P Transfer** in the sidebar → paste your domain and API key
4. Click **Save** — credentials are stored locally and persist across sessions
5. Click **Test TURN** to verify connectivity (you should see `TURN relay OK`)

Once configured, you can use the **copy button** next to the API key field to share your credentials with a friend who plays on the same Metered account.

> **Note:** Share and Receive buttons are disabled until valid Metered credentials are configured.

### Advanced: self-hosted coturn

If you prefer to self-host, you can also configure a custom coturn server via environment variables:

```bash
# Custom TURN server (coturn with use-auth-secret)
VITE_P2P_TURN_URL='turn:your-host:3478?transport=udp,turn:your-host:3478?transport=tcp'
VITE_P2P_TURN_SECRET='your-shared-secret'
```

The app generates ephemeral HMAC-SHA1 credentials from the shared secret automatically.

### Build for Production

```bash
npx tauri build
```

Output installers are generated in:

```
src-tauri/target/release/bundle/
  ├── msi/   → Palworld Host Switcher_0.1.0_x64_en-US.msi
  └── nsis/  → Palworld Host Switcher_0.1.0_x64-setup.exe
```

Double-click either installer to install the app. It will appear in your Start Menu as **Palworld Host Switcher**.

---

## 🖥️ Usage

1. **Launch the app** — double-click the installed app or run `npx tauri dev`
2. **Close Palworld first** — if the game is running, a safety overlay blocks all operations to prevent save corruption
3. **Select Account** — your Steam account ID is auto-detected from the sidebar dropdown
4. **Select World** — pick the world you want to manage
5. **Rename the world** _(optional)_ — click the ✏️ pencil icon above the player list to set a friendly name
6. **View players** — all players with `.sav` files appear as cards showing name, level, pal count, guild, and last-seen time
7. **Swap players** — click **Edit**, then drag player cards to rearrange; click **Save** to apply the minimal set of swaps computed by the net-diff algorithm
8. **Rename players** — click the pencil icon on a player card to set a display name
9. **Backup / Restore** — use the sidebar Backup section before making changes
10. **Export / Import world** — use the sidebar World Transfer section to share worlds as ZIP files
11. **P2P Transfer** — share a world directly with another PC: click **Share** to get a code, or enter a code to **Receive**
12. **Rescan** — click the Rescan button to refresh if you've made changes outside the app

---

## 🐛 Known Issues & Resolutions

| Issue                                                       | Root Cause                                                                                     | Resolution                                                                                        |
| ----------------------------------------------------------- | ---------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| Export freezes the UI                                       | ZIP compression ran on the main thread, blocking the event loop                                | Moved to `tauri::async_runtime::spawn_blocking()` with progress events throttled to ≥ 2% changes  |
| Drag-and-drop for file import breaks player swap drag       | Tauri's `dragDropEnabled: true` intercepts ALL drag events via OLE handler, blocking HTML5 DnD | Replaced HTML5 drag events with **pointer events** (`onPointerDown/Move/Up`) for player swaps     |
| Drag-dropped import doesn't detect existing world conflicts | `onDragDropEvent` closure captures stale `accountId` (empty deps `[]`)                         | Added `accountIdRef` (useRef) to always read the current value in async callbacks                 |
| Player names lost after world import                        | UI didn't refresh players/backups after import when `worldId` stayed the same                  | Explicitly re-fetch players, host slot, and backups after successful import                       |
| Nested folder detection for imports                         | Users often zip the world inside an extra parent folder                                        | `validate_world_folder` auto-detects and resolves same-name subfolder or single-subfolder nesting |
| Backup didn't preserve world display name                   | `BackupSnapshot` didn't include `display_name`                                                 | Added `display_name` to snapshot with `#[serde(default)]` for backward compatibility              |
| Console window flash every 3 seconds                        | `tasklist` process detection opened visible CMD windows in production                          | Added `CREATE_NO_WINDOW` flag (0x08000000) via `.creation_flags()` on Windows                     |
| White flash on app startup                                  | React takes a moment to mount, showing a blank white page                                      | Inline dark splash loader in `index.html` with spinner, fades out once React mounts               |
| Missing permissions in production build                     | Tauri v2 capabilities not configured for events, webview, and dialog in production             | Added `core:event`, `core:webview`, and `dialog` permissions to `capabilities/default.json`       |
| Exported ZIP too large                                      | All backup history included in export                                                          | Only the most recent backup subfolder is included; older ones are skipped                         |
| Rescan doesn't detect deleted worlds                        | Rescan only refreshed accounts, didn't cascade re-fetch worlds/players/backups                 | Rescan now fully re-loads accounts → worlds → players → backups from disk                         |
| P2P transfer times out between different networks           | Only STUN servers configured; STUN fails with symmetric NAT / CGNAT / mobile networks          | Integrated Metered.ca TURN relay with automatic credential fetching and 10-min caching            |
| P2P fails silently with no relay candidates                 | Free Open Relay Project servers had invalid/expired credentials                                | Replaced with Metered.ca API; each user configures their own account; Test TURN button to verify  |
| `json_to_sav` produces corrupted files                      | Size field written included the header bytes, causing the game to reject the file              | Fixed `write_property_inner` to return only the data-only size; header written by the caller      |
| Host detection wrong after swaps                            | Config-based host ID drifted when `.sav` files were renamed                                    | Host is now always the player whose filename is slot `000…001`; no config needed                  |
| UI freezes during scanning / swapping                       | Heavy GVAS parsing ran on the Tauri main thread                                                | All heavy commands (`get_players`, `swap_players`, `set_host_player`, `restore_backup`) use `spawn_blocking` |
| Unnecessary swaps when reordering many players              | Each adjacent drag was counted as a separate swap                                              | Net-diff algorithm computes the minimum permutation difference using greedy cycle decomposition   |
| TAO window-manager warnings flooding console                | `tao` crate emitting debug messages about unhandled WM events                                  | Added `tauri_plugin_log` filter: `.filter(\|m\| !m.target().starts_with("tao::"))`              |
| Player Level and Pals count always 0                        | `decode_character_rawdata` double-wrapped `SaveParameter`, making properties unreachable at the expected JSON path | Flattened decoder output to `{"object": props}` directly; updated encoder to match              |
| Level field not parsed from ByteProperty                    | `Level` is a `ByteProperty` with double-nested value (`{value:{type,value}}`), code only did one `.get("value")` | Changed extraction to `.get("value").get("value").as_u64()` to reach the inner numeric value   |
| Last Seen always showing "Online now"                       | `GameTimeSaveData` was in the `is_skip_path` list, stored as raw blob instead of parsed JSON    | Removed from skip list so `RealDateTimeTicks` is parsed and `current_ticks` reads correctly     |

---

## 🔧 Backend Commands (IPC)

All commands exposed via `tauri::generate_handler!`:

| Command                                                 | Description                                                        |
| ------------------------------------------------------- | ------------------------------------------------------------------ |
| `get_accounts`                                          | List Steam account IDs from SaveGames folder                       |
| `get_worlds` / `get_worlds_with_counts`                 | List worlds with player counts and display names                   |
| `get_players`                                           | List players with name, level, pals, guild, last-seen (async)      |
| `set_host_player`                                       | Swap a player into host slot `000…001` (modifies Level.sav + .savs)|
| `swap_players`                                          | Swap two players' slot data (Level.sav + .sav files, with progress)|
| `set_player_name` / `reset_player_names`                | Rename a player / reset all names to slot IDs                      |
| `set_world_name` / `reset_world_name`                   | Set or clear a world's display name                                |
| `create_backup` / `restore_backup`                      | Create or restore a full snapshot (async for restore)              |
| `list_backups` / `delete_backup` / `delete_all_backups` | Manage backup history                                              |
| `export_world`                                          | Export world as `.zip` (async, with progress events)               |
| `validate_world_folder`                                 | Validate a folder structure as a valid Palworld world              |
| `check_world_exists`                                    | Check if a world name already exists under an account              |
| `import_world`                                          | Import a world folder (replace or copy, with backup merge)         |
| `rescan_storage`                                        | Force re-scan of the SaveGames directory                           |
| `is_palworld_running`                                   | Detect if Palworld is running (silent, no console window)          |
| `export_world_to_temp`                                  | Export world to a temp ZIP for P2P sharing                         |
| `get_file_size` / `read_file_chunk`                     | Read binary file data in chunks (for P2P transfer)                 |
| `append_file_chunk_b64`                                 | Append base64-encoded chunk to a file (P2P receiver)               |
| `get_temp_path` / `delete_temp_file`                    | Manage temporary files                                             |
| `extract_zip_to_temp`                                   | Extract a received ZIP to a temp folder for validation             |

---

## 📄 License

This project is provided as-is for personal use.
Palworld is a trademark of Pocketpair, Inc. This tool is not affiliated with or endorsed by Pocketpair.

---

## 🤝 Contributing

1. Fork the repository
2. Create a feature branch: `git checkout -b feature/my-feature`
3. Commit your changes: `git commit -m "Add my feature"`
4. Push to the branch: `git push origin feature/my-feature`
5. Open a Pull Request
