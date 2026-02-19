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
const CHUNK_SIZE = 262144; // 256KB per chunk (4× faster, fewer IPC round-trips)
const CODE_CHARS = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // no I,O,0,1

/* ── Metered TURN API ──────────────────────────────── */

const STUN_SERVERS: RTCIceServer[] = [
  { urls: "stun:stun.l.google.com:19302" },
  { urls: "stun:stun1.l.google.com:19302" },
  { urls: "stun:stun.relay.metered.ca:80" },
];

/** Cached Metered TURN credentials (refreshed every 10 min or on demand) */
let cachedMeteredServers: RTCIceServer[] | null = null;
let cachedMeteredAt = 0;
const CACHE_TTL = 10 * 60 * 1000; // 10 minutes

function getLocalStorageValue(key: string): string | undefined {
  try {
    if (typeof localStorage === "undefined") return undefined;
    const value = localStorage.getItem(key);
    if (!value || value.trim().length === 0) return undefined;
    return value;
  } catch {
    return undefined;
  }
}

function buildIceServers(): RTCIceServer[] {
  const servers: RTCIceServer[] = [...STUN_SERVERS];

  // Add cached Metered TURN servers if available
  if (cachedMeteredServers) {
    servers.push(...cachedMeteredServers);
  }

  const rawList =
    getLocalStorageValue("p2p.iceServers") ??
    (import.meta.env.VITE_P2P_ICE_SERVERS as string | undefined);
  if (rawList) {
    try {
      const parsed = JSON.parse(rawList) as unknown;
      if (Array.isArray(parsed)) {
        for (const entry of parsed) {
          if (entry && typeof entry === "object" && "urls" in entry) {
            servers.push(entry as RTCIceServer);
          }
        }
      }
    } catch {
      // ignore malformed JSON to keep defaults working
    }
  }

  const turnUrlsRaw =
    getLocalStorageValue("p2p.turnUrls") ??
    (import.meta.env.VITE_P2P_TURN_URL as string | undefined) ??
    "";
  const turnUrls = turnUrlsRaw
    .split(",")
    .map((url) => url.trim())
    .filter(Boolean);
  if (turnUrls.length > 0) {
    const secret =
      getLocalStorageValue("p2p.turnSecret") ??
      (import.meta.env.VITE_P2P_TURN_SECRET as string | undefined);

    if (secret) {
      // HMAC-SHA1 ephemeral credentials (coturn use-auth-secret)
      // Username = expiry timestamp, credential = HMAC-SHA1(username, secret)
      // Credentials are generated synchronously via a pre-computed pair
      // and refreshed each time buildIceServers() is called.
      const creds = generateTurnCredentials(secret);
      servers.push({
        urls: turnUrls,
        username: creds.username,
        credential: creds.credential,
      });
    } else {
      // No secret: push URLs without auth (some servers allow anonymous)
      servers.push({ urls: turnUrls });
    }
  }

  return servers;
}

/**
 * Check whether the user has configured their Metered credentials.
 * P2P transfers are blocked until this returns true.
 */
export function isMeteredConfigured(): boolean {
  return !!(
    getLocalStorageValue("p2p.meteredDomain") &&
    getLocalStorageValue("p2p.meteredApiKey")
  );
}

/**
 * Fetch TURN credentials from Metered API and cache them.
 * Reads domain + API key from localStorage (user must configure first).
 * Called automatically before P2P sessions and by the Test TURN button.
 */
export async function fetchMeteredCredentials(): Promise<RTCIceServer[]> {
  // Return cached if fresh
  if (cachedMeteredServers && Date.now() - cachedMeteredAt < CACHE_TTL) {
    return cachedMeteredServers;
  }

  const domain = getLocalStorageValue("p2p.meteredDomain");
  const apiKey = getLocalStorageValue("p2p.meteredApiKey");
  if (!domain || !apiKey) {
    throw new Error(
      "Metered TURN not configured. Please enter your Metered domain and API key in P2P settings.",
    );
  }

  const url = `https://${domain}/api/v1/turn/credentials?apiKey=${apiKey}`;
  const resp = await fetch(url);
  if (!resp.ok) {
    throw new Error(`Metered API error: ${resp.status} ${resp.statusText}`);
  }
  const data = (await resp.json()) as RTCIceServer[];
  if (!Array.isArray(data) || data.length === 0) {
    throw new Error("Metered API returned empty credentials");
  }
  cachedMeteredServers = data;
  cachedMeteredAt = Date.now();
  return data;
}

