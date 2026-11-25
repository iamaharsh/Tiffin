// Browser network bridge for Tiffin on the web (Milestone 2, Phase 3).
// Talks to tiffin-helper (/page, /photo, /off) and Open Food Facts' direct product API while
// keeping all URL validation, rate limiting, email checks, and parsing in Rust (tiffin-web-wasm).
// Never logs URLs, search text, emails, or response contents.

const PAGE_MAX_BYTES = 10 * 1024 * 1024;
const PICTURE_MAX_BYTES = 15 * 1024 * 1024;
const OFF_MAX_BYTES = 2 * 1024 * 1024;
const ERR_MAX_BYTES = 4 * 1024;

const PAGE_TIMEOUT_MS = 55_000;
const OFF_PRODUCT_TIMEOUT_MS = 8_000;
const OFF_SEARCH_TIMEOUT_MS = 12_000;
const LABEL_OFF_TIMEOUT_MS = 20_000;

const PAGE_TOO_BIG = "The page is larger than 10 MB, so it wasn't imported.";
const PICTURE_TOO_BIG = "The picture is larger than 15 MB, so it wasn't downloaded.";
const BARCODE_NOT_FOUND = "That barcode isn't in Open Food Facts. Try a photo of the label.";

const decoder = new TextDecoder('utf-8', { fatal: false });

export type WorkerCaller = <T>(cmd: string, args: unknown) => Promise<T>;

export function helperBase(): string {
  const raw = String(import.meta.env.VITE_HELPER_URL ?? '').trim();
  if (!raw) return '';
  try {
    const u = new URL(raw);
    if ((u.protocol !== 'http:' && u.protocol !== 'https:') || u.username || u.password || u.search || u.hash) {
      return '';
    }
    return u.href.replace(/\/+$/, '');
  } catch {
    return '';
  }
}

class TooBigError extends Error {}

function isAbortError(e: unknown): boolean {
  return typeof e === 'object' && e !== null && (e as { name?: unknown }).name === 'AbortError';
}

function retryAfterSecs(res: Response): number {
  const raw = res.headers.get('Retry-After')?.trim();
  if (!raw || !/^\d+$/.test(raw)) return 60;
  const n = Number(raw);
  return Math.min(3600, Math.max(1, n));
}

