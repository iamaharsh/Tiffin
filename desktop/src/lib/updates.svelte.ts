// Update state shared by the banner and Settings > About. The check and the install run in Rust;
// this module only holds the answer. Nothing here is saved: "Later" lasts until the next launch (D-08).
import { untrack } from 'svelte';
import * as api from './api';
import type { UpdateInfo } from './api';

export type UpdateStatus = 'idle' | 'checking' | 'current' | 'available' | 'installing' | 'failed' | 'checkfailed';

export const update: {
  status: UpdateStatus;
  info: UpdateInfo | null;
  reason: string;
  checked: string;
  edits: number;
  unsaved: boolean;
  declined: boolean;
  whatsNew: boolean;
} = $state({ status: 'idle', info: null, reason: '', checked: '', edits: 0, unsaved: false, declined: false, whatsNew: false });

/** Call from a component's script: while `dirty()` is true the page holds typed input that isn't saved, and an install waits. */
export function holdsUnsaved(dirty: () => boolean) {
  $effect(() => {
    if (!dirty()) return;
    // untrack: the effect must depend on dirty() only, not on the count it writes
    untrack(() => {
      update.edits += 1;
      update.unsaved = update.edits > 0;
    });
    return () =>
      untrack(() => {
        update.edits -= 1;
        update.unsaved = update.edits > 0;
      });
  });
}

/** `manual` shows progress and failures (Settings > About); the launch check stays silent unless a release is found. */
export async function checkForUpdate(manual: boolean) {
  if (update.status === 'installing') return;
  if (manual) update.status = 'checking';
  try {
    const info = await api.checkUpdate();
    if (info) {
      update.info = info;
      update.status = 'available';
    } else {
      update.status = 'current';
      update.checked = new Date().toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
    }
  } catch (e) {
    update.reason = String(e);
    update.status = manual ? 'checkfailed' : 'idle';
  }
}

/** Snapshot first, then install; Rust refuses while a page holds unsaved input. On Windows the app exits when the installer starts. */
export async function installUpdate() {
  if (update.unsaved || update.status === 'installing') return;
  update.status = 'installing';
  try {
    await api.installUpdate();
  } catch (e) {
    update.reason = String(e);
    update.status = 'failed';
  }
}

export function later() {
  update.declined = true;
}
