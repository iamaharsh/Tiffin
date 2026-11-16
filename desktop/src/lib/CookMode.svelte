<script lang="ts">
  import { fly, fade } from 'svelte/transition';
  import { prefersReducedMotion } from 'svelte/motion';
  import Icon from './Icon.svelte';
  import * as api from './api';
  import type { Detail } from './api';
  import { startTimer } from './timers.svelte';

  let { d, onclose, onerror }: { d: Detail; onclose: () => void; onerror: (m: string) => void } = $props();

  let index = $state(0);
  let dir = $state(1); // which way the step text slides
  let showIngredients = $state(false);
  let started: string[] = $state([]); // time phrases already started, so the button says so
  const n = $derived(d.steps.length);
  const last = $derived(index >= n - 1);
  const c = $derived(d.card);

  function go(i: number) {
    if (i < 0 || i >= n || i === index) return;
    dir = i > index ? 1 : -1;
    index = i;
  }
  const next = () => (last ? onclose() : go(index + 1));

  function timer(text: string, seconds: number) {
    startTimer(`${text} · step ${index + 1}`, seconds);
    started = [...started, `${index}:${text}`];
  }

  function keys(e: KeyboardEvent) {
    if (e.key === 'ArrowRight' || e.key === ' ' || e.key === 'PageDown') {
      e.preventDefault();
      if (!last) go(index + 1);
    } else if (e.key === 'ArrowLeft' || e.key === 'PageUp') {
      e.preventDefault();
      go(index - 1);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopImmediatePropagation();
      onclose();
    } else if (e.key.toLowerCase() === 'i') {
      showIngredients = !showIngredients;
    }
  }

  // Take keyboard focus like a dialog, and hand it back to whatever opened cook mode.
  let doneBtn: HTMLButtonElement;
  const opener = document.activeElement as HTMLElement | null;
  $effect(() => {
    doneBtn.focus();
    return () => opener?.focus();
  });

  // Keep the screen on while cooking (hands are busy); released when cook mode closes.
  $effect(() => {
    let lock: WakeLockSentinel | null = null;
    navigator.wakeLock?.request('screen').then((l) => (lock = l)).catch(() => {});
    return () => { lock?.release(); };
  });

  const slide = (node: Element, { out }: { out: boolean }) =>
    prefersReducedMotion.current ? fade(node, { duration: 120 }) : fly(node, { x: (out ? -dir : dir) * 48, duration: out ? 160 : 300, opacity: 0, easing: (x) => 1 - Math.pow(1 - x, 3) });
</script>

<svelte:window onkeydown={keys} />

