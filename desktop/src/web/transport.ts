// Web transport (swapped in for src/lib/transport.ts by vite.web.config.ts).
// Boots the OPFS sync worker + WASI reactor worker once, asks the browser for persistent storage,
// and implements the same invoke / photoUrl / loadPhotoUrl / setWindowTheme contract as Tauri
// (including rejecting invoke promises with plain string messages, matching Tauri's Result<T, String>).

import { DATA_BYTES } from './opfs-protocol';
import { fetchPhotoBytes, importUrlWeb, labelLookupWeb, offSearchWeb } from './network';
import { initPwa, subscribeInstall, triggerInstall, type InstallOutcome } from './pwa';

initPwa();

export const isWeb = true;
export { subscribeInstall, triggerInstall, type InstallOutcome };

export type StorageStatus = { persisted: boolean | null; lastBackupAt: number | null };

const BACKUP_TS_KEY = 'tiffin-last-backup-at';
let memoryLastBackupAt: number | null = null;

function readLastBackupAt(): number | null {
  try {
    const raw = localStorage.getItem(BACKUP_TS_KEY);
    if (raw) {
      const n = Number(raw);
      if (Number.isFinite(n) && n > 0) return n;
    }
  } catch {
    /* private mode */
  }
  return memoryLastBackupAt;
}

function recordLastBackupAt(ts: number) {
  memoryLastBackupAt = ts;
  try {
    localStorage.setItem(BACKUP_TS_KEY, String(ts));
  } catch {
    /* private mode */
  }
}

export async function storageStatus(): Promise<StorageStatus> {
  let persisted: boolean | null = null;
  try {
    if (typeof navigator.storage?.persisted === 'function') {
      persisted = await navigator.storage.persisted();
    }
  } catch {
    persisted = null;
  }
  return { persisted, lastBackupAt: readLastBackupAt() };
}

export async function requestPersistentStorage(): Promise<boolean> {
  try {
    if (typeof navigator.storage?.persist === 'function') {
      return await navigator.storage.persist();
    }
  } catch {
    /* unsupported or denied */
  }
  return false;
}

type Pending = { ok: (value: unknown) => void; err: (reason: string) => void };
type Session = { io: Worker; wasm: Worker | null; dead: boolean; pending: Map<number, Pending> };

const DESKTOP_ONLY = new Set(['sync_now', 'send_all_to_tracker', 'setup_inbox', 'open_folder', 'rules_from_sync', 'install_update']);

let activeSession: Session | null = null;
let bootPromise: Promise<Session> | null = null;
let nextId = 1;
let photoGen = 0;
const photoCache = new Map<string, string>();
const photoInFlight = new Map<string, Promise<string>>();
const photoPending = new Map<string, Promise<void>>();

function clearPhotoCache() {
  photoGen++;
  for (const url of photoCache.values()) URL.revokeObjectURL(url);
  photoCache.clear();
}

function tearDown(s: Session, reason: string) {
  if (s.dead) return;
  s.dead = true;
  s.wasm?.terminate();
  s.io.terminate();
  for (const p of s.pending.values()) p.err(reason);
  s.pending.clear();
  if (activeSession === s) {
    activeSession = null;
    bootPromise = null;
  }
}

