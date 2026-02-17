/**
 * P2P World Transfer via WebRTC (PeerJS)
 *
 * Flow:
 *   Sender: export ZIP → read chunks → send via DataChannel
 *   Receiver: receive chunks → write to temp → extract → import
 *
 * Uses PeerJS's free public signaling server (0.peerjs.com).
 * File data travels directly between peers (no server for the actual data).
 */

import Peer from "peerjs";
import {
  getFileSize,
  readFileChunk,
  appendFileChunkB64,
  deleteTempFile,
  extractZipToTemp,
  validateWorldFolder,
} from "./palworldService";

const PEER_PREFIX = "palhost-";
const CHUNK_SIZE = 65536; // 64KB per chunk
const CODE_CHARS = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // no I,O,0,1

/* ── Helpers ─────────────────────────────────────────── */

function generateCode(): string {
  let code = "";
  for (let i = 0; i < 6; i++) {
    code += CODE_CHARS[Math.floor(Math.random() * CODE_CHARS.length)];
  }
  return code;
}

function uint8ToBase64(arr: Uint8Array): string {
  let binary = "";
  for (let i = 0; i < arr.length; i++) {
    binary += String.fromCharCode(arr[i]);
  }
  return btoa(binary);
}

function base64ToUint8(b64: string): Uint8Array {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

export type P2PStatus =
  | "idle"
  | "preparing"
  | "waiting"
  | "connecting"
  | "transferring"
  | "extracting"
  | "done"
  | "error";

export type P2PCallbacks = {
  onStatus: (status: P2PStatus, message?: string) => void;
  onProgress: (percent: number) => void;
  onCode?: (code: string) => void;
  /** Called on receiver when transfer is complete. Returns extracted folder path. */
  onComplete?: (folderPath: string) => void;
};

/* ── Active peer handle (for cleanup) ────────────────── */

let activePeer: Peer | null = null;
// eslint-disable-next-line @typescript-eslint/no-explicit-any
let activeConn: any = null;

export function cancelP2P(): void {
  if (activeConn) {
    try {
      activeConn.close();
    } catch {
      /* ignore */
    }
    activeConn = null;
  }
  if (activePeer) {
    try {
      activePeer.destroy();
    } catch {
      /* ignore */
    }
    activePeer = null;
  }
}

/* ── SENDER ──────────────────────────────────────────── */

export async function startSending(
  zipPath: string,
  callbacks: P2PCallbacks,
): Promise<void> {
  cancelP2P(); // cleanup any previous session

  const code = generateCode();
  const peerId = PEER_PREFIX + code;

  callbacks.onStatus("waiting", `Code: ${code}`);
  callbacks.onCode?.(code);

  return new Promise<void>((resolve, reject) => {
    const peer = new Peer(peerId, {
      debug: 0,
      config: {
        iceServers: [
          { urls: "stun:stun.l.google.com:19302" },
          { urls: "stun:stun1.l.google.com:19302" },
        ],
      },
    });
    activePeer = peer;

    const timeout = setTimeout(
      () => {
        callbacks.onStatus(
          "error",
          "Timeout — no one connected within 5 minutes.",
        );
        cancelP2P();
        reject(new Error("Timeout"));
      },
      5 * 60 * 1000,
    );

    peer.on("open", () => {
      callbacks.onStatus("waiting", `Code: ${code} — waiting for connection…`);
    });

    peer.on("error", (err) => {
      clearTimeout(timeout);
      callbacks.onStatus("error", `Connection error: ${err.message || err}`);
      cancelP2P();
      reject(err);
    });

    peer.on("connection", (conn) => {
      clearTimeout(timeout);
      activeConn = conn;
      callbacks.onStatus("connecting", "Peer connected, starting transfer…");

      conn.on("open", async () => {
        try {
          await sendFileViaConnection(conn, zipPath, callbacks);
          callbacks.onStatus("done", "Transfer complete!");
          callbacks.onProgress(100);
          resolve();
        } catch (e) {
          callbacks.onStatus(
            "error",
            `Transfer failed: ${e instanceof Error ? e.message : e}`,
          );
          reject(e);
        } finally {
          // small delay before cleanup so the last messages are flushed
          setTimeout(() => cancelP2P(), 2000);
        }
      });

      conn.on("error", (err) => {
        callbacks.onStatus("error", `Connection error: ${err}`);
        cancelP2P();
        reject(err);
      });
    });
  });
}

async function sendFileViaConnection(
  conn: ReturnType<Peer["connect"]>,
  zipPath: string,
  callbacks: P2PCallbacks,
): Promise<void> {
  const totalSize = await getFileSize(zipPath);
  const totalChunks = Math.ceil(totalSize / CHUNK_SIZE);

  callbacks.onStatus("transferring", "Sending file…");
  callbacks.onProgress(0);

  // Send metadata
  conn.send(
    JSON.stringify({
      type: "meta",
      filename: zipPath.split(/[/\\]/).pop() || "world.zip",
      totalSize,
      totalChunks,
    }),
  );

  // Small delay to let metadata propagate
  await new Promise((r) => setTimeout(r, 100));

  for (let i = 0; i < totalChunks; i++) {
    const offset = i * CHUNK_SIZE;
    const chunkData = await readFileChunk(zipPath, offset, CHUNK_SIZE);
    const bytes = new Uint8Array(chunkData);
    const b64 = uint8ToBase64(bytes);

    conn.send(JSON.stringify({ type: "chunk", index: i, data: b64 }));

    // Back-pressure: wait if DataChannel buffer is too full
    const dc = (conn as unknown as { dataChannel?: RTCDataChannel })
      .dataChannel;
    if (dc && dc.bufferedAmount > 2 * 1024 * 1024) {
      await new Promise<void>((resolve) => {
        const check = () => {
          if (!dc || dc.bufferedAmount < 512 * 1024) resolve();
          else setTimeout(check, 50);
        };
        check();
      });
    }

    const pct = ((i + 1) / totalChunks) * 100;
    callbacks.onProgress(pct);
  }

  // Send done signal
  conn.send(JSON.stringify({ type: "done" }));
}

/* ── RECEIVER ────────────────────────────────────────── */

export async function startReceiving(
  code: string,
  destZipPath: string,
  callbacks: P2PCallbacks,
): Promise<string> {
  cancelP2P();

  const receiverId = PEER_PREFIX + "r-" + Date.now().toString(36);
  const targetId = PEER_PREFIX + code.toUpperCase().trim();

  callbacks.onStatus("connecting", "Connecting to sender…");

  return new Promise<string>((resolve, reject) => {
    let transferStarted = false;

    const peer = new Peer(receiverId, {
      debug: 0,
      config: {
        iceServers: [
          { urls: "stun:stun.l.google.com:19302" },
          { urls: "stun:stun1.l.google.com:19302" },
        ],
      },
    });
    activePeer = peer;

    peer.on("open", () => {
      const conn = peer.connect(targetId, { reliable: true });
      activeConn = conn;

      let totalSize = 0;
      let receivedBytes = 0;
      let tempPath = "";

      conn.on("open", () => {
        callbacks.onStatus("connecting", "Connected! Waiting for file…");
      });

      conn.on("data", async (raw) => {
        try {
          const msg = JSON.parse(raw as string);

          if (msg.type === "meta") {
            totalSize = msg.totalSize;
            tempPath = destZipPath;
            // Clear any previous file at destination
            await deleteTempFile(tempPath).catch(() => {});
            callbacks.onStatus("transferring", "Receiving file…");
            callbacks.onProgress(0);
            transferStarted = true;
          } else if (msg.type === "chunk") {
            const b64: string = msg.data;
            await appendFileChunkB64(tempPath, b64);
            const chunkBytes = base64ToUint8(b64).length;
            receivedBytes += chunkBytes;
            const pct = Math.min((receivedBytes / totalSize) * 100, 99);
            callbacks.onProgress(pct);
          } else if (msg.type === "done") {
            callbacks.onProgress(100);
            callbacks.onStatus("extracting", "Extracting ZIP…");

            // Extract the received ZIP
            const extractedFolder = await extractZipToTemp(tempPath);
            // Validate it
            const validated = await validateWorldFolder(extractedFolder);

            // ZIP stays at the user-chosen destination (not deleted)

            callbacks.onStatus("done", "World received!");
            callbacks.onComplete?.(validated.path);

            setTimeout(() => cancelP2P(), 1000);
            resolve(validated.path);
          }
        } catch (e) {
          callbacks.onStatus(
            "error",
            `Receive error: ${e instanceof Error ? e.message : e}`,
          );
          cancelP2P();
          reject(e);
        }
      });

      conn.on("error", (err) => {
        callbacks.onStatus("error", `Connection error: ${err}`);
        cancelP2P();
        reject(err);
      });

      conn.on("close", () => {
        if (receivedBytes < totalSize && totalSize > 0) {
          callbacks.onStatus(
            "error",
            "Connection closed before transfer completed.",
          );
          reject(new Error("Connection closed prematurely"));
        }
      });
    });

    peer.on("error", (err) => {
      const msg = (err as Error).message || String(err);
      if (msg.includes("Could not connect to peer")) {
        callbacks.onStatus("error", "Invalid code or sender is not online.");
      } else {
        callbacks.onStatus("error", `Error: ${msg}`);
      }
      cancelP2P();
      reject(err);
    });

    // Timeout
    setTimeout(() => {
      if (!transferStarted) {
        callbacks.onStatus("error", "Connection timeout.");
        cancelP2P();
        reject(new Error("Timeout"));
      }
    }, 30000);
  });
}
