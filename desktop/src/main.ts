import { mount } from 'svelte'
import { invoke } from '@tauri-apps/api/core'
import './app.css'
import App from './App.svelte'

// Windows 11: Rust put Mica behind the window; let the sidebar show it (see app.css).
const micaAnswer = invoke<boolean>('mica').then((on) => { if (on) document.documentElement.classList.add('mica') }, () => {})

const app = mount(App, {
  target: document.getElementById('app')!,
})

// The window starts hidden; show it once the first paint has happened.
micaAnswer.then(() => requestAnimationFrame(() => requestAnimationFrame(() => {
  invoke('show_window').catch(() => {}) // Rust shows the window after 5 s anyway
})))

export default app
