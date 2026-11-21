<script lang="ts">
  import type { Snippet } from 'svelte';

  // A sheet over the window: the browser's own modal <dialog>, so focus stays inside, Esc
  // closes it and the page behind can't be clicked.
  /** `oncancel` is what Esc does when that differs from clicking outside (the Add Food dialog undoes typed text first) */
  let { title, onclose, oncancel, children, actions, wide = false, large = false }: { title: string; onclose: () => void; oncancel?: () => void; children: Snippet; actions: Snippet; wide?: boolean; large?: boolean } = $props();

  let el: HTMLDialogElement;
  $effect(() => {
    el.showModal();
    return () => el.close();
  });
</script>

<dialog
  bind:this={el}
  class:wide
  class:large
  aria-label={title}
  oncancel={(e) => { e.preventDefault(); (oncancel ?? onclose)(); }}
  onclick={(e) => { if (e.target === el) onclose(); }}
>
  <div class="sheet">
    <h2>{title}</h2>
    <div class="body">{@render children()}</div>
    <div class="actions">{@render actions()}</div>
  </div>
</dialog>

<style>
  dialog {
    width: min(440px, calc(100vw - 32px));
    max-height: calc(100vh - 48px);
    padding: 0;
    border: 0;
    border-radius: 16px;
    background: var(--card);
    color: var(--ink);
    box-shadow: var(--shadow-float), inset 0 0 0 1px var(--rule);
    overscroll-behavior: contain;
    animation: sheet-in 260ms var(--ease-out);
  }
  dialog.wide { width: min(560px, calc(100vw - 32px)); }
  dialog.large { width: min(760px, calc(100vw - 32px)); }
  dialog::backdrop { background: rgba(20, 14, 8, 0.32); animation: fade-in 200ms ease-out; }
  @keyframes sheet-in { from { opacity: 0; transform: translateY(12px) scale(0.98); } }
  @keyframes fade-in { from { opacity: 0; } }
  /* the sheet takes the dialog's own height limit, so the title and buttons stay put and only the body scrolls */
  .sheet { display: flex; flex-direction: column; gap: 14px; max-height: inherit; padding: 22px 24px 20px; }
  h2 { margin: 0; font: 700 19px/1.25 var(--font-display); letter-spacing: -0.01em; }
  .body { display: flex; flex-direction: column; gap: 12px; min-height: 0; overflow-y: auto; font-size: 14px; color: var(--ink-2); }
  .actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 6px; }

  /* fields used inside every sheet (the buttons are global, in app.css) */
  .sheet :global(input:not([type='checkbox'], [type='radio'])), .sheet :global(select) {
    width: 100%;
    height: 34px;
    padding: 0 10px;
    border: 0;
    border-radius: 9px;
    background: var(--well);
    box-shadow: inset 0 0 0 1px var(--rule);
    color: var(--ink);
    font: inherit;
    font-size: 14px;
    outline: none;
    -webkit-user-select: text;
    user-select: text;
  }
  .sheet :global(input:focus), .sheet :global(select:focus) { box-shadow: 0 0 0 3px var(--focus), inset 0 0 0 1px var(--accent); }
  .sheet :global(input[type='checkbox']), .sheet :global(input[type='radio']) { accent-color: var(--accent); width: 16px; height: 16px; margin: 0; }
  .sheet :global(.check) { display: flex; align-items: center; gap: 9px; color: var(--ink); }
  .sheet :global(.label) { display: flex; flex-direction: column; gap: 5px; font-size: 12.5px; }
  .sheet :global(.fine) { margin: 0; font-size: 12.5px; color: var(--ink-3); }
</style>
