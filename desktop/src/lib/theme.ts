// Light, dark or Follow Windows, remembered on this computer. The page reads data-theme, and the window call keeps the title bar and Mica in step (G-02.1-8).
import { getCurrentWindow } from '@tauri-apps/api/window'

export type Theme = 'system' | 'light' | 'dark'

const remembered = (k: string) => { try { return localStorage.getItem(k); } catch { return null; } };
const remember = (k: string, v: string) => { try { localStorage.setItem(k, v); } catch { /* private mode */ } };

const query = matchMedia('(prefers-color-scheme: dark)')

export function savedTheme(): Theme {
  const t = remembered('theme')
  return t === 'light' || t === 'dark' ? t : 'system'
}

export function applyTheme(t: Theme): Promise<void> {
  document.documentElement.dataset.theme = t === 'system' ? (query.matches ? 'dark' : 'light') : t
  // The page is already themed, so a refused or missing window call only leaves the title bar on the Windows setting and must never stop start-up.
  return Promise.resolve().then(() => getCurrentWindow().setTheme(t === 'system' ? null : t)).catch(() => {})
}

export function chooseTheme(t: Theme): Promise<void> {
  remember('theme', t)
  return applyTheme(t)
}

// The query reports the window's theme after a forced choice and Windows again after Follow Windows, so this also corrects the moment of switching back.
query.addEventListener('change', () => {
  if (savedTheme() === 'system') document.documentElement.dataset.theme = query.matches ? 'dark' : 'light'
})
