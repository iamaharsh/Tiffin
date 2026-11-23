<script lang="ts" module>
  export type MenuItem = { label: string; icon?: string; run: () => void; danger?: boolean; disabled?: boolean } | 'sep';
</script>

<script lang="ts">
  import Icon from './Icon.svelte';

  // A context menu at a point (right-click) or under a button. Arrow keys move, Enter picks,
  // Esc or a click elsewhere closes; focus returns to where it was.
  let { x, y, items, onclose }: { x: number; y: number; items: MenuItem[]; onclose: () => void } = $props();

  let el: HTMLDivElement;
  let pos = $state({ left: 0, top: 0 });
  const opener = document.activeElement as HTMLElement | null;

  $effect(() => {
    // keep it inside the window
    const r = el.getBoundingClientRect();
    pos = { left: Math.min(x, innerWidth - r.width - 8), top: y + r.height > innerHeight - 8 ? Math.max(8, y - r.height) : y };
    buttons()[0]?.focus();
    return () => opener?.focus();
  });

  const buttons = () => [...el.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];
  function keys(e: KeyboardEvent) {
    const b = buttons();
    const i = b.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === 'ArrowDown') b[(i + 1) % b.length]?.focus();
    else if (e.key === 'ArrowUp') b[(i - 1 + b.length) % b.length]?.focus();
    else if (e.key === 'Escape' || e.key === 'Tab') onclose();
    else return;
    e.preventDefault();
    e.stopPropagation();
  }
  function pick(it: Exclude<MenuItem, 'sep'>) {
    onclose();
    it.run();
  }
</script>

<svelte:window onpointerdown={(e) => { if (!el.contains(e.target as Node)) onclose(); }} onblur={onclose} onresize={onclose} />

<div class="menu" role="menu" tabindex="-1" bind:this={el} style:left="{pos.left}px" style:top="{pos.top}px" onkeydown={keys}>
  {#each items as it}
    {#if it === 'sep'}
      <hr />
    {:else}
      <button role="menuitem" class:danger={it.danger} disabled={it.disabled} onclick={() => pick(it)}>
        <span class="ic">{#if it.icon}<Icon name={it.icon} size={15} />{/if}</span>{it.label}
      </button>
    {/if}
  {/each}
</div>

<style>
  .menu {
    position: fixed;
    z-index: 50;
    min-width: 210px;
    padding: 5px;
    border-radius: 11px;
    background: color-mix(in srgb, var(--card) 88%, transparent);
    backdrop-filter: blur(24px) saturate(180%);
    box-shadow: var(--shadow-float), inset 0 0 0 1px var(--rule);
    transform-origin: top left;
    animation: menu-in 140ms var(--ease-out);
    outline: none;
  }
  @keyframes menu-in { from { opacity: 0; transform: scale(0.96); } }
  button {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 28px;
    padding: 0 10px 0 6px;
    border-radius: 6px;
    font-size: 13.5px;
    text-align: left;
    outline: none;
  }
  button:hover:not(:disabled), button:focus-visible { background: var(--accent); color: #fff; }
  button:disabled { color: var(--ink-3); }
  .danger { color: var(--heart); }
  .danger:hover, .danger:focus-visible { background: var(--heart) !important; color: var(--on-heart) !important; }
  .ic { display: grid; place-items: center; width: 18px; color: inherit; opacity: 0.85; }
  hr { margin: 4px 6px; border: 0; border-top: 1px solid var(--rule); }
</style>