/** Ensure TURN credentials are loaded before starting a P2P session */
async function ensureTurnReady(): Promise<void> {
  if (!isMeteredConfigured()) {
    throw new Error(
      "TURN relay not configured. Open P2P settings and enter your Metered.ca credentials.",
    );
  }
  await fetchMeteredCredentials();
}

/* ── HMAC-SHA1 ephemeral credentials (coturn use-auth-secret) ── */

function generateTurnCredentials(secret: string): {
  username: string;
  credential: string;
} {
  // Username = Unix timestamp 24h in the future (coturn convention)
  const expiry = Math.floor(Date.now() / 1000) + 86400;
  const username = expiry.toString();

  // HMAC-SHA1 computed synchronously using a simple implementation
  const credential = hmacSha1Base64(secret, username);
  return { username, credential };
}

function hmacSha1Base64(key: string, message: string): string {
  const BLOCK_SIZE = 64;
  const keyBytes = strToBytes(key);
  const msgBytes = strToBytes(message);

  let k = keyBytes;
  if (k.length > BLOCK_SIZE) k = sha1(k);
  if (k.length < BLOCK_SIZE) {
    const padded = new Uint8Array(BLOCK_SIZE);
    padded.set(k);
    k = padded;
  }

  const iPad = new Uint8Array(BLOCK_SIZE);
  const oPad = new Uint8Array(BLOCK_SIZE);
  for (let i = 0; i < BLOCK_SIZE; i++) {
    iPad[i] = k[i] ^ 0x36;
    oPad[i] = k[i] ^ 0x5c;
  }

  const inner = sha1(concat(iPad, msgBytes));
  const outer = sha1(concat(oPad, inner));
  return uint8ToBase64(outer);
}

function strToBytes(s: string): Uint8Array {
  const enc = new TextEncoder();
  return enc.encode(s);
}

function concat(a: Uint8Array, b: Uint8Array): Uint8Array {
  const c = new Uint8Array(a.length + b.length);
  c.set(a);
  c.set(b, a.length);
  return c;
}

function sha1(data: Uint8Array): Uint8Array {
  let h0 = 0x67452301;
  let h1 = 0xefcdab89;
  let h2 = 0x98badcfe;
  let h3 = 0x10325476;
  let h4 = 0xc3d2e1f0;

  const bitLen = data.length * 8;
  // Padding
  const padLen = 64 - ((data.length + 9) % 64);
  const total = data.length + 1 + (padLen === 64 ? 0 : padLen) + 8;
  const buf = new Uint8Array(total);
  buf.set(data);
  buf[data.length] = 0x80;
  // Length in bits as big-endian 64-bit
  const dv = new DataView(buf.buffer);
  dv.setUint32(total - 4, bitLen, false);

  const rot = (n: number, s: number) => ((n << s) | (n >>> (32 - s))) >>> 0;

  for (let off = 0; off < total; off += 64) {
    const w = new Uint32Array(80);
    for (let i = 0; i < 16; i++) {
      w[i] = dv.getUint32(off + i * 4, false);
    }
    for (let i = 16; i < 80; i++) {
      w[i] = rot(w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16], 1);
    }

    let a = h0,
      b = h1,
      c = h2,
      d = h3,
      e = h4;
    for (let i = 0; i < 80; i++) {
      let f: number, k: number;
      if (i < 20) {
        f = ((b & c) | (~b & d)) >>> 0;
        k = 0x5a827999;
      } else if (i < 40) {
        f = (b ^ c ^ d) >>> 0;
        k = 0x6ed9eba1;
      } else if (i < 60) {
        f = ((b & c) | (b & d) | (c & d)) >>> 0;
        k = 0x8f1bbcdc;
      } else {
        f = (b ^ c ^ d) >>> 0;
        k = 0xca62c1d6;
      }
      const temp = (rot(a, 5) + f + e + k + w[i]) >>> 0;
      e = d;
      d = c;
      c = rot(b, 30);
      b = a;
      a = temp;
    }
    h0 = (h0 + a) >>> 0;
    h1 = (h1 + b) >>> 0;
    h2 = (h2 + c) >>> 0;
    h3 = (h3 + d) >>> 0;
    h4 = (h4 + e) >>> 0;
  }

  const result = new Uint8Array(20);
  const rv = new DataView(result.buffer);
  rv.setUint32(0, h0, false);
  rv.setUint32(4, h1, false);
  rv.setUint32(8, h2, false);
  rv.setUint32(12, h3, false);
  rv.setUint32(16, h4, false);
  return result;
}

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
  onDebug?: (message: string) => void;
  /** Called on receiver when transfer is complete. Returns extracted folder path. */
  onComplete?: (folderPath: string) => void;
};

