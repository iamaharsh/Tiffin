<script lang="ts">
  import { slide } from 'svelte/transition';
  import { prefersReducedMotion } from 'svelte/motion';
  import Icon from './Icon.svelte';
  import * as api from './api';

  let { hidden = false, onerror, onnote, onopenbackup }: {
    /** hide while already viewing Settings > Backup & Restore to avoid duplicating its card */
    hidden?: boolean;
    onerror: (m: string) => void;
    onnote: (m: string) => void;
    onopenbackup: () => void;
  } = $props();

  const DISMISS_KEY = 'tiffin-backup-nudge-dismissed';
  const DISMISS_MS = 7 * 24 * 60 * 60 * 1000;
  const STALE_MS = 14 * 24 * 60 * 60 * 1000;

  let persisted: boolean | null = $state(null);
  let lastBackupAt: number | null = $state(null);
  let dismissed = $state(false);
  let checked = $state(false);
  let backingUp = $state(false);
  let shown = $state(false);
  let bannerEl: HTMLElement | undefined = $state();

  function recentlyDismissed(): boolean {
    try {
      const raw = localStorage.getItem(DISMISS_KEY);
      if (!raw) return false;
      const t = Number(raw);
      const age = Date.now() - t;
      return Number.isFinite(t) && age >= 0 && age < DISMISS_MS;
    } catch {
      return false;
    }
  }

  function rememberDismiss() {
    try {
      localStorage.setItem(DISMISS_KEY, String(Date.now()));
    } catch {
      /* private mode */
    }
  }

  function isEditable(el: EventTarget | null): boolean {
    return el instanceof HTMLElement && (el.matches('input, textarea, select') || el.isContentEditable);
  }

  function restoreFocus() {
    const target = document.querySelector<HTMLElement>('main h1, main h2') ?? document.querySelector<HTMLElement>('main');
    if (target) {
      target.tabIndex = -1;
      target.focus();
    }
  }

  export async function refreshStatus() {
    const st = await api.storageStatus();
    persisted = st.persisted;
    lastBackupAt = st.lastBackupAt;
    dismissed = recentlyDismissed();
    checked = true;
  }

  $effect(() => {
    if (!api.isWeb) return;
    refreshStatus();
    const onChange = () => {
      refreshStatus();
    };
    window.addEventListener('tiffin-storage-change', onChange);
    return () => window.removeEventListener('tiffin-storage-change', onChange);
  });

  // A fresh download satisfies the nudge immediately; unpersisted browsers remind again after 7 days, persistent after 14.
  const staleMs = $derived(persisted === false ? DISMISS_MS : STALE_MS);
  const needsBackup = $derived(
    checked && !hidden && !dismissed && (lastBackupAt === null || Date.now() - lastBackupAt >= staleMs),
  );
  const ms = $derived(prefersReducedMotion.current ? 0 : 160);

  // Never shift layout while the user is typing in a field; wait until focus leaves editable controls.
  $effect(() => {
    if (!needsBackup) {
      shown = false;
      return;
    }
    if (!isEditable(document.activeElement)) {
      shown = true;
      return;
    }
    const onFocusOut = (e: FocusEvent) => {
      if (!isEditable(e.relatedTarget)) {
        shown = true;
        document.removeEventListener('focusout', onFocusOut);
      }
    };
    document.addEventListener('focusout', onFocusOut);
    return () => document.removeEventListener('focusout', onFocusOut);
  });

  async function backupNow() {
    if (backingUp) return;
    const hadFocus = bannerEl?.contains(document.activeElement) ?? false;
    backingUp = true;
    try {
      const msg = await api.backupNow();
      onnote(msg);
      await refreshStatus();
      if (hadFocus && !needsBackup) restoreFocus();
    } catch (e) {
      onerror(String(e));
    } finally {
      backingUp = false;
    }
  }

  function later() {
    dismissed = true;
    rememberDismiss();
    restoreFocus();
  }
</script>

<div class="live" role="status">
  {#if shown}
    <div class="banner" bind:this={bannerEl} transition:slide={{ duration: ms }}>
      <span class="icon"><Icon name="tray" size={16} /></span>
      <span class="text">
        {#if persisted === false}
          Browser storage isn’t persistent — download a backup to keep a safe copy.
        {:else if lastBackupAt === null}
          Your data lives only in this browser — download a backup for safekeeping.
        {:else}
          Over two weeks since your last backup — download a fresh copy.
        {/if}
      </span>
      <span class="buttons">
        <button type="button" class="small primary" aria-busy={backingUp} aria-disabled={backingUp} onclick={backupNow}>{backingUp ? 'Backing Up…' : 'Back Up Now'}</button>
        <button type="button" class="small ghost" onclick={onopenbackup}>Backup Settings</button>
        <button type="button" class="small ghost later" onclick={later}>Later</button>
      </span>
    </div>
  {/if}
</div>

<style>
  .banner {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    min-height: 44px;
    padding: 8px 16px;
    background: var(--well);
    border-bottom: 1px solid var(--rule);
    color: var(--ink);
    font-size: 13.5px;
  }
  .icon { display: inline-flex; flex: none; color: var(--accent-text); }
  .text { flex: 1 1 260px; min-width: 0; color: var(--ink-2); overflow-wrap: anywhere; }
  .buttons { display: inline-flex; flex-wrap: wrap; gap: 6px; }
  .small { height: 28px; padding: 0 12px; border-radius: 8px; font-size: 13px; font-weight: 500; }
  .small.ghost { background: transparent; color: var(--accent-text); }
  .small.ghost.later { color: var(--ink-2); }
  .small.ghost:hover { background: var(--rule); }
  .small.primary { background: var(--accent); color: #fff; }
  .small.primary:hover:not([aria-disabled='true']) { background: var(--accent-press); }
  [aria-disabled='true'] { opacity: 0.5; cursor: default; }
</style>
