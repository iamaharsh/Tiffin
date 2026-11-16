<script lang="ts">
  import { slide } from 'svelte/transition';
  import { prefersReducedMotion } from 'svelte/motion';
  import Icon from './Icon.svelte';
  import Dialog from './Dialog.svelte';
  import { update, installUpdate, later } from './updates.svelte';

  const want = $derived((update.status === 'available' && !update.declined) || update.status === 'installing' || update.status === 'failed');
  const ms = $derived(prefersReducedMotion.current ? 0 : 160);

  // A banner that appears while someone is typing waits until focus leaves the field (no layout shift under the cursor).
  let shown = $state(false);
  $effect(() => {
    if (!want) {
      shown = false;
      return;
    }
    const el = document.activeElement;
    if (!el?.matches('input, textarea, [contenteditable]')) {
      shown = true;
      return;
    }
    const show = () => (shown = true);
    document.addEventListener('focusout', show, { once: true });
    return () => document.removeEventListener('focusout', show);
  });

  function decline() {
    later();
    const h = document.querySelector<HTMLElement>('main h1');
    if (h) {
      h.tabIndex = -1;
      h.focus();
    }
  }
</script>

<div class="live" role="status">
  {#if shown}
    <div class="banner" class:failed={update.status === 'failed'} role={update.status === 'failed' ? 'alert' : undefined} transition:slide={{ duration: ms }}>
      {#if update.status === 'installing'}
        <span class="icon spin"><Icon name="refresh" size={16} /></span>
        <span class="text">Backing up your library, then installing Tiffin {update.info?.version}. Tiffin restarts when it’s done.</span>
      {:else if update.status === 'failed'}
        <span class="icon"><Icon name="warn" size={16} /></span>
        <span class="text">Update couldn’t install: {update.reason}.</span>
        <span class="buttons"><button class="small primary" onclick={installUpdate}>Try again</button></span>
      {:else}
        <span class="icon"><Icon name="refresh" size={16} /></span>
        <span class="text">Tiffin <b>{update.info?.short}</b> is ready.{update.unsaved ? ' Finish your edit to install.' : ''}</span>
        <span class="buttons">
          <button class="small ghost" onclick={() => (update.whatsNew = true)}>What’s new</button>
          <button class="small primary" aria-disabled={update.unsaved} onclick={installUpdate}>Install and restart</button>
          <button class="small ghost later" onclick={decline}>Later</button>
        </span>
      {/if}
    </div>
  {/if}
</div>

{#if update.whatsNew && update.info}
  <Dialog title="What’s new in Tiffin {update.info.version}" wide onclose={() => (update.whatsNew = false)}>
    {#if update.info.notes.trim()}
      <div class="notes">{update.info.notes}</div>
    {:else}
      <p class="fine">No release notes were published for this version.</p>
    {/if}
    {#snippet actions()}
      <button class="btn" onclick={() => (update.whatsNew = false)}>Close</button>
      <button class="btn primary" aria-disabled={update.unsaved} onclick={() => { if (!update.unsaved) { update.whatsNew = false; installUpdate(); } }}>Install and restart</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .banner {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    min-height: 48px;
    padding: 8px 16px;
    background: var(--accent-wash);
    border-bottom: 1px solid var(--rule-strong);
    color: var(--ink);
    font-size: 14.5px;
  }
  .banner.failed { background: var(--warn-wash); color: var(--warn); }
  .icon { display: inline-flex; color: var(--accent-text); }
  .failed .icon { color: var(--warn); }
  .text { flex: 1 1 240px; min-width: 0; overflow-wrap: anywhere; }
  .buttons { display: inline-flex; flex-wrap: wrap; gap: 8px; }
  .small { height: 28px; padding: 0 12px; border-radius: 8px; font-size: 13px; font-weight: 500; }
  .small.ghost { background: transparent; color: var(--accent-text); }
  .small.ghost.later { color: var(--ink-2); }
  .small.ghost:hover { background: var(--rule); }
  .small.primary { background: var(--accent); color: #fff; }
  .small.primary:hover { background: var(--accent-press); }
  .failed .small.primary { background: var(--warn); color: var(--warn-wash); }
  [aria-disabled='true'] { opacity: 0.5; cursor: default; }
  @media (prefers-reduced-motion: no-preference) {
    .spin { animation: turn 1s linear infinite; }
  }
  @keyframes turn { to { transform: rotate(360deg); } }
  /* release notes are plain text from the release page, never HTML */
  .notes { white-space: pre-wrap; max-height: 50vh; overflow-y: auto; overflow-wrap: anywhere; }
</style>
