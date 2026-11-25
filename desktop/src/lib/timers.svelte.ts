// Kitchen timers, shared by the recipe page and cook mode. They count against the clock
// (an end time), so they stay right when the window is hidden or the computer is busy.

import { isWeb } from './api';

export type Timer = {
  id: number;
  label: string;
  total: number; // seconds
  left: number; // seconds, updated every tick
  endsAt: number; // ms; 0 while paused
  done: boolean;
};

export const timers: Timer[] = $state([]);
let nextId = 1;
let ticker = 0;
let ringer = 0;
let ringGen = 0;
let resumePending = false;
let pendingChimeGen = -1;
let listeningVisibility = false;

export function startTimer(label: string, seconds: number) {
  if (!Number.isFinite(seconds) || seconds <= 0) return;
  unlockAudio();
  timers.push({ id: nextId++, label, total: seconds, left: seconds, endsAt: Date.now() + seconds * 1000, done: false });
  ensureTicking();
  syncTitle();
}

export function toggleTimer(t: Timer) {
  if (t.done) return;
  if (!t.endsAt) unlockAudio();
  t.endsAt = t.endsAt ? 0 : Date.now() + t.left * 1000;
  ensureTicking();
  syncTitle();
}

export function removeTimer(t: Timer) {
  const i = timers.findIndex((x) => x.id === t.id);
  if (i >= 0) timers.splice(i, 1);
  if (!timers.some((x) => x.done)) stopRinging();
  if (!timers.some((x) => x.endsAt)) stopTicking();
  syncTitle();
}

export function formatLeft(secs: number) {
  const s = Number.isFinite(secs) ? Math.max(0, Math.ceil(secs)) : 0;
  const mm = String(Math.floor((s % 3600) / 60)).padStart(s >= 3600 ? 2 : 1, '0');
  const ss = String(s % 60).padStart(2, '0');
  return s >= 3600 ? `${Math.floor(s / 3600)}:${mm}:${ss}` : `${mm}:${ss}`;
}

/** Browser tab title reflecting the most urgent timer: finished timers take priority, then the soonest running timer. */
export function timerTitle(list: readonly Timer[]): string {
  const done = list.filter((t) => t.done);
  if (done.length > 1) return `Done (${done.length}) · Tiffin`;
  if (done.length === 1) {
    const label = done[0].label.trim();
    return label ? `Done · ${label} · Tiffin` : 'Done · Tiffin';
  }
  let soonest: Timer | null = null;
  for (const t of list) {
    if (!t.endsAt) continue;
    if (!soonest || t.left < soonest.left) soonest = t;
  }
  return soonest ? `${formatLeft(soonest.left)} · Tiffin` : 'Tiffin';
}

function syncTitle() {
  if (!isWeb || typeof document === 'undefined') return;
  const next = timerTitle(timers);
  if (document.title !== next) document.title = next;
}

function step() {
  const now = Date.now();
  let finished = false;
  for (const t of timers) {
    if (!t.endsAt || t.done) continue;
    t.left = Math.max(0, (t.endsAt - now) / 1000);
    if (t.left === 0) {
      t.done = true;
      t.endsAt = 0;
      finished = true;
    }
  }
  if (finished) ring();
  if (!timers.some((t) => t.endsAt)) stopTicking();
  syncTitle();
}

function onVisibility() {
  if (typeof document !== 'undefined' && !document.hidden) step();
}

function ensureTicking() {
  if (!timers.some((t) => t.endsAt)) {
    stopTicking();
    return;
  }
  if (!listeningVisibility && typeof document !== 'undefined') {
    document.addEventListener('visibilitychange', onVisibility);
    listeningVisibility = true;
  }
  if (ticker) return;
  ticker = window.setInterval(step, 250);
}

function stopTicking() {
  if (ticker) {
    clearInterval(ticker);
    ticker = 0;
  }
  if (listeningVisibility && typeof document !== 'undefined') {
    document.removeEventListener('visibilitychange', onVisibility);
    listeningVisibility = false;
  }
}

// A finished timer chimes now and every 10 s until it's dismissed, for at most two minutes.
function ring() {
  chime();
  if (ringer) return;
  let left = 12;
  ringer = window.setInterval(() => {
    if (--left <= 0 || !timers.some((t) => t.done)) return stopRinging();
    chime();
  }, 10_000);
}

function stopRinging() {
  clearInterval(ringer);
  ringer = 0;
  ringGen++;
}

let audio: AudioContext | null = null;

// Create and resume AudioContext during a user gesture (Start/Resume) so browsers don't mute the chime later.
function unlockAudio() {
  try {
    if (typeof AudioContext === 'undefined') return;
    audio ??= new AudioContext();
    if (audio.state === 'suspended') {
      audio.resume().catch(() => {});
    }
  } catch {
    /* no sound device: the timer still shows as done */
  }
}

function playBell(ctx: AudioContext) {
  const t0 = ctx.currentTime;
  // three soft bell notes, rising
  [880, 1108.7, 1318.5].forEach((f, i) => {
    const o = ctx.createOscillator();
    const g = ctx.createGain();
    o.type = 'sine';
    o.frequency.value = f;
    const at = t0 + i * 0.22;
    g.gain.setValueAtTime(0, at);
    g.gain.linearRampToValueAtTime(0.25, at + 0.02);
    g.gain.exponentialRampToValueAtTime(0.001, at + 0.9);
    o.connect(g).connect(ctx.destination);
    o.start(at);
    o.stop(at + 0.95);
  });
}

function chime() {
  try {
    if (typeof AudioContext === 'undefined') return;
    audio ??= new AudioContext();
    if (audio.state === 'suspended') {
      pendingChimeGen = ringGen;
      if (resumePending) return;
      const ctx = audio;
      resumePending = true;
      ctx.resume()
        .then(() => {
          if (pendingChimeGen === ringGen && timers.some((t) => t.done) && ctx.state === 'running') {
            playBell(ctx);
          }
        })
        .catch(() => {})
        .finally(() => {
          resumePending = false;
        });
      return;
    }
    playBell(audio);
  } catch {
    /* no sound device: the timer still shows as done */
  }
}
