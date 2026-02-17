import { useCallback, useEffect, useRef, useState } from "react";
import "./App.css";
import { save, open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  createBackup,
  checkWorldExists,
  deleteBackup,
  deleteAllBackups,
  exportWorld,
  getAccounts,
  getPlayers,
  getWorldsWithCounts,
  importWorld,
  listBackups,
  rescanStorage,
  setHostPlayer,
  setHostSlot,
  setPlayerName,
  resetPlayerNames,
  resetWorldName,
  restoreBackup,
  setWorldName,
  swapPlayers,
  validateWorldFolder,
  isPalworldRunning,
  exportWorldToTemp,
  deleteTempFile,
  type Player,
  type WorldInfo,
} from "./services/palworldService";
import {
  startSending,
  startReceiving,
  cancelP2P,
  type P2PStatus,
} from "./services/p2pService";

/* ── SVG Icons ─────────────────────────────────────────── */
const s = {
  display: "inline-block",
  verticalAlign: "middle",
  flexShrink: 0,
} as const;

const IconGamepad = ({ size = 16 }: { size?: number }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    style={s}>
    <line x1="6" y1="11" x2="10" y2="11" />
    <line x1="8" y1="9" x2="8" y2="13" />
    <line x1="15" y1="12" x2="15.01" y2="12" />
    <line x1="18" y1="10" x2="18.01" y2="10" />
    <path d="M17.32 5H6.68a4 4 0 0 0-3.978 3.59c-.006.052-.01.101-.017.152C2.604 9.416 2 14.456 2 16a3 3 0 0 0 3 3c1 0 1.5-.5 2-1l1.414-1.414A2 2 0 0 1 9.828 16h4.344a2 2 0 0 1 1.414.586L17 18c.5.5 1 1 2 1a3 3 0 0 0 3-3c0-1.545-.604-6.584-.685-7.258-.007-.05-.011-.1-.017-.151A4 4 0 0 0 17.32 5z" />
  </svg>
);

const IconChevron = ({ open, size = 12 }: { open: boolean; size?: number }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2.5"
    strokeLinecap="round"
    strokeLinejoin="round"
    style={{
      ...s,
      transition: "transform 150ms ease",
      transform: open ? "rotate(90deg)" : "rotate(0deg)",
    }}>
    <polyline points="9 6 15 12 9 18" />
  </svg>
);

const IconRefresh = ({ size = 14 }: { size?: number }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    style={s}>
    <polyline points="23 4 23 10 17 10" />
    <polyline points="1 20 1 14 7 14" />
    <path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15" />
  </svg>
);

const IconPlus = ({ size = 14 }: { size?: number }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2.5"
    strokeLinecap="round"
    strokeLinejoin="round"
    style={s}>
    <line x1="12" y1="5" x2="12" y2="19" />
    <line x1="5" y1="12" x2="19" y2="12" />
  </svg>
);

const IconTerminal = ({ size = 14 }: { size?: number }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    style={s}>
    <polyline points="4 17 10 11 4 5" />
    <line x1="12" y1="19" x2="20" y2="19" />
  </svg>
);

const IconCrown = ({ size = 14 }: { size?: number }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    style={s}>
    <path d="M2 4l3 12h14l3-12-5 4-5-4-5 4z" />
    <line x1="5" y1="20" x2="19" y2="20" />
  </svg>
);

const IconUser = ({ size = 14 }: { size?: number }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    style={s}>
    <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" />
    <circle cx="12" cy="7" r="4" />
  </svg>
);

const IconPencil = ({ size = 12 }: { size?: number }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    style={s}>
    <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" />
    <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z" />
  </svg>
);

const IconX = ({ size = 14 }: { size?: number }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2.5"
    strokeLinecap="round"
    strokeLinejoin="round"
    style={s}>
    <line x1="18" y1="6" x2="6" y2="18" />
    <line x1="6" y1="6" x2="18" y2="18" />
  </svg>
);

const IconTrash = ({ size = 14 }: { size?: number }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    style={s}>
    <polyline points="3 6 5 6 21 6" />
    <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
  </svg>
);

const IconCheck = ({ size = 12 }: { size?: number }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="3"
    strokeLinecap="round"
    strokeLinejoin="round"
    style={s}>
    <polyline points="20 6 9 17 4 12" />
  </svg>
);

const IconInfo = ({ size = 12 }: { size?: number }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    style={s}>
    <circle cx="12" cy="12" r="10" />
    <line x1="12" y1="16" x2="12" y2="12" />
    <line x1="12" y1="8" x2="12.01" y2="8" />
  </svg>
);

const IconAlert = ({ size = 12 }: { size?: number }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    style={s}>
    <circle cx="12" cy="12" r="10" />
    <line x1="15" y1="9" x2="9" y2="15" />
    <line x1="9" y1="9" x2="15" y2="15" />
  </svg>
);

const IconDrag = ({ size = 14 }: { size?: number }) => {
  const s: React.CSSProperties = { width: size, height: size, flexShrink: 0 };
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" style={s}>
      <circle cx="9" cy="5" r="2" />
      <circle cx="15" cy="5" r="2" />
      <circle cx="9" cy="12" r="2" />
      <circle cx="15" cy="12" r="2" />
      <circle cx="9" cy="19" r="2" />
      <circle cx="15" cy="19" r="2" />
    </svg>
  );
};

const IconSwap = ({ size = 14 }: { size?: number }) => {
  const s: React.CSSProperties = { width: size, height: size, flexShrink: 0 };
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      style={s}>
      <polyline points="17 1 21 5 17 9" />
      <path d="M3 11V9a4 4 0 0 1 4-4h14" />
      <polyline points="7 23 3 19 7 15" />
      <path d="M21 13v2a4 4 0 0 1-4 4H3" />
    </svg>
  );
};

const IconDownload = ({ size = 14 }: { size?: number }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    style={s}>
    <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
    <polyline points="7 10 12 15 17 10" />
    <line x1="12" y1="15" x2="12" y2="3" />
  </svg>
);

const IconUpload = ({ size = 14 }: { size?: number }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    style={s}>
    <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
    <polyline points="17 8 12 3 7 8" />
    <line x1="12" y1="3" x2="12" y2="15" />
  </svg>
);

const IconFolder = ({ size = 14 }: { size?: number }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    style={s}>
    <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
  </svg>
);

const IconShare = ({ size = 14 }: { size?: number }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    style={s}>
    <circle cx="18" cy="5" r="3" />
    <circle cx="6" cy="12" r="3" />
    <circle cx="18" cy="19" r="3" />
    <line x1="8.59" y1="13.51" x2="15.42" y2="17.49" />
    <line x1="15.41" y1="6.51" x2="8.59" y2="10.49" />
  </svg>
);

const IconCopy = ({ size = 14 }: { size?: number }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    style={s}>
    <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
    <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
  </svg>
);

/* ── Collapsible section ───────────────────────────────── */
function Section({
  title,
  defaultOpen = true,
  children,
}: {
  title: string;
  defaultOpen?: boolean;
  children: React.ReactNode;
}) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className={`sidebar__section${open ? "" : " collapsed"}`}>
      <button
        className="sidebar__toggle"
        onClick={() => setOpen((v) => !v)}
        type="button">
        <IconChevron open={open} />
        <span className="sidebar__title">{title}</span>
      </button>
      {open && <div className="sidebar__body">{children}</div>}
    </div>
  );
}

