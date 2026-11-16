<script lang="ts">
  import { fly } from 'svelte/transition';
  import { flip } from 'svelte/animate';
  import Icon from './Icon.svelte';
  import { timers, toggleTimer, removeTimer, formatLeft } from './timers.svelte';

  const R = 13;
  const C = 2 * Math.PI * R;
</script>

{#if timers.length}
  <section class="tray" aria-label="Timers">
    {#each timers as t (t.id)}
      <div
        class="timer"
        class:done={t.done}
        class:paused={!t.endsAt && !t.done}
        role={t.done ? 'alert' : undefined}
        animate:flip={{ duration: 240 }}
        in:fly={{ y: 24, duration: 320, easing: (x) => 1 - Math.pow(1 - x, 3) }}
        out:fly={{ x: 40, duration: 200 }}
      >
        <button class="face" onclick={() => (t.done ? removeTimer(t) : toggleTimer(t))} aria-label={t.done ? `Dismiss ${t.label} timer` : t.endsAt ? `Pause ${t.label} timer` : `Resume ${t.label} timer`}>
          <svg width="32" height="32" viewBox="0 0 32 32" aria-hidden="true">
            <circle cx="16" cy="16" r={R} class="track" />
            <circle cx="16" cy="16" r={R} class="ring" stroke-dasharray={C} stroke-dashoffset={C * (1 - t.left / t.total)} />
          </svg>
          <span class="glyph">
            {#if t.done}<Icon name="check" size={14} />{:else if t.endsAt}<Icon name="pause" size={13} />{:else}<Icon name="play" size={13} />{/if}
          </span>
        </button>
        <div class="text">
          <span class="time num">{t.done ? 'Done' : formatLeft(t.left)}</span>
          <span class="label">{t.label}</span>
        </div>
        <button class="x" onclick={() => removeTimer(t)} aria-label="Remove {t.label} timer"><Icon name="close" size={13} /></button>
      </div>
    {/each}
  </section>
{/if}

<style>
  .tray {
    position: fixed;
    right: 20px;
    bottom: 20px;
    z-index: 40;
    display: flex;
    flex-direction: column;
    gap: 8px;
    align-items: flex-end;
    transition: bottom 300ms var(--ease-out);
  }
  /* cook mode's Next button lives in that corner */
  :global(body:has(.cook[role='dialog'])) .tray { bottom: 100px; }
  .timer {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 200px;
    padding: 7px 8px 7px 7px;
    border-radius: 14px;
    background: color-mix(in srgb, var(--card) 86%, transparent);
    backdrop-filter: blur(18px) saturate(170%);
    box-shadow: var(--shadow-float), inset 0 0 0 1px var(--rule);
  }
  .face { position: relative; display: grid; place-items: center; width: 32px; height: 32px; border-radius: 50%; color: var(--accent-text); }
  .face svg { position: absolute; inset: 0; transform: rotate(-90deg); }
  .track { fill: none; stroke: var(--rule-strong); stroke-width: 3; }
  .ring { fill: none; stroke: var(--accent); stroke-width: 3; stroke-linecap: round; transition: stroke-dashoffset 250ms linear; }
  .glyph { display: grid; place-items: center; }
  .face:hover { background: var(--accent-wash); }
  .face:active { transform: scale(0.92); }
  .text { display: flex; flex-direction: column; flex: 1; min-width: 0; line-height: 1.15; }
  .time { font-size: 20px; }
  .label { font-size: 12px; color: var(--ink-2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 160px; }
  .paused .time { color: var(--ink-3); }
  .x { display: grid; place-items: center; width: 24px; height: 24px; border-radius: 7px; color: var(--ink-3); }
  .x:hover { background: var(--rule); color: var(--ink); }

  .done { background: var(--accent); color: #fff; animation: nudge 1.6s var(--ease-out) infinite; }
  .done .label { color: rgba(255, 255, 255, 0.85); }
  .done .face { color: #fff; }
  .done .track { stroke: rgba(255, 255, 255, 0.35); }
  .done .ring { stroke: #fff; }
  .done .face:hover, .done .x:hover { background: rgba(255, 255, 255, 0.18); color: #fff; }
  .done .x { color: rgba(255, 255, 255, 0.8); }
  @keyframes nudge {
    0%, 60%, 100% { transform: none; }
    10% { transform: rotate(-2.5deg); }
    25% { transform: rotate(2.5deg); }
    40% { transform: rotate(-1deg); }
  }
  @media (prefers-reduced-motion: reduce) { .done { animation: none; } }
</style>