/* ── Active peer handle (for cleanup) ────────────────── */

let activePeer: Peer | null = null;
// eslint-disable-next-line @typescript-eslint/no-explicit-any
let activeConn: any = null;

function attachIceDebug(
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  conn: any,
  callbacks: P2PCallbacks,
  label: string,
): void {
  const pc = conn?.peerConnection as RTCPeerConnection | undefined;
  if (!pc || !callbacks.onDebug) return;

  const emit = (msg: string) => callbacks.onDebug?.(`${label}: ${msg}`);

  pc.addEventListener("iceconnectionstatechange", () => {
    emit(`iceConnectionState=${pc.iceConnectionState}`);
  });
  pc.addEventListener("connectionstatechange", () => {
    emit(`connectionState=${pc.connectionState}`);
  });
  pc.addEventListener("icegatheringstatechange", () => {
    emit(`iceGatheringState=${pc.iceGatheringState}`);
  });
  pc.addEventListener("signalingstatechange", () => {
    emit(`signalingState=${pc.signalingState}`);
  });
  pc.addEventListener("icecandidateerror", (event) => {
    emit(`iceCandidateError code=${event.errorCode} text=${event.errorText}`);
  });
}

async function waitForIceGatheringComplete(
  pc: RTCPeerConnection,
  timeoutMs: number,
): Promise<void> {
  if (pc.iceGatheringState === "complete") return;

  await new Promise<void>((resolve) => {
    const timeout = setTimeout(() => {
      // Resolve even on timeout — we'll check whatever candidates we got
      cleanup();
      resolve();
    }, timeoutMs);

    const onState = () => {
      if (pc.iceGatheringState === "complete") {
        cleanup();
        resolve();
      }
    };

    const cleanup = () => {
      clearTimeout(timeout);
      pc.removeEventListener("icegatheringstatechange", onState);
    };

    pc.addEventListener("icegatheringstatechange", onState);
  });
}

