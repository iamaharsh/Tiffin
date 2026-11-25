// Progressive Web App helpers for Tiffin on the web (service worker registration + install flow).
// Follows the Web Install API (`navigator.install()`) with `beforeinstallprompt` fallback, hiding
// any custom install button when neither mechanism is available or when already running standalone.

type BeforeInstallPromptEvent = Event & {
  prompt: () => Promise<void>;
  userChoice: Promise<{ outcome: 'accepted' | 'dismissed' }>;
};

type InstallNavigator = Navigator & {
  install?: () => Promise<void>;
};

export type InstallOutcome = 'installed' | 'cancelled' | 'unavailable' | 'error';

const listeners = new Set<(available: boolean) => void>();
let deferredPrompt: BeforeInstallPromptEvent | null = null;
let installed = false;
let installInFlight = false;
let initialized = false;

function isStandalone(): boolean {
  if (typeof window === 'undefined') return false;
  if (window.matchMedia?.('(display-mode: standalone)')?.matches) return true;
  return typeof navigator !== 'undefined' && (navigator as Navigator & { standalone?: boolean }).standalone === true;
}

function supportsWebInstall(): boolean {
  return typeof navigator !== 'undefined' && typeof (navigator as InstallNavigator).install === 'function';
}

export function canInstallApp(): boolean {
  if (typeof window === 'undefined' || !window.isSecureContext || isStandalone() || installed) return false;
  return supportsWebInstall() || deferredPrompt !== null;
}

function notify() {
  const ok = canInstallApp();
  for (const fn of listeners) fn(ok);
}

function registerServiceWorker() {
  if (import.meta.env.DEV || !window.isSecureContext || typeof navigator === 'undefined' || !('serviceWorker' in navigator)) return;
  const base = import.meta.env.BASE_URL || '/';
  const swUrl = base.endsWith('/') ? `${base}sw.js` : `${base}/sw.js`;
  navigator.serviceWorker.register(swUrl, { updateViaCache: 'none' }).catch(() => {});
}

export function initPwa() {
  if (initialized || typeof window === 'undefined') return;
  initialized = true;

  // Defer SW precaching until window load so it does not compete with first-visit WASM startup.
  if (typeof document !== 'undefined' && document.readyState === 'complete') {
    registerServiceWorker();
  } else {
    window.addEventListener('load', registerServiceWorker, { once: true });
  }

  window.addEventListener('beforeinstallprompt', (event) => {
    event.preventDefault();
    installed = false;
    deferredPrompt = event as BeforeInstallPromptEvent;
    notify();
  });

  window.addEventListener('appinstalled', () => {
    installed = true;
    deferredPrompt = null;
    notify();
  });

  window.matchMedia?.('(display-mode: standalone)')?.addEventListener?.('change', notify);
}

export function subscribeInstall(fn: (available: boolean) => void): () => void {
  initPwa();
  listeners.add(fn);
  fn(canInstallApp());
  return () => {
    listeners.delete(fn);
  };
}

/** Trigger browser-controlled app installation directly from a user click handler. */
export async function triggerInstall(): Promise<InstallOutcome> {
  if (installInFlight || typeof window === 'undefined' || !window.isSecureContext || isStandalone() || installed) return 'unavailable';
  installInFlight = true;
  try {
    if (supportsWebInstall()) {
      try {
        await (navigator as InstallNavigator).install!();
        installed = true;
        deferredPrompt = null;
        notify();
        return 'installed';
      } catch (e) {
        const name = typeof e === 'object' && e !== null && 'name' in e ? String((e as { name?: unknown }).name) : '';
        if (name === 'AbortError') return 'cancelled';
        if (name !== 'TypeError' || !deferredPrompt) return 'error';
        // Experimental signature mismatch: fall through to captured beforeinstallprompt if available.
      }
    }

    if (deferredPrompt) {
      // Single-use event: capture and clear before awaiting; notify listeners after settlement so focus isn't dropped mid-prompt.
      const ev = deferredPrompt;
      deferredPrompt = null;
      try {
        await ev.prompt();
        const { outcome } = await ev.userChoice;
        if (outcome === 'accepted') installed = true;
        return outcome === 'accepted' ? 'installed' : 'cancelled';
      } catch (e) {
        const name = typeof e === 'object' && e !== null && 'name' in e ? String((e as { name?: unknown }).name) : '';
        return name === 'AbortError' ? 'cancelled' : 'error';
      } finally {
        notify();
      }
    }

    return 'unavailable';
  } finally {
    installInFlight = false;
  }
}
