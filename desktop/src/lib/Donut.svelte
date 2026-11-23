<script lang="ts">
  import type { Slice } from './api';

  // A ring split by macro, from the crate's shares. It only maps percentages to stroke lengths; the sentence for screen
  // readers and every number in the legend are the crate's. Nothing animates.
  let { slices, label, center, centerUnit }: {
    slices: Slice[];
    /** the aria-label summary */
    label: string;
    /** the big number in the middle, e.g. "2,000", and the line under it */
    center: string | null;
    centerUnit: string;
  } = $props();

  const R = 52, STROKE = 16, SIZE = 2 * R + STROKE;
  const C = 2 * Math.PI * R;
  // each slice starts where the last ended; a hairline gap keeps neighbours apart, but a ring of one slice is whole
  const arcs = $derived.by(() => {
    let at = 0;
    const gap = slices.filter((s) => s.pct > 0).length > 1 ? 2 : 0;
    return slices.map((s) => {
      const len = (s.pct / 100) * C;
      const arc = { tone: s.tone, dash: `${Math.max(0, len - gap)} ${C}`, offset: -at };
      at += len;
      return arc;
    });
  });
</script>

<div class="donut">
  <svg width={SIZE} height={SIZE} viewBox="0 0 {SIZE} {SIZE}" role="img" aria-label={label}>
    <circle class="track" cx={SIZE / 2} cy={SIZE / 2} r={R} stroke-width={STROKE} fill="none" />
    <g transform="rotate(-90 {SIZE / 2} {SIZE / 2})">
      {#each arcs as a, i (i)}
        <circle class={a.tone} cx={SIZE / 2} cy={SIZE / 2} r={R} stroke-width={STROKE} fill="none" stroke-dasharray={a.dash} stroke-dashoffset={a.offset} />
      {/each}
    </g>
    {#if center}
      <text class="mid num" x={SIZE / 2} y={SIZE / 2 - 4} text-anchor="middle" dominant-baseline="middle">{center}</text>
      <text class="sub" x={SIZE / 2} y={SIZE / 2 + 16} text-anchor="middle" dominant-baseline="middle">{centerUnit}</text>
    {/if}
  </svg>
  <ul class="legend" aria-hidden="true">
    {#each slices as s (s.name)}
      <li><i class="key {s.tone}"></i>{s.text}</li>
    {/each}
  </ul>
</div>

<style>
  .donut { display: flex; flex-wrap: wrap; align-items: center; gap: 16px 32px; }
  svg { display: block; flex: none; }
  .track { stroke: var(--well); }
  .protein { stroke: var(--protein); }
  .carbs { stroke: var(--carbs); }
  .fat { stroke: var(--fat); }
  .mid { fill: var(--ink); font: 700 20px var(--font-display); }
  .sub { fill: var(--ink-2); font-size: 12px; }
  .legend { display: flex; flex-direction: column; gap: 8px; margin: 0; padding: 0; list-style: none; font-size: 14px; }
  .legend li { display: flex; align-items: center; gap: 8px; }
  .key { display: inline-block; width: 10px; height: 10px; border-radius: 3px; flex: none; }
  .key.protein { background: var(--protein); }
  .key.carbs { background: var(--carbs); }
  .key.fat { background: var(--fat); }
</style>