<div class="cook label-scope" style:--l={api.labelVar(c.label_index)} role="dialog" aria-modal="true" aria-label="Cooking {c.name}" transition:fade={{ duration: 200 }}>
  <header>
    <button class="done" bind:this={doneBtn} onclick={onclose}><Icon name="close" size={16} /> Done</button>
    <h1>{c.name}</h1>
    {#if d.video}
      <button class="tool" onclick={() => api.openLink(d.video!).catch((e) => onerror(String(e)))}><Icon name="play" size={15} /> Video</button>
    {/if}
    <button class="tool" class:on={showIngredients} aria-pressed={showIngredients} onclick={() => (showIngredients = !showIngredients)} title="Ingredients (I)">
      <Icon name="sidebar" size={16} /> Ingredients
    </button>
  </header>

  <div class="body">
    <main>
      {#if n === 0}
        <p class="none">This recipe has no steps yet. Add directions on its edit page.</p>
      {:else}
        <nav class="progress" aria-label="Steps">
          {#each d.steps as _, i}
            <button class="seg" class:past={i < index} class:now={i === index} aria-label="Step {i + 1}" aria-current={i === index ? 'step' : undefined} onclick={() => go(i)}></button>
          {/each}
        </nav>
        <p class="count caps" aria-live="polite">Step {index + 1} <span>of {n}</span></p>
        <div class="stage">
          {#key index}
            <p class="step selectable" in:slide={{ out: false }} out:slide={{ out: true }}>
              {#each d.steps[index] as p}
                {#if p.seconds}
                  {@const on = started.includes(`${index}:${p.text}`)}
                  <button class="time" class:on onclick={() => timer(p.text, p.seconds!)} title={on ? 'Timer started' : `Start a ${p.text} timer`}>
                    <Icon name={on ? 'check' : 'clock'} size={22} />{p.text}
                  </button>
                {:else}{p.text}{/if}
              {/each}
            </p>
          {/key}
        </div>
      {/if}
    </main>

    {#if showIngredients}
      <aside transition:fly={{ x: 320, duration: prefersReducedMotion.current ? 0 : 280, opacity: 1, easing: (x) => 1 - Math.pow(1 - x, 3) }}>
        <h2 class="caps">Ingredients</h2>
        <ul class="selectable">
          {#each d.rows as r}
            {#if r.heading}<li class="sub">{r.text}</li>
            {:else}<li><span class="amt num">{r.amount ?? ''}</span><span>{r.text}</span></li>{/if}
          {/each}
        </ul>
      </aside>
    {/if}
  </div>

  {#if n}
    <footer>
      <button class="nav" onclick={() => go(index - 1)} disabled={index === 0}><Icon name="back" size={20} /> Previous</button>
      <span class="keys">← → to move · I for ingredients · Esc to finish</span>
      <button class="nav primary" class:finish={last} onclick={next}>
        {#if last}<Icon name="check" size={20} /> Finish{:else}Next <span class="flip"><Icon name="back" size={20} /></span>{/if}
      </button>
    </footer>
  {/if}
</div>

<style>
  .cook {
    position: fixed;
    inset: 0;
    z-index: 30;
    display: flex;
    flex-direction: column;
    background: var(--page);
  }
  header { display: flex; align-items: center; gap: 10px; height: 60px; padding: 0 20px; border-bottom: 1px solid var(--rule); }
  h1 { flex: 1; min-width: 0; margin: 0 0 0 6px; font: 600 16px var(--font-text); color: var(--ink-2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .done, .tool {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    padding: 0 12px;
    border-radius: 9px;
    font-size: 13.5px;
    font-weight: 500;
    transition: background-color 120ms ease-out, color 120ms ease-out;
  }
  .done { background: var(--well); box-shadow: inset 0 0 0 1px var(--rule); }
  .done:hover, .tool:hover { background: var(--rule); }
  .tool { color: var(--ink-2); }
  .tool.on { color: var(--accent-text); background: var(--accent-wash); }

  .body { flex: 1; display: flex; min-height: 0; }
  main { flex: 1; min-width: 0; display: flex; flex-direction: column; align-items: center; padding: 40px 48px 0; overflow-y: auto; overscroll-behavior: contain; }
  .progress { display: flex; gap: 5px; width: min(640px, 100%); }
  .seg { flex: 1; height: 6px; border-radius: 3px; background: var(--rule-strong); transition: background-color 300ms var(--ease-out); position: relative; }
  .seg::before { content: ''; position: absolute; inset: -10px 0; } /* bigger target than the 6px bar */
  .seg.past { background: color-mix(in srgb, var(--l) 55%, var(--page)); }
  .seg.now { background: var(--l); }
  .seg:hover:not(.now) { background: color-mix(in srgb, var(--l) 35%, var(--rule-strong)); }
  .count { margin: 22px 0 0; font-size: 20px; color: var(--l-text); }
  .count span { color: var(--ink-3); }
  .stage { display: grid; width: min(820px, 100%); margin-top: 18px; padding-bottom: 32px; }
  .step {
    grid-area: 1 / 1; /* the old and new step share one cell while they cross */
    margin: 0;
    font: 500 clamp(24px, 3vw, 34px) / 1.4 var(--font-display);
    letter-spacing: -0.012em;
    text-wrap: pretty;
  }
  .time {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0 12px 0 9px;
    margin: 0 2px;
    border-radius: 12px;
    font: inherit;
    color: var(--accent-text);
    background: var(--accent-wash);
    vertical-align: baseline;
    transition: background-color 140ms ease-out, transform 100ms ease-out;
  }
  .time:hover { background: color-mix(in srgb, var(--accent) 22%, var(--page)); }
  .time:active { transform: scale(0.96); }
  .time.on { color: #fff; background: var(--accent); }
  .none { margin-top: 18vh; font-size: 18px; color: var(--ink-2); }

  aside { width: 320px; flex: none; overflow-y: auto; padding: 24px 22px; background: var(--sidebar); border-left: 1px solid var(--rule); }
  h2 { margin: 0 0 10px; font-size: 17px; color: var(--l-text); }
  ul { list-style: none; margin: 0; padding: 0; font-size: 15px; }
  li { display: flex; gap: 10px; padding: 6px 0; border-bottom: 1px solid var(--rule); }
  .amt { flex: none; min-width: 48px; text-align: right; font-size: 16.5px; color: var(--l-text); }
  .sub { padding-top: 14px; font-weight: 600; border: 0; }

  footer { display: flex; align-items: center; justify-content: space-between; gap: 16px; padding: 16px 28px 22px; }
  .keys { font-size: 12.5px; color: var(--ink-3); }
  .nav {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    min-width: 160px;
    height: 52px;
    padding: 0 22px;
    border-radius: 14px;
    font-size: 17px;
    font-weight: 600;
    background: var(--well);
    box-shadow: inset 0 0 0 1px var(--rule);
    transition: background-color 140ms ease-out, transform 100ms ease-out, opacity 140ms ease-out;
  }
  .nav:hover:not(:disabled) { background: var(--rule); }
  .nav:active:not(:disabled) { transform: scale(0.97); }
  .nav:disabled { opacity: 0.4; }
  .nav.primary { color: #fff; background: var(--accent); box-shadow: inset 0 -2px 0 rgba(0, 0, 0, 0.15); }
  .nav.primary:hover { background: var(--accent-press); }
  .nav.finish { background: var(--l); }
  .nav.finish:hover { background: color-mix(in srgb, var(--l) 85%, black); }
  .flip { display: inline-flex; transform: scaleX(-1); }

  @media (max-width: 860px) {
    aside { position: absolute; right: 0; top: 60px; bottom: 0; box-shadow: var(--shadow-float); }
    .keys { display: none; }
  }
</style>