export async function testIceServers(): Promise<{
  relayFound: boolean;
  summary: string;
}> {
  // Ensure Metered TURN credentials are fetched first
  await fetchMeteredCredentials();

  const pc = new RTCPeerConnection({ iceServers: buildIceServers() });
  try {
    pc.createDataChannel("turn-test");
    const offer = await pc.createOffer({ offerToReceiveAudio: false });
    await pc.setLocalDescription(offer);

    await waitForIceGatheringComplete(pc, 12000);

    const stats = await pc.getStats();
    const counts = { host: 0, srflx: 0, relay: 0, prflx: 0 } as Record<
      string,
      number
    >;

    stats.forEach((report) => {
      if (report.type === "local-candidate") {
        const candidateType = (report as { candidateType?: string })
          .candidateType;
        if (candidateType && candidateType in counts) {
          counts[candidateType] += 1;
        }
      }
    });

    const relayFound = counts.relay > 0;
    const summary = `host=${counts.host}, srflx=${counts.srflx}, relay=${counts.relay}, prflx=${counts.prflx}`;
    return { relayFound, summary };
  } finally {
    pc.close();
  }
}

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
  await ensureTurnReady();

  const code = generateCode();
  const peerId = PEER_PREFIX + code;

  callbacks.onStatus("waiting", `Code: ${code}`);
  callbacks.onCode?.(code);

  return new Promise<void>((resolve, reject) => {
    const peer = new Peer(peerId, {
      debug: 0,
      config: {
        iceServers: buildIceServers(),
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
      attachIceDebug(conn, callbacks, "Sender");

      conn.on("open", async () => {
        try {
          await sendFileViaConnection(conn, zipPath, callbacks);

          // Wait for receiver to acknowledge it processed everything
          callbacks.onStatus(
            "transferring",
            "Waiting for receiver to confirm…",
          );
          await waitForReceiverAck(conn, 120_000);

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
          // Longer delay before cleanup so the receiver finishes any remaining work
          setTimeout(() => cancelP2P(), 5000);
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

  // Send metadata as JSON string
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
    // Send raw binary — no base64 encoding needed (saves ~33% bandwidth)
    conn.send(new Uint8Array(chunkData));

    // Back-pressure: wait if DataChannel buffer is too full (4MB threshold)
    const dc = (conn as unknown as { dataChannel?: RTCDataChannel })
      .dataChannel;
    if (dc && dc.bufferedAmount > 4 * 1024 * 1024) {
      await drainBuffer(dc, 1024 * 1024);
    }

    const pct = ((i + 1) / totalChunks) * 100;
    callbacks.onProgress(pct);
  }

  // Wait for DataChannel send-buffer to drain before signaling done
  const dcFinal = (conn as unknown as { dataChannel?: RTCDataChannel })
    .dataChannel;
  if (dcFinal && dcFinal.bufferedAmount > 0) {
    await drainBuffer(dcFinal, 0);
  }

  // Send done signal
  conn.send(JSON.stringify({ type: "done" }));
}

/**
 * Wait until DataChannel bufferedAmount drops to the target threshold.
 * Uses the efficient bufferedAmountLowThreshold event with a fallback poll.
 */
function drainBuffer(dc: RTCDataChannel, target: number): Promise<void> {
  return new Promise<void>((resolve) => {
    if (dc.bufferedAmount <= target) {
      resolve();
      return;
    }
    const prev = dc.bufferedAmountLowThreshold;
    dc.bufferedAmountLowThreshold = target;

    let fallback: ReturnType<typeof setInterval> | null = null;
    const cleanup = () => {
      dc.removeEventListener("bufferedamountlow", onLow);
      dc.bufferedAmountLowThreshold = prev;
      if (fallback) clearInterval(fallback);
    };
    const onLow = () => {
      cleanup();
      resolve();
    };
    dc.addEventListener("bufferedamountlow", onLow);
    // Fallback polling in case the event doesn't fire (some browsers)
    fallback = setInterval(() => {
      if (dc.bufferedAmount <= target) {
        cleanup();
        resolve();
      }
    }, 100);
  });
}

/**
 * Wait for the receiver to send back an { type: "ack" } message.
 * This ensures the receiver has actually written all chunks to disk
 * and processed the "done" signal before the sender tears down the connection.
 */
function waitForReceiverAck(
  conn: ReturnType<Peer["connect"]>,
  timeoutMs: number,
): Promise<void> {
  return new Promise<void>((resolve) => {
    const timeout = setTimeout(() => {
      cleanup();
      // Resolve anyway — data was sent, ack may have been lost
      resolve();
    }, timeoutMs);

    const onData = (raw: unknown) => {
      try {
        const msg = JSON.parse(raw as string);
        if (msg.type === "ack") {
          cleanup();
          resolve();
        }
      } catch {
        // ignore non-JSON messages
      }
    };

    const onClose = () => {
      cleanup();
      // Connection closed before ack — resolve anyway since data was sent
      resolve();
    };

    const cleanup = () => {
      clearTimeout(timeout);
      conn.off("data", onData);
      conn.off("close", onClose);
    };

    conn.on("data", onData);
    conn.on("close", onClose);
  });
}

/* ── RECEIVER ────────────────────────────────────────── */

export async function startReceiving(
  code: string,
  destZipPath: string,
  callbacks: P2PCallbacks,
): Promise<string> {
  cancelP2P();
  await ensureTurnReady();

  const receiverId = PEER_PREFIX + "r-" + Date.now().toString(36);
  const targetId = PEER_PREFIX + code.toUpperCase().trim();

  callbacks.onStatus("connecting", "Connecting to sender…");

  return new Promise<string>((resolve, reject) => {
    let transferStarted = false;
    let settled = false;

    const settle = (type: "resolve" | "reject", value: string | Error) => {
      if (settled) return;
      settled = true;
      if (type === "resolve") resolve(value as string);
      else reject(value);
    };

    const peer = new Peer(receiverId, {
      debug: 0,
      config: {
        iceServers: buildIceServers(),
      },
    });
    activePeer = peer;

    peer.on("open", () => {
      const conn = peer.connect(targetId, { reliable: true });
      activeConn = conn;
      attachIceDebug(conn, callbacks, "Receiver");

      let totalSize = 0;
      let receivedBytes = 0;
      let tempPath = "";
      // Sequential write queue — prevents race conditions from concurrent async handlers
      let writeChain = Promise.resolve();

      conn.on("open", () => {
        callbacks.onStatus("connecting", "Connected! Waiting for file…");
      });

      // NOT async — we queue work instead of awaiting inside the handler
      conn.on("data", (raw) => {
        // JSON string → control message; binary → chunk data
        if (typeof raw === "string") {
          try {
            const msg = JSON.parse(raw);

            if (msg.type === "meta") {
              totalSize = msg.totalSize;
              tempPath = destZipPath;
              // Clear any previous file at destination (queued)
              writeChain = writeChain.then(() =>
                deleteTempFile(tempPath).catch(() => {}),
              );
              callbacks.onStatus("transferring", "Receiving file…");
              callbacks.onProgress(0);
              transferStarted = true;
            } else if (msg.type === "done") {
              // Wait for ALL queued chunk writes to finish, then extract
              writeChain
                .then(async () => {
                  callbacks.onProgress(100);
                  callbacks.onStatus("extracting", "Extracting ZIP…");

                  // Send ack BEFORE extraction so sender knows data arrived
                  try {
                    conn.send(JSON.stringify({ type: "ack" }));
                  } catch {
                    // Connection may already be closing — not critical
                  }

                  const extractedFolder = await extractZipToTemp(tempPath);
                  const validated = await validateWorldFolder(extractedFolder);

                  callbacks.onStatus("done", "World received!");
                  callbacks.onComplete?.(validated.path);

                  setTimeout(() => cancelP2P(), 2000);
                  settle("resolve", validated.path);
                })
                .catch((e) => {
                  callbacks.onStatus(
                    "error",
                    `Receive error: ${e instanceof Error ? e.message : e}`,
                  );
                  cancelP2P();
                  settle(
                    "reject",
                    e instanceof Error ? e : new Error(String(e)),
                  );
                });
            }
          } catch (e) {
            callbacks.onStatus(
              "error",
              `Receive error: ${e instanceof Error ? e.message : e}`,
            );
            cancelP2P();
            settle("reject", e instanceof Error ? e : new Error(String(e)));
          }
        } else {
          // Binary chunk data — extract Uint8Array robustly
          let bytes: Uint8Array;
          if (raw instanceof ArrayBuffer) {
            bytes = new Uint8Array(raw);
          } else if (raw instanceof Uint8Array) {
            bytes = raw;
          } else if (ArrayBuffer.isView(raw)) {
            bytes = new Uint8Array(
              (raw as ArrayBufferView).buffer,
              (raw as ArrayBufferView).byteOffset,
              (raw as ArrayBufferView).byteLength,
            );
          } else {
            // Unexpected type — skip
            return;
          }

          const chunkLen = bytes.byteLength;

          // Queue the write so chunks are processed sequentially
          writeChain = writeChain.then(async () => {
            // Convert to base64 for Tauri IPC (local-only, fast)
            const b64 = uint8ToBase64(bytes);
            await appendFileChunkB64(tempPath, b64);
            receivedBytes += chunkLen;
            const pct = Math.min((receivedBytes / totalSize) * 100, 99);
            callbacks.onProgress(pct);
          });
        }
      });

      conn.on("error", (err) => {
        callbacks.onStatus("error", `Connection error: ${err}`);
        cancelP2P();
        settle("reject", err instanceof Error ? err : new Error(String(err)));
      });

      conn.on("close", () => {
        if (receivedBytes < totalSize && totalSize > 0) {
          callbacks.onStatus(
            "error",
            "Connection closed before transfer completed.",
          );
          settle("reject", new Error("Connection closed prematurely"));
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
      settle("reject", err instanceof Error ? err : new Error(msg));
    });

    // Timeout
    setTimeout(() => {
      if (!transferStarted && !settled) {
        callbacks.onStatus("error", "Connection timeout.");
        cancelP2P();
        settle("reject", new Error("Timeout"));
      }
    }, 120000);
  });
}
