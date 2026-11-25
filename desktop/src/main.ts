import { mount } from 'svelte'
import { invoke } from './lib/transport'
import './app.css'
import App from './App.svelte'
import { applyTheme, savedTheme } from './lib/theme'

// Windows 11: Rust put Mica behind the window; let the sidebar show it (see app.css).
const micaAnswer = invoke<boolean>('mica').then((on) => { if (on) document.documentElement.classList.add('mica') }, () => {})

// The remembered theme goes on before the first paint; the window theme (title bar, Mica) is awaited before show.
const themed = applyTheme(savedTheme())

const app = mount(App, {
  target: document.getElementById('app')!,
})

// The window starts hidden; show it once the first paint has happened.
Promise.all([micaAnswer, themed]).then(() => requestAnimationFrame(() => requestAnimationFrame(() => {
  invoke('show_window').catch(() => {}) // Rust shows the window after 5 s anyway
})))

export default app
