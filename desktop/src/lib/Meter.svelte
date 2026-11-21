<script lang="ts">
  // A 4px progress bar for one macro against its goal. The crate says what was eaten, the goal and the words
  // (`said`); the only arithmetic here is turning eaten-over-goal into a bar width, as a chart maps a value to pixels.
  // Over a goal the bar is full with a --warn cap; the words carry the meaning. Nothing animates.
  let { value, goal, label, tone, said, over }: {
    /** grams eaten, or null when a food didn't report it */
    value: number | null;
    goal: number;
    label: string;
    tone: 'protein' | 'carbs' | 'fat';
    said: string;
    over: boolean;
  } = $props();

  const width = $derived(value == null || goal <= 0 ? 0 : Math.min(100, (value / goal) * 100));
</script>

<div
  class="meter"
  role="meter"
  aria-label={label}
  aria-valuemin="0"
  aria-valuemax={goal}
  aria-valuenow={value ?? undefined}
  aria-valuetext={said}
>
  <span class="fill {tone}" style:width="{over ? 100 : width}%"></span>
  {#if over}<span class="cap"></span>{/if}
</div>

<style>
  .meter { position: relative; width: 100%; height: 4px; border-radius: 2px; background: var(--well); overflow: hidden; }
  .fill { position: absolute; inset: 0 auto 0 0; border-radius: 2px; }
  .fill.protein { background: var(--protein); }
  .fill.carbs { background: var(--carbs); }
  .fill.fat { background: var(--fat); }
  .cap { position: absolute; inset: 0 0 0 auto; width: 4px; background: var(--warn); }
</style>