async function readBytesCapped(res: Response, maxBytes: number, tooBigMsg: string): Promise<Uint8Array> {
  const cl = res.headers.get('Content-Length')?.trim();
  if (cl && /^\d+$/.test(cl) && Number(cl) > maxBytes) {
    await res.body?.cancel().catch(() => {});
    throw new TooBigError(tooBigMsg);
  }
  if (!res.body) {
    const buf = new Uint8Array(await res.arrayBuffer());
    if (buf.byteLength > maxBytes) throw new TooBigError(tooBigMsg);
    return buf;
  }
  const reader = res.body.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      if (!value) continue;
      total += value.byteLength;
      if (total > maxBytes) {
        await reader.cancel().catch(() => {});
        throw new TooBigError(tooBigMsg);
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  const out = new Uint8Array(total);
  let offset = 0;
  for (const c of chunks) {
    out.set(c, offset);
    offset += c.byteLength;
  }
  return out;
}

async function fetchTimed<T>(url: string, init: RequestInit, timeoutMs: number, handle: (res: Response) => Promise<T>): Promise<T> {
  const ac = new AbortController();
  const timer = setTimeout(() => ac.abort(), timeoutMs);
  try {
    const res = await fetch(url, { credentials: 'omit', referrerPolicy: 'no-referrer', ...init, signal: ac.signal });
    return await handle(res);
  } finally {
    clearTimeout(timer);
  }
}

async function readErrorBody(res: Response): Promise<string> {
  const ct = (res.headers.get('content-type') ?? '').toLowerCase();
  if (!ct.startsWith('text/plain')) {
    await res.body?.cancel().catch(() => {});
    return '';
  }
  try {
    const bytes = await readBytesCapped(res, ERR_MAX_BYTES, '');
    return decoder.decode(bytes).trim();
  } catch {
    return '';
  }
}

async function fetchPageHtml(helper: string, targetUrl: string): Promise<string> {
  const endpoint = `${helper}/page?url=${encodeURIComponent(targetUrl)}`;
  try {
    return await fetchTimed(endpoint, { method: 'GET' }, PAGE_TIMEOUT_MS, async (res) => {
      if (!res.ok) {
        const body = await readErrorBody(res);
        throw body || `Tiffin's helper returned HTTP ${res.status}.`;
      }
      const bytes = await readBytesCapped(res, PAGE_MAX_BYTES, PAGE_TOO_BIG);
      return decoder.decode(bytes);
    });
  } catch (e) {
    if (typeof e === 'string') throw e;
    if (e instanceof TooBigError) throw e.message;
    if (isAbortError(e)) {
      throw 'The site took too long to answer, so Tiffin stopped.';
    }
    throw "Couldn't reach Tiffin's helper server.";
  }
}

export async function fetchPhotoBytes(photoUrl: string): Promise<Uint8Array | null> {
  if (!/^https?:\/\//i.test(photoUrl)) return null;
  const helper = helperBase();
  if (!helper) return null;
  const endpoint = `${helper}/photo?url=${encodeURIComponent(photoUrl)}`;
  try {
    return await fetchTimed(endpoint, { method: 'GET' }, PAGE_TIMEOUT_MS, async (res) => {
      if (!res.ok) {
        await res.body?.cancel().catch(() => {});
        return null;
      }
      return await readBytesCapped(res, PICTURE_MAX_BYTES, PICTURE_TOO_BIG);
    });
  } catch {
    return null;
  }
}

export async function importUrlWeb<T>(call: WorkerCaller, rawUrl: string): Promise<T> {
  const normalized = await call<string>('web_prepare_import_url', { url: rawUrl });
  const helper = helperBase();
  if (!helper) {
    throw 'Online recipe import needs a Tiffin helper server. Paste the recipe text instead.';
  }
  let html: string;
  try {
    html = await fetchPageHtml(helper, normalized);
  } catch (e) {
    const msg = typeof e === 'string' ? e : e instanceof Error ? e.message : String(e);
    throw `Couldn't read ${normalized}: ${msg}`;
  }
  return await call<T>('web_finish_import_url', { html, url: normalized });
}

type OffPrep =
  | { kind: 'done'; result: unknown }
  | { kind: 'product'; code: string; url: string; user_agent: string }
  | { kind: 'search'; text: string; email: string };

type OffAnswerWire = { answer: 'empty' | 'offline' | 'slow' | 'needs_email' } | { answer: 'limited'; value: number } | { answer: 'found'; value: unknown[] };

function isOffAnswer(v: unknown): v is OffAnswerWire {
  if (typeof v !== 'object' || v === null) return false;
  const o = v as { answer?: unknown; value?: unknown };
  switch (o.answer) {
    case 'empty':
    case 'offline':
    case 'slow':
    case 'needs_email':
      return o.value === undefined;
    case 'limited':
      return typeof o.value === 'number' && Number.isSafeInteger(o.value) && o.value >= 0;
    case 'found':
      return Array.isArray(o.value);
    default:
      return false;
  }
}

export async function offSearchWeb<T>(call: WorkerCaller, q: string): Promise<T> {
  const prep = await call<OffPrep>('web_off_prepare', { q });
  if (prep.kind === 'done') return prep.result as T;

  if (prep.kind === 'product') {
    let body: string | null = null;
    let fallback: OffAnswerWire | null = null;
    try {
      await fetchTimed(
        prep.url,
        { method: 'GET', headers: { 'X-User-Agent': prep.user_agent }, redirect: 'error' },
        OFF_PRODUCT_TIMEOUT_MS,
        async (res) => {
          if (res.status === 404) {
            await res.body?.cancel().catch(() => {});
            fallback = { answer: 'empty' };
            return;
          }
          if (res.status === 429 || res.status === 503) {
            await res.body?.cancel().catch(() => {});
            fallback = { answer: 'limited', value: retryAfterSecs(res) };
            return;
          }
          if (res.status !== 200) {
            await res.body?.cancel().catch(() => {});
            fallback = { answer: 'slow' };
            return;
          }
          const bytes = await readBytesCapped(res, OFF_MAX_BYTES, 'too big');
          body = decoder.decode(bytes);
        },
      );
    } catch (e) {
      if (e instanceof TooBigError || isAbortError(e)) {
        fallback = { answer: 'slow' };
      } else {
        fallback = { answer: 'offline' };
      }
    }
    if (body !== null) {
      return await call<T>('web_off_finish_product', { q, code: prep.code, body });
    }
    return await call<T>('web_off_finish', { q, barcode: true, answer: fallback ?? { answer: 'slow' } });
  }

  // Name search through tiffin-helper (/off)
  const helper = helperBase();
  let answer: OffAnswerWire = { answer: 'offline' };
  if (helper) {
    const endpoint = `${helper}/off?q=${encodeURIComponent(prep.text)}&email=${encodeURIComponent(prep.email)}`;
    try {
      answer = await fetchTimed(endpoint, { method: 'GET' }, OFF_SEARCH_TIMEOUT_MS, async (res) => {
        if (res.status === 429) {
          await res.body?.cancel().catch(() => {});
          return { answer: 'limited', value: retryAfterSecs(res) };
        }
        if (res.status !== 200) {
          await res.body?.cancel().catch(() => {});
          return { answer: 'slow' };
        }
        const bytes = await readBytesCapped(res, OFF_MAX_BYTES, 'too big');
        const parsed: unknown = JSON.parse(decoder.decode(bytes));
        return isOffAnswer(parsed) ? parsed : { answer: 'slow' };
      });
    } catch (e) {
      if (e instanceof TooBigError || e instanceof SyntaxError || isAbortError(e)) {
        answer = { answer: 'slow' };
      } else {
        answer = { answer: 'offline' };
      }
    }
  }
  return await call<T>('web_off_finish', { q, barcode: false, answer });
}

type LabelPrep =
  | { kind: 'barcode'; code: string; url: string; user_agent: string }
  | { kind: 'url'; url: string };

export async function labelLookupWeb<T>(call: WorkerCaller, input: string): Promise<T> {
  const prep = await call<LabelPrep>('web_label_prepare', { input });
  if (prep.kind === 'barcode') {
    let body: string;
    try {
      body = await fetchTimed(
        prep.url,
        { method: 'GET', headers: { 'X-User-Agent': prep.user_agent }, redirect: 'error' },
        LABEL_OFF_TIMEOUT_MS,
        async (res) => {
          if (res.status === 404) {
            await res.body?.cancel().catch(() => {});
            throw BARCODE_NOT_FOUND;
          }
          if (res.status !== 200) {
            await res.body?.cancel().catch(() => {});
            throw `Couldn't reach Open Food Facts: HTTP ${res.status}`;
          }
          const bytes = await readBytesCapped(res, OFF_MAX_BYTES, "Couldn't reach Open Food Facts: response too large.");
          return decoder.decode(bytes);
        },
      );
    } catch (e) {
      if (typeof e === 'string') throw e;
      if (e instanceof TooBigError) throw e.message;
      if (isAbortError(e)) {
        throw "Couldn't reach Open Food Facts: timed out.";
      }
      throw "Couldn't reach Open Food Facts.";
    }
    return await call<T>('web_label_finish_off', { code: prep.code, body });
  }

  const helper = helperBase();
  if (!helper) {
    throw 'Reading a product page needs a Tiffin helper server. Try the barcode number or enter the numbers from the label.';
  }
  const html = await fetchPageHtml(helper, prep.url);
  return await call<T>('web_label_finish_html', { html, url: prep.url });
}