function App() {
  const [accounts, setAccounts] = useState<string[]>([]);
  const [worlds, setWorlds] = useState<WorldInfo[]>([]);
  const [players, setPlayers] = useState<Player[]>([]);
  const [accountId, setAccountId] = useState("");
  const [worldId, setWorldId] = useState("");
  const [hostSlot, setHostSlotState] = useState("");
  const [backups, setBackups] = useState<string[]>([]);
  const [backupTarget, setBackupTarget] = useState("");
  const [nameEdits, setNameEdits] = useState<Record<string, string>>({});
  const [searchText, setSearchText] = useState("");
  const [autoBackup, setAutoBackup] = useState(true);
  const [toasts, setToasts] = useState<
    { id: string; text: string; type: "success" | "error" | "info" }[]
  >([]);
  const [logs, setLogs] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [editingPlayerId, setEditingPlayerId] = useState<string | null>(null);

  /* ── P2P Transfer state ──────────────────────────────── */
  const [p2pStatus, setP2pStatus] = useState<P2PStatus>("idle");
  const [p2pMessage, setP2pMessage] = useState("");
  const [p2pProgress, setP2pProgress] = useState(0);
  const [p2pCode, setP2pCode] = useState("");
  const [p2pReceiveCode, setP2pReceiveCode] = useState("");
  const [p2pCopied, setP2pCopied] = useState(false);
  const [p2pTempZip, setP2pTempZip] = useState<string | null>(null);

  /* ── World name editing ────────────────────────────── */
  const [editingWorldName, setEditingWorldName] = useState(false);
  const [worldNameDraft, setWorldNameDraft] = useState("");

  /* ── Drag-and-drop swap ────────────────────────────── */
  const [editMode, setEditMode] = useState(false);
  const [dragSourceId, setDragSourceId] = useState<string | null>(null);
  const [dragOverId, setDragOverId] = useState<string | null>(null);
  const [pendingSwaps, setPendingSwaps] = useState<
    { fromId: string; toId: string }[]
  >([]);
  const dragSourceRef = useRef<string | null>(null);
  const originalPlayersRef = useRef<Player[]>([]);

  /* Ref to always have current accountId in async callbacks */
  const accountIdRef = useRef(accountId);
  useEffect(() => {
    accountIdRef.current = accountId;
  }, [accountId]);

  /* ── Dismiss splash loader once React has mounted ──── */
  useEffect(() => {
    const splash = document.getElementById("splash-loader");
    if (splash) {
      splash.classList.add("fade-out");
      const timer = setTimeout(() => splash.remove(), 200);
      return () => clearTimeout(timer);
    }
  }, []);

  /* ── Palworld process detection ────────────────────── */
  const [gameRunning, setGameRunning] = useState(false);

  useEffect(() => {
    let active = true;
    const check = async () => {
      try {
        const running = await isPalworldRunning();
        if (active) setGameRunning(running);
      } catch {
        // ignore
      }
    };
    check();
    const id = window.setInterval(check, 3000);
    return () => {
      active = false;
      window.clearInterval(id);
    };
  }, []);

  /* ── Custom confirm modal ──────────────────────────── */
  const [confirmModal, setConfirmModal] = useState<{
    message: string;
    onConfirm: () => void;
  } | null>(null);

  const showConfirm = (message: string): Promise<boolean> => {
    return new Promise((resolve) => {
      setConfirmModal({
        message,
        onConfirm: () => {
          setConfirmModal(null);
          resolve(true);
        },
      });
      // If user cancels, we resolve false via the cancel handler
      const cancel = () => {
        setConfirmModal(null);
        resolve(false);
      };
      // Store cancel on a ref so it can be called from JSX
      confirmCancelRef.current = cancel;
    });
  };
  const confirmCancelRef = useRef<(() => void) | null>(null);

  /* ── Sidebar resize ────────────────────────────────── */
  const [sidebarWidth, setSidebarWidth] = useState(280);
  const sidebarDragging = useRef(false);
  const sidebarStartX = useRef(0);
  const sidebarStartW = useRef(280);

  const onSidebarMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      sidebarDragging.current = true;
      sidebarStartX.current = e.clientX;
      sidebarStartW.current = sidebarWidth;
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
    },
    [sidebarWidth],
  );

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (!sidebarDragging.current) return;
      const delta = e.clientX - sidebarStartX.current;
      const next = Math.min(Math.max(sidebarStartW.current + delta, 200), 480);
      setSidebarWidth(next);
    };
    const onUp = () => {
      if (!sidebarDragging.current) return;
      sidebarDragging.current = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, []);

  /* ── Console panel resize ──────────────────────────── */
  const [consoleOpen, setConsoleOpen] = useState(false);
  const [consoleHeight, setConsoleHeight] = useState(180);
  const consoleDragging = useRef(false);
  const consoleStartY = useRef(0);
  const consoleStartH = useRef(180);

  const onConsoleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      consoleDragging.current = true;
      consoleStartY.current = e.clientY;
      consoleStartH.current = consoleHeight;
      document.body.style.cursor = "row-resize";
      document.body.style.userSelect = "none";
    },
    [consoleHeight],
  );

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (!consoleDragging.current) return;
      const delta = consoleStartY.current - e.clientY;
      const next = Math.min(Math.max(consoleStartH.current + delta, 80), 500);
      setConsoleHeight(next);
    };
    const onUp = () => {
      if (!consoleDragging.current) return;
      consoleDragging.current = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, []);

  /* Prevent browser default drag for internal elements in edit mode */
  useEffect(() => {
    const prevent = (e: DragEvent) => {
      if (editMode) {
        e.preventDefault();
        e.stopPropagation();
      }
    };
    document.addEventListener("dragover", prevent, true);
    document.addEventListener("drop", prevent, true);
    document.addEventListener("dragstart", prevent, true);
    return () => {
      document.removeEventListener("dragover", prevent, true);
      document.removeEventListener("drop", prevent, true);
      document.removeEventListener("dragstart", prevent, true);
    };
  }, [editMode]);

  useEffect(() => {
    getAccounts().then((items) => {
      setAccounts(items);
      setAccountId(items[0] ?? "");
    });
  }, []);

  useEffect(() => {
    if (!accountId) {
      setWorlds([]);
      setWorldId("");
      return;
    }
    getWorldsWithCounts(accountId).then((items) => {
      setWorlds(items);
      setWorldId(items[0]?.id ?? "");
    });
  }, [accountId]);

  useEffect(() => {
    if (!accountId || !worldId) {
      setPlayers([]);
      return;
    }
    getPlayers(accountId, worldId).then((items) => {
      setPlayers(items);
      setHostSlotState(items.find((player) => player.isHost)?.id ?? "");
      const nextEdits: Record<string, string> = {};
      items.forEach((player) => {
        nextEdits[player.id] = player.name;
      });
      setNameEdits(nextEdits);
    });
    listBackups(accountId, worldId).then((items) => {
      setBackups(items);
      setBackupTarget(items[0] ?? "");
    });
  }, [accountId, worldId]);

  const pushLog = (message: string) => {
    const now = new Date();
    const ts = [now.getHours(), now.getMinutes(), now.getSeconds()]
      .map((n) => String(n).padStart(2, "0"))
      .join(":");
    setLogs((prev) => [`[${ts}] ${message}`, ...prev].slice(0, 50));
  };

  const pushToast = (
    text: string,
    type: "success" | "error" | "info" = "success",
  ) => {
    const id = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
    setToasts((prev) => [{ id, text, type }, ...prev].slice(0, 4));
    window.setTimeout(() => {
      setToasts((prev) => prev.filter((toast) => toast.id !== id));
    }, 3200);
  };

  const handleAutoBackup = async (actionLabel: string) => {
    if (!autoBackup || !players.length) {
      return;
    }
    const backupPath = await createBackup(accountId, worldId, players);
    pushLog(`Backup created: ${backupPath}.`);
    pushToast(`Backup created for ${actionLabel}.`, "info");
    const items = await listBackups(accountId, worldId);
    setBackups(items);
    setBackupTarget(items[0] ?? "");
  };

  const handleRescan = async () => {
    setLoading(true);
    try {
      await rescanStorage();

      // Re-fetch everything from disk, exactly like app startup
      const accs = await getAccounts();
      setAccounts(accs);
      const selAccount = accs[0] ?? "";
      setAccountId(selAccount);

      if (selAccount) {
        const ws = await getWorldsWithCounts(selAccount);
        setWorlds(ws);
        const selWorld = ws[0]?.id ?? "";
        setWorldId(selWorld);

        if (selWorld) {
          const ps = await getPlayers(selAccount, selWorld);
          setPlayers(ps);
          setHostSlotState(ps.find((p) => p.isHost)?.id ?? "");
          const nextEdits: Record<string, string> = {};
          ps.forEach((p) => {
            nextEdits[p.id] = p.name;
          });
          setNameEdits(nextEdits);

          const bk = await listBackups(selAccount, selWorld);
          setBackups(bk);
          setBackupTarget(bk[0] ?? "");
        } else {
          setPlayers([]);
          setBackups([]);
          setBackupTarget("");
        }
      } else {
        setWorlds([]);
        setWorldId("");
        setPlayers([]);
        setBackups([]);
        setBackupTarget("");
      }

      pushLog("Storage rescan complete.");
      pushToast("Storage rescan complete.");
    } catch (err) {
      pushLog(`Error: ${err}`);
      pushToast(`Rescan failed: ${err}`, "error");
    } finally {
      setLoading(false);
    }
  };

  const handleSetHostSlot = async () => {
    if (!hostSlot) {
      return;
    }
    setLoading(true);
    try {
      const updated = await setHostSlot(accountId, worldId, hostSlot);
      setPlayers(updated);
      const nextEdits: Record<string, string> = {};
      updated.forEach((player) => {
        nextEdits[player.id] = player.name;
      });
      setNameEdits(nextEdits);
      pushLog(`Host slot set to ${hostSlot}.`);
      pushToast("Host slot updated.");
    } catch (err) {
      pushLog(`Error setting host slot: ${err}`);
      pushToast(`Host slot failed: ${err}`, "error");
    } finally {
      setLoading(false);
    }
  };

  const handleBackup = async () => {
    setLoading(true);
    try {
      const backupPath = await createBackup(accountId, worldId, players);
      pushLog(`Backup created: ${backupPath}.`);
      pushToast("Backup created.");
      const items = await listBackups(accountId, worldId);
      setBackups(items);
      setBackupTarget(items[0] ?? "");
    } catch (err) {
      pushLog(`Error creating backup: ${err}`);
      pushToast(`Backup failed: ${err}`, "error");
    } finally {
      setLoading(false);
    }
  };

  const handleRestoreBackup = async () => {
    if (!backupTarget) {
      return;
    }
    const ok = await showConfirm(
      "Restore this backup? Current files will be overwritten.",
    );
    if (!ok) {
      return;
    }
    setLoading(true);
    try {
      const updated = await restoreBackup(accountId, worldId, backupTarget);
      setPlayers(updated);
      setHostSlotState(updated.find((player) => player.isHost)?.id ?? "");
      const nextEdits: Record<string, string> = {};
      updated.forEach((player) => {
        nextEdits[player.id] = player.name;
      });
      setNameEdits(nextEdits);
      // Refresh worlds list (display_name may have changed)
      const refreshedWorlds = await getWorldsWithCounts(accountId);
      setWorlds(refreshedWorlds);
      pushLog(`Backup restored: ${backupTarget}.`);
      pushToast("Backup restored.");
    } catch (err) {
      pushLog(`Error restoring backup: ${err}`);
      pushToast(`Restore failed: ${err}`, "error");
    } finally {
      setLoading(false);
    }
  };

  const handleNameChange = (id: string, value: string) => {
    setNameEdits((prev) => ({ ...prev, [id]: value }));
  };

  const handleDeleteBackup = async (name: string) => {
    const ok = await showConfirm(`Delete backup "${name}"?`);
    if (!ok) return;
    setLoading(true);
    try {
      const items = await deleteBackup(accountId, worldId, name);
      setBackups(items);
      setBackupTarget(items[0] ?? "");
      pushLog(`Backup deleted: ${name}.`);
      pushToast("Backup deleted.");
    } catch (err) {
      pushLog(`Error deleting backup: ${err}`);
      pushToast(`Delete failed: ${err}`, "error");
    } finally {
      setLoading(false);
    }
  };

  const handleDeleteAllBackups = async () => {
    const ok = await showConfirm(
      "Delete ALL backups for this world? This action cannot be undone.",
    );
    if (!ok) return;
    setLoading(true);
    try {
      const items = await deleteAllBackups(accountId, worldId);
      setBackups(items);
      setBackupTarget("");
      pushLog("All backups deleted.");
      pushToast("All backups deleted.");
    } catch (err) {
      pushLog(`Error deleting backups: ${err}`);
      pushToast(`Delete failed: ${err}`, "error");
    } finally {
      setLoading(false);
    }
  };

  const handleSaveName = async (id: string) => {
    const name = nameEdits[id] ?? "";
    try {
      const updated = await setPlayerName(accountId, worldId, id, name);
      setPlayers(updated);
      const nextEdits: Record<string, string> = {};
      updated.forEach((player) => {
        nextEdits[player.id] = player.name;
      });
      setNameEdits(nextEdits);
      setEditingPlayerId(null);
      pushLog(`Name saved for ${id}.`);
      pushToast("Name saved.");
    } catch (err) {
      pushLog(`Error saving name: ${err}`);
      pushToast(`Save failed: ${err}`, "error");
    }
  };

  const handleResetNames = async () => {
    const ok = await showConfirm(
      "Reset all player names to their original IDs?",
    );
    if (!ok) {
      return;
    }
    setLoading(true);
    try {
      const updated = await resetPlayerNames(accountId, worldId);
      setPlayers(updated);
      const nextEdits: Record<string, string> = {};
      updated.forEach((player) => {
        nextEdits[player.id] = player.name;
      });
      setNameEdits(nextEdits);
      pushLog("Player names reset.");
      pushToast("Player names reset.");
    } catch (err) {
      pushLog(`Error resetting names: ${err}`);
      pushToast(`Reset failed: ${err}`, "error");
    } finally {
      setLoading(false);
    }
  };

  /* ── World name handlers ───────────────────────────── */
  const currentWorld = worlds.find((w) => w.id === worldId);

  const handleStartWorldRename = () => {
    setWorldNameDraft(currentWorld?.displayName ?? "");
    setEditingWorldName(true);
  };

  const handleSaveWorldName = async () => {
    if (!accountId || !worldId) return;
    const trimmed = worldNameDraft.trim();
    setLoading(true);
    try {
      const updated = await setWorldName(accountId, worldId, trimmed);
      setWorlds(updated);
      pushToast(
        trimmed ? `World renamed to "${trimmed}".` : "World name cleared.",
      );
      setEditingWorldName(false);
    } catch (err) {
      pushToast(`Rename failed: ${err}`, "error");
    } finally {
      setLoading(false);
    }
  };

  const handleResetWorldName = async () => {
    if (!accountId || !worldId) return;
    setLoading(true);
    try {
      const updated = await resetWorldName(accountId, worldId);
      setWorlds(updated);
      pushToast("World name reset to folder name.");
      setEditingWorldName(false);
      setWorldNameDraft("");
    } catch (err) {
      pushToast(`Reset failed: ${err}`, "error");
    } finally {
      setLoading(false);
    }
  };

  /* ── Pointer-based swap handlers (no HTML5 drag, no native conflict) ── */
  const cardRefs = useRef<Map<string, HTMLElement>>(new Map());

  const registerCardRef = useCallback((id: string, el: HTMLElement | null) => {
    if (el) cardRefs.current.set(id, el);
    else cardRefs.current.delete(id);
  }, []);

  const getCardUnderPointer = (
    clientX: number,
    clientY: number,
  ): string | null => {
    for (const [id, el] of cardRefs.current.entries()) {
      const r = el.getBoundingClientRect();
      if (
        clientX >= r.left &&
        clientX <= r.right &&
        clientY >= r.top &&
        clientY <= r.bottom
      ) {
        return id;
      }
    }
    return null;
  };

  const onSwapPointerDown = (e: React.PointerEvent, playerId: string) => {
    if (!editMode) return;
    // Only primary button
    if (e.button !== 0) return;
    e.preventDefault();
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    dragSourceRef.current = playerId;
    setDragSourceId(playerId);
  };

  const onSwapPointerMove = (e: React.PointerEvent) => {
    if (!editMode || !dragSourceRef.current) return;
    const overId = getCardUnderPointer(e.clientX, e.clientY);
    setDragOverId(overId && overId !== dragSourceRef.current ? overId : null);
  };

  const onSwapPointerUp = (e: React.PointerEvent) => {
    if (!editMode || !dragSourceRef.current) return;
    const sourceId = dragSourceRef.current;
    const targetId = getCardUnderPointer(e.clientX, e.clientY);

    if (targetId && targetId !== sourceId) {
      const source = players.find((p) => p.id === sourceId);
      const target = players.find((p) => p.id === targetId);
      if (source && target) {
        setPlayers((prev) => {
          const next = prev.map((p) => ({ ...p }));
          const iA = next.findIndex((p) => p.id === sourceId);
          const iB = next.findIndex((p) => p.id === targetId);
          if (iA !== -1 && iB !== -1) {
            const tmpName = next[iA].name;
            const tmpOrig = next[iA].originalId;
            next[iA].name = next[iB].name;
            next[iA].originalId = next[iB].originalId;
            next[iB].name = tmpName;
            next[iB].originalId = tmpOrig;
          }
          return next;
        });
        setPendingSwaps((prev) => [
          ...prev,
          { fromId: sourceId, toId: targetId },
        ]);
        pushToast(`Swapped: ${source.name} ↔ ${target.name}`, "info");
      }
    }

    dragSourceRef.current = null;
    setDragSourceId(null);
    setDragOverId(null);
  };

  const handleApplySwaps = async () => {
    if (pendingSwaps.length === 0) return;
    const ok = await showConfirm(
      `Confirm ${pendingSwaps.length} swap${pendingSwaps.length > 1 ? "s" : ""}?`,
    );
    if (!ok) return;
    setLoading(true);
    try {
      await handleAutoBackup("drag swap");
      // Replay swaps against backend state (original order)
      // We use a shadow copy to track evolving host status
      let shadow = originalPlayersRef.current.map((p) => ({ ...p }));
      let updatedPlayers = shadow;
      for (const swap of pendingSwaps) {
        const isFromHost = shadow.find((p) => p.id === swap.fromId)?.isHost;
        const isToHost = shadow.find((p) => p.id === swap.toId)?.isHost;
        if (isFromHost || isToHost) {
          const nonHostId = isFromHost ? swap.toId : swap.fromId;
          updatedPlayers = await setHostPlayer(accountId, worldId, nonHostId);
        } else {
          updatedPlayers = await swapPlayers(
            accountId,
            worldId,
            swap.fromId,
            swap.toId,
          );
        }
        // Update shadow so next iteration sees correct host flags
        shadow = updatedPlayers.map((p) => ({ ...p }));
        pushLog(`Swapped: ${swap.fromId} ↔ ${swap.toId}`);
      }
      setPlayers(updatedPlayers);
      originalPlayersRef.current = [];
      setHostSlotState(updatedPlayers.find((p) => p.isHost)?.id ?? "");
      const nextEdits: Record<string, string> = {};
      updatedPlayers.forEach((p) => {
        nextEdits[p.id] = p.name;
      });
      setNameEdits(nextEdits);
      pushToast(`${pendingSwaps.length} swap(s) applied.`);
      setPendingSwaps([]);
      setEditMode(false);
    } catch (err) {
      pushLog(`Swap error: ${err}`);
      pushToast(`Swap failed: ${err}`, "error");
    } finally {
      setLoading(false);
    }
  };

  const handleCancelEditMode = () => {
    // Restore original order
    if (originalPlayersRef.current.length > 0) {
      setPlayers(originalPlayersRef.current);
    }
    setPendingSwaps([]);
    setEditMode(false);
    pushToast("Edit mode cancelled.", "info");
  };

  /* ── World Transfer ────────────────────────────────── */
  const [importDragOver, setImportDragOver] = useState(false);
  const [importFolder, setImportFolder] = useState<string | null>(null);
  const [importFolderName, setImportFolderName] = useState("");
  const [importConflict, setImportConflict] = useState(false);
  const [importMode, setImportMode] = useState<"replace" | "new">("replace");
  const [importNewName, setImportNewName] = useState("");
  const [exporting, setExporting] = useState(false);
  const [importing, setImporting] = useState(false);
  const [exportProgress, setExportProgress] = useState<number | null>(null);
  const [importProgress, setImportProgress] = useState<number | null>(null);

  /* Listen for Tauri native drag-drop events (file drops from OS) */
  useEffect(() => {
    let cancelled = false;
    const setup = async () => {
      const unlisten = await getCurrentWebview().onDragDropEvent((event) => {
        if (cancelled) return;
        if (event.payload.type === "over") {
          setImportDragOver(true);
        } else if (event.payload.type === "leave") {
          setImportDragOver(false);
        } else if (event.payload.type === "drop") {
          setImportDragOver(false);
          const paths = event.payload.paths;
          if (paths && paths.length > 0) {
            processImportFolder(paths[0]);
          }
        }
      });
      if (cancelled) unlisten();
      else cleanupRef.current = unlisten;
    };
    const cleanupRef = { current: () => {} };
    setup();
    return () => {
      cancelled = true;
      cleanupRef.current();
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  /* Listen for progress events from Rust */
  useEffect(() => {
    let cancelled = false;
    const unsubs: (() => void)[] = [];
    const setup = async () => {
      const u1 = await listen<{ percent: number; message: string }>(
        "export-progress",
        (event) => {
          if (!cancelled) setExportProgress(event.payload.percent);
        },
      );
      const u2 = await listen<{ percent: number; message: string }>(
        "import-progress",
        (event) => {
          if (!cancelled) setImportProgress(event.payload.percent);
        },
      );
      unsubs.push(u1, u2);
    };
    setup();
    return () => {
      cancelled = true;
      unsubs.forEach((u) => u());
    };
  }, []);

  const handleExportWorld = async () => {
    if (!accountId || !worldId) return;
    try {
      const dest = await save({
        defaultPath: `${worldId}.zip`,
        filters: [{ name: "ZIP Archive", extensions: ["zip"] }],
      });
      if (!dest) return;
      setExporting(true);
      setExportProgress(0);
      pushLog(`Exporting world ${worldId}…`);
      const result = await exportWorld(accountId, worldId, dest);
      pushLog(`World exported to: ${result}`);
      pushToast("World exported successfully.");
    } catch (err) {
      pushLog(`Export error: ${err}`);
      pushToast(`Export failed: ${err}`, "error");
    } finally {
      setExporting(false);
      setExportProgress(null);
    }
  };

  const handleBrowseImport = async () => {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (selected) {
        await processImportFolder(selected as string);
      }
    } catch (err) {
      pushToast(`Browse failed: ${err}`, "error");
    }
  };

  const processImportFolder = async (folderPath: string) => {
    try {
      const result = await validateWorldFolder(folderPath);
      setImportFolder(result.path);
      setImportFolderName(result.name);
      // Check conflict — use ref to avoid stale closure in native DnD callback
      const currentAccountId = accountIdRef.current;
      const exists = currentAccountId
        ? await checkWorldExists(currentAccountId, result.name)
        : false;
      setImportConflict(exists);
      setImportMode("replace");
      setImportNewName("");
    } catch (err) {
      pushToast(`${err}`, "error");
      setImportFolder(null);
      setImportFolderName("");
    }
  };

  const handleImportConfirm = async () => {
    const currentAccountId = accountIdRef.current;
    if (!importFolder || !currentAccountId) return;
    // When there's no conflict, always use "replace" mode (creates fresh copy)
    const mode = importConflict ? importMode : "replace";
    const newName = mode === "new" ? importNewName.trim() : undefined;
    if (mode === "new" && (!newName || newName.length === 0)) {
      pushToast("Enter a name for the new world.", "error");
      return;
    }
    setImporting(true);
    setImportProgress(0);
    try {
      pushLog(`Importing world from ${importFolder} (mode: ${mode})…`);
      const updatedWorlds = await importWorld(
        currentAccountId,
        importFolder,
        mode,
        newName,
      );
      setWorlds(updatedWorlds);
      // Select the imported world
      const targetName = mode === "new" ? newName! : importFolderName;
      setWorldId(targetName);

      // Force refresh players & backups for the imported world
      // (the useEffect on [accountId, worldId] won't fire if worldId is unchanged)
      try {
        const freshPlayers = await getPlayers(currentAccountId, targetName);
        setPlayers(freshPlayers);
        setHostSlotState(freshPlayers.find((p) => p.isHost)?.id ?? "");
        const nextEdits: Record<string, string> = {};
        freshPlayers.forEach((p) => {
          nextEdits[p.id] = p.name;
        });
        setNameEdits(nextEdits);

        const freshBackups = await listBackups(currentAccountId, targetName);
        setBackups(freshBackups);
        setBackupTarget(freshBackups[0] ?? "");
      } catch {
        // non-critical — UI will still show imported world
      }

      pushLog(`World imported: ${targetName}`);
      pushToast("World imported successfully.");
      // Reset import state
      setImportFolder(null);
      setImportFolderName("");
      setImportConflict(false);
      setImportNewName("");
    } catch (err) {
      pushLog(`Import error: ${err}`);
      pushToast(`Import failed: ${err}`, "error");
    } finally {
      setImporting(false);
      setImportProgress(null);
    }
  };

  const handleCancelImport = () => {
    setImportFolder(null);
    setImportFolderName("");
    setImportConflict(false);
    setImportNewName("");
  };

  /* ── P2P Share handlers ────────────────────────────── */

  const handleP2PSend = async () => {
    if (!accountId || !worldId) return;
    setP2pStatus("preparing");
    setP2pMessage("Preparing ZIP…");
    setP2pProgress(0);
    setP2pCode("");
    setP2pCopied(false);
    try {
      pushLog("P2P: Creating temp ZIP for sharing…");
      const zipPath = await exportWorldToTemp(accountId, worldId);
      setP2pTempZip(zipPath);
      pushLog("P2P: ZIP ready, waiting for peer…");

      await startSending(zipPath, {
        onStatus: (status, message) => {
          setP2pStatus(status);
          setP2pMessage(message ?? "");
          if (status === "waiting")
            pushLog("P2P: Waiting for peer to connect…");
          if (status === "transferring")
            pushLog("P2P: Peer connected, sending file…");
          if (status === "error") pushLog(`P2P: Error — ${message}`);
        },
        onProgress: (pct) => setP2pProgress(pct),
        onCode: (code) => {
          setP2pCode(code);
          pushLog(`P2P: Code generated: ${code}`);
        },
      });

      pushLog("P2P: World sent successfully!");
      pushToast("World shared via P2P!", "success");
    } catch (err) {
      pushLog(`P2P send error: ${err}`);
      if (p2pStatus !== "error") {
        pushToast(`P2P failed: ${err}`, "error");
      }
    } finally {
      // Cleanup temp ZIP
      if (p2pTempZip) {
        deleteTempFile(p2pTempZip).catch(() => {});
        setP2pTempZip(null);
      }
    }
  };

  const handleP2PReceive = async () => {
    const code = p2pReceiveCode.trim();
    if (!code || code.length < 4) {
      pushToast("Enter a valid code.", "error");
      return;
    }
    if (!accountId) {
      pushToast("Select an account first.", "error");
      return;
    }

    // Let user choose where to save the received ZIP
    const dest = await save({
      defaultPath: "world.zip",
      filters: [{ name: "ZIP Archive", extensions: ["zip"] }],
    });
    if (!dest) return; // user cancelled

    setP2pStatus("connecting");
    setP2pMessage("Connecting…");
    setP2pProgress(0);
    try {
      pushLog(`P2P: Connecting to sender with code ${code}…`);
      const folderPath = await startReceiving(code, dest, {
        onStatus: (status, message) => {
          setP2pStatus(status);
          setP2pMessage(message ?? "");
          if (status === "connecting") pushLog(`P2P: ${message}`);
          if (status === "transferring") pushLog("P2P: Receiving file…");
          if (status === "extracting")
            pushLog("P2P: Download complete, extracting ZIP…");
          if (status === "done") pushLog("P2P: Extraction complete!");
          if (status === "error") pushLog(`P2P: Error — ${message}`);
        },
        onProgress: (pct) => setP2pProgress(pct),
      });

      pushLog(`P2P: World received and saved to ${dest}`);
      // Use existing import flow
      await processImportFolder(folderPath);
      pushToast("World received! Review import options below.", "success");
    } catch (err) {
      pushLog(`P2P receive error: ${err}`);
      if (p2pStatus !== "error") {
        pushToast(`P2P receive failed: ${err}`, "error");
      }
    }
  };

  const handleP2PCancel = () => {
    cancelP2P();
    if (p2pTempZip) {
      deleteTempFile(p2pTempZip).catch(() => {});
      setP2pTempZip(null);
    }
    setP2pStatus("idle");
    setP2pMessage("");
    setP2pProgress(0);
    setP2pCode("");
    setP2pReceiveCode("");
  };

  const handleCopyCode = () => {
    if (p2pCode) {
      navigator.clipboard.writeText(p2pCode).then(() => {
        setP2pCopied(true);
        setTimeout(() => setP2pCopied(false), 2000);
      });
    }
  };

  const p2pBusy =
    p2pStatus !== "idle" && p2pStatus !== "done" && p2pStatus !== "error";

  const playerOptions = players.map((player) => (
    <option key={player.id} value={player.id}>
      {player.name} ({player.id})
    </option>
  ));

  const filteredPlayers = players.filter((player) => {
    const term = searchText.trim().toLowerCase();
    if (!term) {
      return true;
    }
    return (
      player.name.toLowerCase().includes(term) ||
      player.id.toLowerCase().includes(term)
    );
  });

  return (
    <div className="app">
      {/* ── Game running overlay ─────────────────────────── */}
      {gameRunning && (
        <div className="game-running-overlay">
          <div className="game-running-overlay__card">
            <div className="game-running-overlay__icon">
              <IconAlert size={48} />
            </div>
            <h2 className="game-running-overlay__title">Palworld is running</h2>
            <p className="game-running-overlay__text">
              Close the game before making any changes to save files.
              <br />
              Editing files while playing can cause{" "}
              <strong>data corruption</strong>.
            </p>
            <p className="game-running-overlay__hint">
              This overlay will disappear automatically when the game is closed.
            </p>
          </div>
        </div>
      )}

      {/* ── Header ──────────────────────────────────────── */}
      <header className="app__header">
        <div className="app__brand">
          <div className="app__icon">
            <IconGamepad size={16} />
          </div>
          <div>
            <h1>Palworld Host Switcher</h1>
            <p className="app__brand-sub">
              Swap ownership &amp; keep progress aligned
            </p>
          </div>
        </div>
        <div className="app__header-actions">
          {loading && <span className="spinner" />}
          <button
            className="btn-ghost btn-sm"
            onClick={handleRescan}
            disabled={loading}>
            <IconRefresh /> Rescan
          </button>
          <button
            className="btn-primary btn-sm"
            onClick={handleBackup}
            disabled={!players.length || loading}>
            <IconPlus /> Backup
          </button>
          <button
            className={`btn-ghost btn-sm${consoleOpen ? " active" : ""}`}
            onClick={() => setConsoleOpen((v) => !v)}
            title="Toggle console">
            <IconTerminal /> Console
          </button>
        </div>
      </header>

      <div className="app__body">
        {/* ── Sidebar ───────────────────────────────────── */}
        <aside
          className="sidebar"
          style={{ width: sidebarWidth, minWidth: sidebarWidth }}>
          <div className="sidebar__scroll">
            <Section title="Storage" defaultOpen={true}>
              <div className="field">
                <span className="field__label">Account</span>
                <select
                  value={accountId}
                  onChange={(event) => setAccountId(event.target.value)}>
                  {accounts.map((account) => (
                    <option key={account} value={account}>
                      {account}
                    </option>
                  ))}
                </select>
              </div>
              <div className="field">
                <span className="field__label">World</span>
                <select
                  value={worldId}
                  onChange={(event) => setWorldId(event.target.value)}>
                  {worlds.map((world) => (
                    <option key={world.id} value={world.id}>
                      {world.displayName
                        ? `${world.displayName} (${world.id})`
                        : world.id}{" "}
                      — {world.playerCount} players
                    </option>
                  ))}
                </select>
              </div>
              <div className="stats-row">
                <span className="stat">{players.length} players</span>
                <span className="stat truncate">
                  Host: {players.find((p) => p.isHost)?.name ?? "—"}
                </span>
              </div>
            </Section>

            <Section title="Host slot" defaultOpen={false}>
              <select
                value={hostSlot}
                onChange={(event) => setHostSlotState(event.target.value)}>
                <option value="">Select slot…</option>
                {playerOptions}
              </select>
              <button
                className="btn-secondary btn-full btn-sm"
                onClick={handleSetHostSlot}
                disabled={!hostSlot || !players.length || loading}>
                Apply
              </button>
            </Section>

            <Section title="Restore backup" defaultOpen={false}>
              <select
                value={backupTarget}
                onChange={(event) => setBackupTarget(event.target.value)}>
                <option value="">Select backup…</option>
                {backups.map((backup) => (
                  <option key={backup} value={backup}>
                    {backup}
                  </option>
                ))}
              </select>
              <div className="btn-row">
                <button
                  className="btn-secondary btn-sm"
                  style={{ flex: 1 }}
                  onClick={handleRestoreBackup}
                  disabled={!backupTarget || loading}>
                  Restore
                </button>
                <button
                  className="btn-ghost btn-sm btn-danger-hover"
                  onClick={() =>
                    backupTarget && handleDeleteBackup(backupTarget)
                  }
                  disabled={!backupTarget || loading}
                  title="Delete selected backup">
                  <IconTrash size={12} />
                </button>
              </div>
              {backups.length > 0 && (
                <button
                  className="btn-ghost btn-sm btn-danger-hover btn-full"
                  onClick={handleDeleteAllBackups}
                  disabled={loading}>
                  <IconTrash size={12} /> Delete all ({backups.length})
                </button>
              )}
            </Section>

            <Section title="Safety" defaultOpen={true}>
              <label className="toggle">
                <input
                  type="checkbox"
                  checked={autoBackup}
                  onChange={(event) => setAutoBackup(event.target.checked)}
                />
                <span>Auto-backup before swaps</span>
              </label>
            </Section>

            <Section title="World Transfer" defaultOpen={false}>
              {/* Export */}
              <div className="transfer-block">
                <span className="transfer-block__label">
                  <IconDownload size={12} /> Export World
                </span>
                <p className="transfer-block__desc">
                  Save the current world as a ZIP file to share with others.
                </p>
                <button
                  className="btn-secondary btn-full btn-sm"
                  onClick={handleExportWorld}
                  disabled={!accountId || !worldId || exporting || loading}>
                  {exporting ? "Exporting…" : "Export as ZIP"}
                </button>
                {exporting && exportProgress !== null && (
                  <div className="progress-bar">
                    <div
                      className="progress-bar__fill"
                      style={{ width: `${Math.round(exportProgress)}%` }}
                    />
                    <span className="progress-bar__label">
                      {Math.round(exportProgress)}%
                    </span>
                  </div>
                )}
              </div>

              <div className="transfer-divider" />

              {/* Import */}
              <div className="transfer-block">
                <span className="transfer-block__label">
                  <IconUpload size={12} /> Import World
                </span>
                <p className="transfer-block__desc">
                  Add someone else's world folder to your saves.
                </p>

                {!importFolder ? (
                  <>
                    <div
                      className={`import-dropzone${importDragOver ? " import-dropzone--active" : ""}`}>
                      <IconFolder size={20} />
                      <span>Drop world folder here</span>
                    </div>
                    <button
                      className="btn-ghost btn-full btn-sm"
                      onClick={handleBrowseImport}
                      disabled={!accountId || loading}>
                      <IconFolder size={12} /> Browse…
                    </button>
                  </>
                ) : (
                  <div className="import-preview">
                    <div className="import-preview__header">
                      <IconFolder size={14} />
                      <span className="import-preview__name truncate">
                        {importFolderName}
                      </span>
                      <button
                        className="btn-ghost btn-sm"
                        onClick={handleCancelImport}
                        title="Remove">
                        <IconX size={12} />
                      </button>
                    </div>

                    {importConflict && (
                      <div className="import-conflict">
                        <span className="import-conflict__warning">
                          <IconAlert size={11} /> World already exists
                        </span>
                        <label className="import-radio">
                          <input
                            type="radio"
                            name="importMode"
                            checked={importMode === "replace"}
                            onChange={() => setImportMode("replace")}
                          />
                          <span>Replace existing</span>
                        </label>
                        <label className="import-radio">
                          <input
                            type="radio"
                            name="importMode"
                            checked={importMode === "new"}
                            onChange={() => setImportMode("new")}
                          />
                          <span>Create as new</span>
                        </label>
                        {importMode === "new" && (
                          <input
                            className="import-new-name"
                            value={importNewName}
                            onChange={(e) => setImportNewName(e.target.value)}
                            placeholder="New world name…"
                          />
                        )}
                      </div>
                    )}

                    <button
                      className="btn-primary btn-full btn-sm"
                      onClick={handleImportConfirm}
                      disabled={
                        importing ||
                        loading ||
                        (importConflict &&
                          importMode === "new" &&
                          importNewName.trim().length === 0)
                      }>
                      {importing
                        ? "Importing…"
                        : importConflict && importMode === "replace"
                          ? "Replace & Import"
                          : "Import"}
                    </button>
                    {importing && importProgress !== null && (
                      <div className="progress-bar">
                        <div
                          className="progress-bar__fill"
                          style={{ width: `${Math.round(importProgress)}%` }}
                        />
                        <span className="progress-bar__label">
                          {Math.round(importProgress)}%
                        </span>
                      </div>
                    )}
                  </div>
                )}
              </div>
            </Section>

            <Section title="P2P Transfer" defaultOpen={false}>
              <div className="transfer-block">
                <p className="transfer-block__desc">
                  Share worlds directly with another player via peer-to-peer. No
                  server needed — data travels directly between PCs.
                </p>

                {p2pStatus === "idle" ||
                p2pStatus === "done" ||
                p2pStatus === "error" ? (
                  <>
                    {/* Send */}
                    <span className="transfer-block__label">
                      <IconShare size={12} /> Share World
                    </span>
                    <button
                      className="btn-secondary btn-full btn-sm"
                      onClick={handleP2PSend}
                      disabled={!accountId || !worldId || loading || p2pBusy}>
                      Share Current World
                    </button>

                    <div className="transfer-divider" />

                    {/* Receive */}
                    <span className="transfer-block__label">
                      <IconDownload size={12} /> Receive World
                    </span>
                    <div className="p2p-receive-row">
                      <input
                        className="p2p-code-input"
                        value={p2pReceiveCode}
                        onChange={(e) =>
                          setP2pReceiveCode(e.target.value.toUpperCase())
                        }
                        placeholder="Enter code…"
                        maxLength={6}
                        disabled={p2pBusy}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") handleP2PReceive();
                        }}
                      />
                      <button
                        className="btn-primary btn-sm"
                        onClick={handleP2PReceive}
                        disabled={
                          !accountId ||
                          p2pReceiveCode.trim().length < 4 ||
                          p2pBusy
                        }>
                        Connect
                      </button>
                    </div>

                    {/* Show last result */}
                    {p2pStatus === "done" && (
                      <div className="p2p-status p2p-status--done">
                        <IconCheck size={12} /> {p2pMessage}
                      </div>
                    )}
                    {p2pStatus === "error" && (
                      <div className="p2p-status p2p-status--error">
                        <IconAlert size={12} /> {p2pMessage}
                      </div>
                    )}
                  </>
                ) : (
                  /* Active transfer */
                  <div className="p2p-active">
                    <div className="p2p-status p2p-status--active">
                      {p2pStatus === "waiting" && p2pCode ? (
                        <>
                          <div className="p2p-code-display">
                            <span className="p2p-code-label">
                              Share this code:
                            </span>
                            <span className="p2p-code-value">{p2pCode}</span>
                            <button
                              className="btn-ghost btn-sm"
                              onClick={handleCopyCode}
                              title="Copy code">
                              {p2pCopied ? (
                                <IconCheck size={12} />
                              ) : (
                                <IconCopy size={12} />
                              )}
                            </button>
                          </div>
                          <span className="p2p-status-text">
                            Waiting for peer to connect…
                          </span>
                        </>
                      ) : (
                        <span className="p2p-status-text">{p2pMessage}</span>
                      )}
                    </div>

                    {(p2pStatus === "transferring" ||
                      p2pStatus === "extracting") && (
                      <div className="progress-bar">
                        <div
                          className="progress-bar__fill"
                          style={{
                            width: `${Math.round(p2pProgress)}%`,
                          }}
                        />
                        <span className="progress-bar__label">
                          {Math.round(p2pProgress)}%
                        </span>
                      </div>
                    )}

                    {p2pStatus === "preparing" && (
                      <div className="progress-bar">
                        <div className="progress-bar__fill progress-bar__fill--indeterminate" />
                      </div>
                    )}

                    <button
                      className="btn-ghost btn-full btn-sm"
                      onClick={handleP2PCancel}>
                      <IconX size={12} /> Cancel
                    </button>
                  </div>
                )}
              </div>
            </Section>
          </div>
        </aside>

        {/* ── Sidebar resize handle ─────────────────────── */}
        <div
          className="resize-handle resize-handle--v"
          onMouseDown={onSidebarMouseDown}
        />

        {/* ── Main content: Players ─────────────────────── */}
        <div className="content-wrapper">
          <main className="content">
            {/* ── World name header ─────────────────────── */}
            {worldId && (
              <div className="world-name-header">
                {editingWorldName ? (
                  <div className="world-name-header__edit">
                    <input
                      className="world-name-header__input"
                      value={worldNameDraft}
                      onChange={(e) => setWorldNameDraft(e.target.value)}
                      placeholder="Enter display name…"
                      autoFocus
                      onKeyDown={(e) => {
                        if (e.key === "Enter") handleSaveWorldName();
                        if (e.key === "Escape") setEditingWorldName(false);
                      }}
                    />
                    <button
                      className="btn-primary btn-sm"
                      onClick={handleSaveWorldName}
                      disabled={loading}>
                      <IconCheck size={12} /> Save
                    </button>
                    <button
                      className="btn-ghost btn-sm"
                      onClick={() => setEditingWorldName(false)}>
                      <IconX size={12} />
                    </button>
                  </div>
                ) : (
                  <div className="world-name-header__display">
                    <h2 className="world-name-header__title">
                      {currentWorld?.displayName ?? worldId}
                    </h2>
                    {currentWorld?.displayName && (
                      <span className="world-name-header__id">{worldId}</span>
                    )}
                    <button
                      className="btn-ghost btn-sm"
                      onClick={handleStartWorldRename}
                      title="Rename world"
                      disabled={loading}>
                      <IconPencil size={12} />
                    </button>
                    {currentWorld?.displayName && (
                      <button
                        className="btn-ghost btn-sm"
                        onClick={handleResetWorldName}
                        title="Reset to folder name"
                        disabled={loading}>
                        <IconRefresh size={12} />
                      </button>
                    )}
                  </div>
                )}
              </div>
            )}

            <div className="players-toolbar">
              <input
                className="search players-toolbar__search"
                placeholder="Search by name or ID…"
                value={searchText}
                onChange={(event) => setSearchText(event.target.value)}
              />
              <span className="players-toolbar__count">
                {filteredPlayers.length}/{players.length}
              </span>
              {editMode && pendingSwaps.length > 0 && (
                <span className="players-toolbar__pending">
                  <IconSwap size={12} /> {pendingSwaps.length} swap
                  {pendingSwaps.length > 1 ? "s" : ""}
                </span>
              )}
              {!editMode ? (
                <button
                  className="btn-ghost btn-sm"
                  onClick={() => {
                    originalPlayersRef.current = [...players];
                    setEditMode(true);
                    pushToast("Edit mode: drag players to swap them.", "info");
                  }}
                  disabled={players.length < 2 || loading}>
                  <IconDrag /> Edit
                </button>
              ) : (
                <>
                  <button
                    className="btn-primary btn-sm"
                    onClick={handleApplySwaps}
                    disabled={pendingSwaps.length === 0 || loading}>
                    <IconCheck size={12} /> Apply ({pendingSwaps.length})
                  </button>
                  <button
                    className="btn-ghost btn-sm"
                    onClick={handleCancelEditMode}>
                    <IconX size={12} /> Cancel
                  </button>
                </>
              )}
              <button
                className="btn-ghost btn-sm"
                onClick={handleResetNames}
                disabled={!players.length || loading || editMode}>
                Reset Names
              </button>
            </div>

            <div className="player-list-wrapper">
              <ul className="player-list">
                {filteredPlayers.map((player, idx) => {
                  // Check if this slot's content differs from original
                  const orig = originalPlayersRef.current[idx];
                  const hasMoved =
                    editMode &&
                    orig &&
                    (orig.name !== player.name ||
                      orig.originalId !== player.originalId);
                  return (
                    <li
                      key={player.id}
                      ref={(el) => registerCardRef(player.id, el)}
                      className={[
                        "player-card",
                        player.isHost && "is-host",
                        editMode && "edit-mode",
                        dragOverId === player.id && "drag-over",
                        hasMoved && "swapped",
                      ]
                        .filter(Boolean)
                        .join(" ")}
                      onPointerMove={onSwapPointerMove}
                      onPointerUp={onSwapPointerUp}>
                      {/* Fixed slot header: avatar + ID + badge */}
                      <div className="slot-header">
                        <div className="player-avatar">
                          {player.isHost ? <IconCrown /> : <IconUser />}
                        </div>
                        <div className="slot-id">
                          <span className="slot-id__label">Slot</span>
                          <span className="slot-id__value truncate">
                            {player.id}
                          </span>
                        </div>
                        <div className="player-meta">
                          {player.isHost ? (
                            <span className="badge badge--host">Host</span>
                          ) : editMode ? (
                            <span className="badge badge--player">
                              <IconSwap size={10} /> Drop
                            </span>
                          ) : (
                            <span className="badge badge--player">Player</span>
                          )}
                        </div>
                      </div>
                      {/* Pointer-based draggable player content */}
                      <div
                        className={[
                          "player-content",
                          editMode && "player-content--draggable",
                          dragSourceId === player.id &&
                            "player-content--dragging",
                        ]
                          .filter(Boolean)
                          .join(" ")}
                        onPointerDown={(e) => onSwapPointerDown(e, player.id)}
                        style={editMode ? { touchAction: "none" } : undefined}>
                        {editMode && (
                          <div className="player-drag-handle">
                            <IconDrag />
                          </div>
                        )}
                        <div className="player-info">
                          <p className="player-name">
                            <span className="truncate">{player.name}</span>
                            {player.name !== player.originalId && (
                              <span className="player-orig truncate">
                                {" "}
                                ({player.originalId})
                              </span>
                            )}
                            {!editMode && (
                              <button
                                className="btn-ghost btn-sm player-edit-toggle"
                                onClick={() =>
                                  setEditingPlayerId(
                                    editingPlayerId === player.id
                                      ? null
                                      : player.id,
                                  )
                                }
                                title="Rename">
                                <IconPencil />
                              </button>
                            )}
                          </p>
                          {!editMode && editingPlayerId === player.id && (
                            <div className="player-edit">
                              <input
                                value={nameEdits[player.id] ?? ""}
                                onChange={(event) =>
                                  handleNameChange(
                                    player.id,
                                    event.target.value,
                                  )
                                }
                                placeholder="Rename…"
                                autoFocus
                                onKeyDown={(event) => {
                                  if (event.key === "Enter") {
                                    handleSaveName(player.id);
                                  }
                                  if (event.key === "Escape") {
                                    setEditingPlayerId(null);
                                  }
                                }}
                              />
                              <button
                                className="btn-secondary btn-sm"
                                onClick={() => handleSaveName(player.id)}
                                disabled={
                                  (nameEdits[player.id] ?? "").trim() ===
                                  player.name
                                }>
                                Save
                              </button>
                            </div>
                          )}
                        </div>
                      </div>
                    </li>
                  );
                })}
                {filteredPlayers.length === 0 && players.length > 0 && (
                  <li className="player-card">
                    <p className="empty-msg">No players match your search.</p>
                  </li>
                )}
                {players.length === 0 && (
                  <li className="player-card">
                    <p className="empty-msg">
                      Select an account and world to load players.
                    </p>
                  </li>
                )}
              </ul>
            </div>
          </main>

          {/* ── Console panel ─────────────────────────────── */}
          {consoleOpen && (
            <div className="console" style={{ height: consoleHeight }}>
              <div
                className="resize-handle resize-handle--h"
                onMouseDown={onConsoleMouseDown}
              />
              <div className="console__header">
                <span className="console__title">Console</span>
                <span className="console__count">{logs.length} entries</span>
                <button
                  className="btn-ghost btn-sm"
                  onClick={() => setLogs([])}>
                  Clear
                </button>
                <button
                  className="btn-ghost btn-sm"
                  onClick={() => setConsoleOpen(false)}>
                  <IconX size={12} />
                </button>
              </div>
              <div className="console__body">
                {logs.length === 0 ? (
                  <p className="console__empty">No actions yet.</p>
                ) : (
                  logs.map((entry, index) => (
                    <p className="console__entry" key={index}>
                      <span className="console__dot" />
                      {entry}
                    </p>
                  ))
                )}
              </div>
            </div>
          )}
        </div>
      </div>

      {/* ── Status bar ──────────────────────────────────── */}
      <div className="status-bar">
        <span className="status-bar__item">
          {players.length} player{players.length !== 1 ? "s" : ""} loaded
        </span>
        {logs.length > 0 && (
          <span className="status-bar__item truncate">Last: {logs[0]}</span>
        )}
      </div>

      {/* ── Confirm Modal ─────────────────────────────────── */}
      {confirmModal && (
        <div
          className="modal-overlay"
          onClick={() => confirmCancelRef.current?.()}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <p className="modal__message">{confirmModal.message}</p>
            <div className="modal__actions">
              <button
                className="btn-secondary"
                onClick={() => confirmCancelRef.current?.()}
                type="button">
                Cancel
              </button>
              <button
                className="btn-primary"
                onClick={confirmModal.onConfirm}
                type="button">
                Confirm
              </button>
            </div>
          </div>
        </div>
      )}

      {/* ── Toasts ──────────────────────────────────────── */}
      <div className="toast-stack" aria-live="polite">
        {toasts.map((toast) => (
          <div key={toast.id} className={`toast toast--${toast.type}`}>
            <span className="toast__icon">
              {toast.type === "success" ? (
                <IconCheck />
              ) : toast.type === "error" ? (
                <IconAlert />
              ) : (
                <IconInfo />
              )}
            </span>
            {toast.text}
          </div>
        ))}
      </div>
    </div>
  );
}

export default App;
