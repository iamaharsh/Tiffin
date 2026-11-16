<script module lang="ts">
  // Key map of a radio group: arrows wrap, Home and End jump, anything else is not ours (null).
  // Exported so desktop/tools/segmented-check.mjs tests the code the control runs.
  export function step(key: string, i: number, n: number): number | null {
    if (key === 'ArrowRight' || key === 'ArrowDown') return (i + 1) % n;
    if (key === 'ArrowLeft' || key === 'ArrowUp') return (i - 1 + n) % n;
    if (key === 'Home') return 0;
    if (key === 'End') return n - 1;
    return null;
  }
  // The pill jumps (no spring) on first paint and under reduced motion.
  export function instant(ready: boolean, reduced: boolean): boolean {
    return !ready || reduced;
  }
</script>

<script lang="ts" generics="T extends string | number">
  import type { Snippet } from 'svelte';
  import { Spring, prefersReducedMotion } from 'svelte/motion';
  // A Mac-style segmented control: the selected pill slides to the new choice on a spring.
  let {
    options, value, onchange, label, item, title, status,
  }: { options: T[]; value: T; onchange: (v: T) => void; label: string; item: Snippet<[T]>; title?: string; status?: (v: T) => string } = $props();

  let root: HTMLDivElement | undefined = $state();
  let buttons: HTMLButtonElement[] = $state([]);
  const pill = new Spring({ x: 0, w: 0 }, { stiffness: 0.2, damping: 0.75 });
  let ready = $state(false);
  let said = $state(''); // spoken by the polite status, only after a change the user made

  $effect(() => {
    const b = buttons[options.indexOf(value)];
    if (!b) return;
    const measure = (instantly: boolean) => pill.set({ x: b.offsetLeft, w: b.offsetWidth }, { instant: instantly });
    measure(instant(ready, prefersReducedMotion.current));
    ready = true;
    if (!root) return;
    const watch = new ResizeObserver(() => measure(true)); // a resized window moves the segments under the pill
    watch.observe(root);
    return () => watch.disconnect();
  });

  function pick(o: T) {
    onchange(o);
    if (status) said = status(o);
  }

  function keys(e: KeyboardEvent) {
    const next = step(e.key, options.indexOf(value), options.length);
    if (next === null) return;
    e.preventDefault();
    pick(options[next]);
    buttons[next]?.focus();
  }
</script>

<div class="seg" bind:this={root} role="radiogroup" aria-label={label} {title} tabindex="-1" onkeydown={keys}>
  <span class="pill" class:ready style:transform="translateX({pill.current.x}px)" style:width="{pill.current.w}px"></span>
  {#each options as o, i}
    <button
      bind:this={buttons[i]}
      role="radio"
      aria-checked={o === value}
      tabindex={o === value ? 0 : -1}
      class:sel={o === value}
      onclick={() => pick(o)}><span class="face">{@render item(o)}</span><span class="sizer" aria-hidden="true">{@render item(o)}</span></button
    >
  {/each}
  {#if status}<span class="sr" role="status">{said}</span>{/if}
</div>

<style>
  .seg {
    position: relative;
    display: inline-flex;
    padding: 2px;
    border-radius: 9px;
    background: var(--well);
    box-shadow: inset 0 0 0 1px var(--rule-strong);
  }
  .pill {
    position: absolute;
    top: 2px;
    bottom: 2px;
    left: 0;
    border-radius: 7px;
    background: var(--pill);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.12), 0 0 0 0.5px var(--rule-strong);
    opacity: 0;
  }
  .pill.ready { opacity: 1; }
  button {
    position: relative;
    display: inline-grid;
    align-items: center;
    height: 26px;
    padding: 0 8px;
    border-radius: 7px;
    color: var(--ink-2);
    font-size: 13px;
    transition: color 120ms ease-out;
  }
  /* the label and an invisible bold copy share one cell, so a segment keeps its width when it turns bold */
  .face, .sizer { grid-area: 1 / 1; display: inline-flex; align-items: center; justify-content: center; gap: 4px; }
  .sizer { height: 0; overflow: hidden; visibility: hidden; font-weight: 600; }
  button:hover { color: var(--ink); }
  button.sel { color: var(--ink); font-weight: 600; }
  button:active { transform: scale(0.96); }
</style>
