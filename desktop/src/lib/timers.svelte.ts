// Kitchen timers, shared by the recipe page and cook mode. They count against the clock
// (an end time), so they stay right when the window is hidden or the computer is busy.

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

export function startTimer(label: string, seconds: number) {
  timers.push({ id: nextId++, label, total: seconds, left: seconds, endsAt: Date.now() + seconds * 1000, done: false });
  ensureTicking();
}

export function toggleTimer(t: Timer) {
  if (t.done) return;
  t.endsAt = t.endsAt ? 0 : Date.now() + t.left * 1000;
  ensureTicking();
}

export function removeTimer(t: Timer) {
  const i = timers.findIndex((x) => x.id === t.id);
  if (i >= 0) timers.splice(i, 1);
  if (!timers.some((x) => x.done)) stopRinging();
}

export function formatLeft(secs: number) {
  const s = Math.ceil(secs);
  const mm = String(Math.floor((s % 3600) / 60)).padStart(s >= 3600 ? 2 : 1, '0');
  const ss = String(s % 60).padStart(2, '0');
  return s >= 3600 ? `${Math.floor(s / 3600)}:${mm}:${ss}` : `${mm}:${ss}`;
}

function ensureTicking() {
  if (ticker) return;
  ticker = window.setInterval(() => {
    const now = Date.now();
    for (const t of timers) {
      if (!t.endsAt || t.done) continue;
      t.left = Math.max(0, (t.endsAt - now) / 1000);
      if (t.left === 0) {
        t.done = true;
        t.endsAt = 0;
        ring();
      }
    }
    if (!timers.some((t) => t.endsAt)) {
      clearInterval(ticker);
      ticker = 0;
    }
  }, 250);
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
}

let audio: AudioContext | null = null;
function chime() {
  try {
    audio ??= new AudioContext();
    const t0 = audio.currentTime;
    // three soft bell notes, rising
    [880, 1108.7, 1318.5].forEach((f, i) => {
      const o = audio!.createOscillator();
      const g = audio!.createGain();
      o.type = 'sine';
      o.frequency.value = f;
      const at = t0 + i * 0.22;
      g.gain.setValueAtTime(0, at);
      g.gain.linearRampToValueAtTime(0.25, at + 0.02);
      g.gain.exponentialRampToValueAtTime(0.001, at + 0.9);
      o.connect(g).connect(audio!.destination);
      o.start(at);
      o.stop(at + 0.95);
    });
  } catch {
    /* no sound device: the timer still shows as done */
  }
}