function webAddress(u: string): boolean {
  return (u.startsWith('https://') || u.startsWith('http://')) && !/[\s\p{Cc}"]/u.test(u);
}

function wasmUrl(): string {
  const base = import.meta.env.BASE_URL || '/';
  return new URL(base.endsWith('/') ? `${base}tiffin.wasm` : `${base}/tiffin.wasm`, location.origin).href;
}

function boot(): Promise<Session> {
  if (bootPromise) return bootPromise;
  const p = (async (): Promise<Session> => {
    if (!crossOriginIsolated || typeof SharedArrayBuffer === 'undefined') {
      throw 'Tiffin on the web needs cross-origin isolation (COOP/COEP) so browser storage saves atomically.';
    }
    // Ask the browser to keep OPFS data across storage pressure; non-blocking if denied or unsupported.
    navigator.storage?.persist?.()
      .then((granted) => {
        if (granted) window.dispatchEvent(new Event('tiffin-storage-change'));
      })
      .catch(() => false);

    const ctrl = new SharedArrayBuffer(16);
    const data = new SharedArrayBuffer(DATA_BYTES);
    const channel = new MessageChannel();

    const io = new Worker(new URL('./opfs-io.worker.ts', import.meta.url), { type: 'module' });
    const session: Session = { io, wasm: null, dead: false, pending: new Map() };
    activeSession = session;

    let failBoot: ((msg: string) => void) | null = null;
    io.onerror = (ev) => {
      const msg = ev.message || 'The browser storage worker stopped unexpectedly.';
      tearDown(session, msg);
      failBoot?.(msg);
    };

    try {
      await new Promise<void>((ok, err) => {
        failBoot = err;
        io.onmessage = (e) => {
          if (e.data?.type === 'ready') {
            failBoot = null;
            ok();
          }
        };
        io.postMessage({ type: 'init', ctrl, data, port: channel.port1, mounts: { '/data': 'tiffin-data' } }, [channel.port1]);
      });

      const wasm = new Worker(new URL('./tiffin.worker.ts', import.meta.url), { type: 'module' });
      session.wasm = wasm;
      await new Promise<void>((ok, err) => {
        failBoot = err;
        wasm.onerror = (ev) => {
          const msg = ev.message || 'Could not start Tiffin WebAssembly.';
          tearDown(session, msg);
          err(msg);
        };
        wasm.onmessage = (e) => {
          if (e.data?.type === 'ready') {
            failBoot = null;
            ok();
          } else if (e.data?.type === 'init-error') {
            failBoot = null;
            err(String(e.data.error));
          }
        };
        wasm.postMessage({ type: 'init', wasmUrl: wasmUrl(), ctrl, data, port: channel.port2 }, [channel.port2]);
      });

      if (session.dead) throw 'The browser storage worker stopped unexpectedly.';

      wasm.onmessage = (e) => {
        if (session.dead) return;
        const m = e.data;
        if (m?.type !== 'reply') return;
        const waiter = session.pending.get(m.id);
        if (!waiter) return;
        session.pending.delete(m.id);
        if (m.ok) waiter.ok(m.value);
        else waiter.err(String(m.error));
      };
      wasm.onerror = (ev) => {
        tearDown(session, ev.message || 'Tiffin WebAssembly stopped unexpectedly.');
      };

      // Warm the 18k-food search index after the worker is ready, without delaying boot.
      setTimeout(() => {
        if (!session.dead && activeSession === session) {
          callWorker(session, 'warm_foods', null).catch(() => {});
        }
      }, 0);
      return session;
    } catch (e) {
      const msg = typeof e === 'string' ? e : e instanceof Error ? e.message : String(e);
      if (!session.wasm) channel.port2.close();
      tearDown(session, msg);
      throw msg;
    }
  })().catch((e) => {
    if (bootPromise === p) bootPromise = null;
    throw typeof e === 'string' ? e : e instanceof Error ? e.message : String(e);
  });
  bootPromise = p;
  return p;
}

function callWorker<T>(session: Session, cmd: string, args: unknown): Promise<T> {
  if (session.dead || !session.wasm) return Promise.reject('Tiffin WebAssembly is not running.');
  return new Promise<T>((ok, err) => {
    const id = nextId++;
    session.pending.set(id, { ok: ok as (v: unknown) => void, err });
    session.wasm!.postMessage({ type: 'call', id, cmd, args });
  });
}

function callWorkerBytes<T>(session: Session, cmd: string, arg: string, bin: Uint8Array, rawReply = false): Promise<T> {
  if (session.dead || !session.wasm) return Promise.reject('Tiffin WebAssembly is not running.');
  return new Promise<T>((ok, err) => {
    const id = nextId++;
    session.pending.set(id, { ok: ok as (v: unknown) => void, err });
    const copy = bin.slice();
    session.wasm!.postMessage({ type: 'call-bytes', id, cmd, arg, bin: copy, rawReply }, [copy.buffer]);
  });
}

function triggerDownload(filename: string, text: string) {
  const blob = new Blob([text], { type: 'application/json' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  a.style.display = 'none';
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
}

export async function invoke<T>(cmd: string, args?: Record<string, unknown> | Uint8Array, options?: { headers?: Record<string, string> }): Promise<T> {
  if (cmd === 'open_link') {
    const raw = args != null && typeof args === 'object' && !(args instanceof Uint8Array) ? String(args.url ?? '').trim() : '';
    if (!webAddress(raw)) throw "That isn't a web address.";
    window.open(raw, '_blank', 'noopener,noreferrer');
    return undefined as T;
  }
  if (DESKTOP_ONLY.has(cmd)) {
    throw 'This feature is only available in the Tiffin desktop app.';
  }

  const session = await boot();
  const call = <R>(c: string, a: unknown) => callWorker<R>(session, c, a);

  if (cmd === 'barcode_photo') {
    if (!(args instanceof Uint8Array)) throw 'No picture was sent.';
    const code = await callWorkerBytes<string>(session, 'barcode_photo', '', args, false);
    return (code || null) as T;
  }

  if (cmd === 'label_photo') {
    if (!(args instanceof Uint8Array)) throw 'No picture was sent.';
    const name = options?.headers?.name ?? 'label';
    const json = await callWorkerBytes<string>(session, 'label_photo', name, args, false);
    return JSON.parse(json) as T;
  }

  if (cmd === 'import_url') {
    const rawUrl = args != null && typeof args === 'object' && !(args instanceof Uint8Array) ? String(args.url ?? '') : '';
    return await importUrlWeb<T>(call, rawUrl);
  }

  if (cmd === 'off_search') {
    const q = args != null && typeof args === 'object' && !(args instanceof Uint8Array) ? String(args.q ?? '') : '';
    return await offSearchWeb<T>(call, q);
  }

  if (cmd === 'label_lookup') {
    const input = args != null && typeof args === 'object' && !(args instanceof Uint8Array) ? String(args.input ?? '') : '';
    return await labelLookupWeb<T>(call, input);
  }

  if (cmd === 'save_review') {
    const res = await callWorker<{ saved: T; photo: { url: string; file: string } | null }>(session, 'web_save_review', args ?? null);
    if (res.photo) {
      const { url, file } = res.photo;
      const gen = photoGen;
      const pending = fetchPhotoBytes(url)
        .then(async (bytes) => {
          if (!bytes || session.dead || gen !== photoGen) return;
          await callWorkerBytes(session, 'write_photo', file, bytes, false);
          const old = photoCache.get(file);
          if (old) {
            URL.revokeObjectURL(old);
            photoCache.delete(file);
          }
        })
        .catch(() => {})
        .finally(() => {
          if (photoPending.get(file) === pending) photoPending.delete(file);
        });
      photoPending.set(file, pending);
    }
    return res.saved;
  }

  if (cmd === 'add_photo') {
    if (!(args instanceof Uint8Array)) throw 'No picture was sent.';
    const stem = options?.headers?.stem ?? 'photo';
    const name = await callWorkerBytes<string>(session, 'add_photo', stem, args, false);
    const old = photoCache.get(name);
    if (old) {
      URL.revokeObjectURL(old);
      photoCache.delete(name);
    }
    return name as T;
  }

  if (cmd === 'backup_now') {
    const exp = await callWorker<{ filename: string; text: string; message: string }>(session, 'web_backup_export', null);
    triggerDownload(exp.filename, exp.text);
    recordLastBackupAt(Date.now());
    return exp.message as T;
  }

  const out = await callWorker<T>(session, cmd, args ?? null);
  if (cmd === 'restore_backup') clearPhotoCache();
  return out;
}

export const photoUrl = (name: string) => photoCache.get(name) ?? '';

export async function loadPhotoUrl(name: string): Promise<string> {
  const cached = photoCache.get(name);
  if (cached) return cached;
  const existing = photoInFlight.get(name);
  if (existing) return existing;
  const job = (async () => {
    const pending = photoPending.get(name);
    if (pending) await pending;
    const afterPending = photoCache.get(name);
    if (afterPending) return afterPending;
    const gen = photoGen;
    const session = await boot();
    const bytes = await callWorkerBytes<Uint8Array>(session, 'read_photo', name, new Uint8Array(), true);
    const type = name.endsWith('.png') ? 'image/png' : 'image/jpeg';
    const url = URL.createObjectURL(new Blob([bytes as Uint8Array<ArrayBuffer>], { type }));
    if (gen === photoGen) {
      photoCache.set(name, url);
    } else {
      URL.revokeObjectURL(url);
      throw 'Photo cache cleared.';
    }
    return url;
  })().finally(() => {
    if (photoInFlight.get(name) === job) photoInFlight.delete(name);
  });
  photoInFlight.set(name, job);
  return job;
}

export const setWindowTheme = (_theme: 'light' | 'dark' | null) => Promise.resolve();
