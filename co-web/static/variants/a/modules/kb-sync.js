/**
 * CO-367: KB sync retry queue.
 *
 * Surface-side module that queues KbIngestEvents in IndexedDB when the
 * CO KB endpoint is unavailable, and drains the queue on reconnect.
 *
 * Resilience guarantee: local entry writes are never blocked by sync state.
 * Retry schedule: 1m → 5m → 30m → 2h (exponential backoff, 4 levels).
 */

const DB_NAME = 'co-kb-sync';
const STORE   = 'pending';
const DB_VERSION = 1;

const RETRY_DELAYS_MS = [
  60_000,       // 1 min
  300_000,      // 5 min
  1_800_000,    // 30 min
  7_200_000,    // 2 h
];

// ---------------------------------------------------------------------------
// IndexedDB helpers
// ---------------------------------------------------------------------------

function openDb() {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onupgradeneeded = (e) => {
      const db = e.target.result;
      if (!db.objectStoreNames.contains(STORE)) {
        const store = db.createObjectStore(STORE, { keyPath: 'id', autoIncrement: true });
        store.createIndex('next_attempt', 'next_attempt', { unique: false });
      }
    };
    req.onsuccess  = (e) => resolve(e.target.result);
    req.onerror    = (e) => reject(e.target.error);
  });
}

async function enqueue(db, event, attempt = 0) {
  const delay = RETRY_DELAYS_MS[Math.min(attempt, RETRY_DELAYS_MS.length - 1)];
  const record = {
    event,
    attempt,
    next_attempt: Date.now() + delay,
    created_at: Date.now(),
  };
  return new Promise((resolve, reject) => {
    const tx  = db.transaction(STORE, 'readwrite');
    const req = tx.objectStore(STORE).add(record);
    req.onsuccess = () => resolve(req.result);
    req.onerror   = () => reject(req.error);
  });
}

async function popDue(db) {
  const now = Date.now();
  return new Promise((resolve, reject) => {
    const tx    = db.transaction(STORE, 'readwrite');
    const store = tx.objectStore(STORE);
    const index = store.index('next_attempt');
    const range = IDBKeyRange.upperBound(now);
    const items = [];
    const cursor = index.openCursor(range);
    cursor.onsuccess = (e) => {
      const c = e.target.result;
      if (!c) { resolve(items); return; }
      items.push({ id: c.primaryKey, ...c.value });
      c.continue();
    };
    cursor.onerror = () => reject(cursor.error);
  });
}

async function deleteRecord(db, id) {
  return new Promise((resolve, reject) => {
    const tx  = db.transaction(STORE, 'readwrite');
    const req = tx.objectStore(STORE).delete(id);
    req.onsuccess = () => resolve();
    req.onerror   = () => reject(req.error);
  });
}

async function requeueRecord(db, record) {
  await deleteRecord(db, record.id);
  await enqueue(db, record.event, record.attempt + 1);
}

// ---------------------------------------------------------------------------
// HTTP send
// ---------------------------------------------------------------------------

/**
 * @param {string} endpoint  Full URL of /api/v1/kb/ingest
 * @param {string} token     Bearer token (CO_KB_TOKEN)
 * @param {object} event     KbIngestBody payload
 * @returns {Promise<boolean>} true on 2xx, false on retriable error, throws on fatal
 */
async function sendEvent(endpoint, token, event) {
  let resp;
  try {
    resp = await fetch(endpoint, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Authorization': `Bearer ${token}`,
      },
      body: JSON.stringify(event),
    });
  } catch (_networkErr) {
    return false; // network down → retry
  }

  if (resp.ok || resp.status === 202) return true;
  if (resp.status === 401 || resp.status === 400) {
    throw new Error(`kb-sync: fatal ${resp.status} — dropping event`);
  }
  return false; // 5xx / 503 → retry
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

let _db = null;
let _drainTimer = null;

async function getDb() {
  if (!_db) _db = await openDb();
  return _db;
}

/**
 * Queue a KbIngestEvent for async delivery.
 * Called immediately after a local entry write — never awaited on the write path.
 *
 * @param {object} event  KbIngestBody payload (see CO-367 design)
 */
export async function queueIngest(event) {
  try {
    const db = await getDb();
    await enqueue(db, event, 0);
  } catch (e) {
    console.warn('kb-sync: failed to enqueue event', e);
  }
}

/**
 * Drain all due events from the queue.
 *
 * @param {string} endpoint  Full URL of /api/v1/kb/ingest
 * @param {string} token     Bearer token
 */
export async function drain(endpoint, token) {
  if (!endpoint || !token) return;
  let db;
  try {
    db = await getDb();
  } catch (e) {
    console.warn('kb-sync: IndexedDB unavailable', e);
    return;
  }

  const due = await popDue(db);
  for (const record of due) {
    try {
      const sent = await sendEvent(endpoint, token, record.event);
      if (sent) {
        await deleteRecord(db, record.id);
      } else if (record.attempt < RETRY_DELAYS_MS.length - 1) {
        await requeueRecord(db, record);
      } else {
        console.warn('kb-sync: max retries reached, dropping event', record.event);
        await deleteRecord(db, record.id);
      }
    } catch (fatal) {
      console.warn('kb-sync:', fatal.message);
      await deleteRecord(db, record.id);
    }
  }
}

/**
 * Start the drain loop. Called once at app boot / when the user comes online.
 *
 * @param {string} endpoint
 * @param {string} token
 * @param {number} [intervalMs=60000]  How often to check for due events
 */
export function startDrainLoop(endpoint, token, intervalMs = 60_000) {
  if (_drainTimer !== null) return; // already running
  const tick = () => drain(endpoint, token).catch(() => {});
  tick(); // drain immediately on start
  _drainTimer = setInterval(tick, intervalMs);

  // Also drain on reconnect
  window.addEventListener('online', tick, { passive: true });
}

/** Stop the drain loop (e.g. on logout). */
export function stopDrainLoop() {
  if (_drainTimer !== null) {
    clearInterval(_drainTimer);
    _drainTimer = null;
  }
}
