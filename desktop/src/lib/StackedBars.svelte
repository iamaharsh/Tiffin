<script lang="ts">
  import { scaleLinear } from 'd3-scale';
  import { dayLabel, type EnergyBar, type EnergyRow } from './api';

  // Daily calories as bars stacked by macro (protein, carbs, fat), with each day's own target as a short dashed mark (weekday
  // templates can change it). It only maps the crate's numbers to pixels; a day whose split is unknown is one neutral block.
  // Days with nothing logged draw nothing. Nothing animates: a new range swaps the marks in place.
  let { bars, xMin, xMax, yMax, label, emptyText, table, asTable }: {
    bars: EnergyBar[];
    xMin: number;
    xMax: number;
    yMax: number;
    /** the aria-label summary */
    label: string;
    emptyText: string | null;
    /** one row per logged day, newest first, for Show as Table and the tooltip */
    table: EnergyRow[];
    asTable: boolean;
  } = $props();

  const LEFT = 56, RIGHT = 16, TOP = 16, BOTTOM = 32;
  let wrapWidth = $state(0);
  const width = $derived(wrapWidth || 640); // 0 before the first measure and in a server render
  const height = $derived(width < 640 ? 224 : 280);

  // each day owns a slot one day wide, centred on its day number
  const x = $derived(scaleLinear().domain([xMin - 0.5, xMax + 0.5]).range([LEFT, width - RIGHT]));
  const y = $derived(scaleLinear().domain([0, yMax]).range([height - BOTTOM, TOP]));
  const slot = $derived((width - LEFT - RIGHT) / Math.max(1, xMax - xMin + 1));
  const barW = $derived(Math.max(1, Math.min(28, slot * 0.6)));
  const yTicks = $derived(y.ticks(4));
  const xTicks = $derived([...new Set(scaleLinear().domain([xMin, xMax]).ticks(5).filter(Number.isInteger))]);

  const logged = $derived(bars.filter((b) => b.logged));
  /** the table is newest first, the bars oldest first */
  const rowOf = (k: number) => table[logged.length - 1 - k];
  const macroSum = (b: EnergyBar) => (b.protein_kcal ?? 0) + (b.carbs_kcal ?? 0) + (b.fat_kcal ?? 0);
  const split = (b: EnergyBar) => b.protein_kcal != null && b.carbs_kcal != null && b.fat_kcal != null && macroSum(b) > 0;
  // the bar is as tall as the calories eaten; the macros (4/4/9 kcal per g) only set the shares, since labels rarely add up exactly
  // shortcut: alcohol's calories are spread over the three shares, until the crate reports alcohol as its own part
  const scale = (b: EnergyBar) => (b.kcal != null ? b.kcal / macroSum(b) : 1);

  let active: number | null = $state(null); // index into logged
  let keyboard = $state(false); // the status line speaks only for the arrow keys, never for every mouse move
  const at = $derived(active != null ? logged[active] : null);
  const row = $derived(active != null ? rowOf(active) : null);
  const said = $derived(row ? `${row.date_text}, ${row.kcal_text}, ${row.macros_text}, target ${row.target_text}` : '');
  const side = $derived(at != null && x(at.day) > width / 2 ? 'left' : 'right');
  const top = (b: EnergyBar) => b.kcal ?? (split(b) ? macroSum(b) : 0);

  function nearest(day: number) {
    let best = 0;
    logged.forEach((b, i) => { if (Math.abs(b.day - day) < Math.abs(logged[best].day - day)) best = i; });
    return best;
  }
  function move(e: PointerEvent) {
    keyboard = false; // the mouse took over: the status line goes quiet until an arrow key
    if (!logged.length) return;
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    active = nearest(x.invert(e.clientX - r.left + LEFT));
  }
  function keys(e: KeyboardEvent) {
    if (!logged.length) return;
    const last = logged.length - 1;
    const next = e.key === 'ArrowLeft' ? Math.max(0, (active ?? last) - 1) : e.key === 'ArrowRight' ? Math.min(last, (active ?? last) + 1) : e.key === 'Home' ? 0 : e.key === 'End' ? last : null;
    if (next == null) return;
    e.preventDefault();
    keyboard = true; // arrow keys after a click still speak
    active = next;
  }
</script>

{#if asTable}
  <table class="grid">
    <caption class="sr">{label}</caption>
    <thead>
      <tr><th scope="col" class="caps">Date</th><th scope="col" class="caps">Calories</th><th scope="col" class="caps">From Macros</th><th scope="col" class="caps">Target</th></tr>
    </thead>
    <tbody>
      {#each table as r}
        <tr><th scope="row">{r.date_text}</th><td class="num">{r.kcal_text}</td><td>{r.macros_text}</td><td class="num">{r.target_text}</td></tr>
      {/each}
    </tbody>
  </table>
  {#if !table.length && emptyText}<p class="fine">{emptyText}</p>{/if}
{:else if !logged.length}
  <p class="fine">{emptyText ?? ''}</p>
{:else}
  <ul class="legend fine" aria-hidden="true">
    <li><i class="key protein"></i>Protein</li>
    <li><i class="key carbs"></i>Carbs</li>
    <li><i class="key fat"></i>Fat</li>
    <li><i class="key target"></i>Target</li>
  </ul>
  <div class="wrap" bind:clientWidth={wrapWidth} style:height="{height}px">
    <svg {width} {height} viewBox="0 0 {width} {height}" role="img" aria-label={label}>
      {#each yTicks as t (t)}
        <line class="grid-line" x1={LEFT} x2={width - RIGHT} y1={y(t)} y2={y(t)} />
        <text class="tick num" x={LEFT - 8} y={y(t)} text-anchor="end" dominant-baseline="middle">{t.toLocaleString('en-US')}</text>
      {/each}
      {#each xTicks as t (t)}
        <text class="tick num" x={x(t)} y={height - BOTTOM + 16} text-anchor="middle" dominant-baseline="middle">{dayLabel(t, { month: 'short', day: 'numeric' })}</text>
      {/each}
      {#each logged as b, i (b.day)}
        {@const cx = x(b.day)}
        <g class:dim={active != null && active !== i}>
          {#if split(b)}
            {@const k = scale(b)}
            {@const p = (b.protein_kcal ?? 0) * k}
            {@const c = (b.carbs_kcal ?? 0) * k}
            {@const f = (b.fat_kcal ?? 0) * k}
            <rect class="protein" x={cx - barW / 2} width={barW} y={y(p)} height={y(0) - y(p)} />
            <rect class="carbs" x={cx - barW / 2} width={barW} y={y(p + c)} height={y(p) - y(p + c)} />
            <rect class="fat" x={cx - barW / 2} width={barW} y={y(p + c + f)} height={y(p + c) - y(p + c + f)} />
          {:else}
            <rect class="unknown" x={cx - barW / 2} width={barW} y={y(top(b))} height={y(0) - y(top(b))} />
          {/if}
        </g>
      {/each}
      {#each bars as b (b.day)}
        {#if b.target != null}
          <line class="target" x1={x(b.day) - Math.max(barW, 4) / 2 - 2} x2={x(b.day) + Math.max(barW, 4) / 2 + 2} y1={y(b.target)} y2={y(b.target)} />
        {/if}
      {/each}
    </svg>
    <!-- the pointer and the arrow keys pick the nearest logged day; the table behind Show as Table is the screen-reader path to every one -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div
      class="hit"
      role="application"
      tabindex="0"
      aria-label="Calorie bars. Left and right arrow keys move between logged days."
      style:left="{LEFT}px"
      style:right="{RIGHT}px"
      style:top="{TOP}px"
      style:bottom="{BOTTOM}px"
      onpointermove={move}
      onpointerleave={() => { if (!keyboard) active = null; }}
      onfocus={(e) => { keyboard = e.currentTarget.matches(':focus-visible'); active ??= logged.length - 1; }}
      onblur={() => { keyboard = false; active = null; }}
      onkeydown={keys}
    ></div>
    <span class="sr" role="status">{keyboard ? said : ''}</span>
    {#if at && row}
      {@const py = Math.min(Math.max(y(top(at)), 56), height - BOTTOM - 56)}
      <div class="tip fine" style:top="{py}px" style:left={side === 'right' ? `${x(at.day) + 16}px` : undefined} style:right={side === 'left' ? `${width - x(at.day) + 16}px` : undefined}>
        <span>{row.date_text}</span>
        <span class="num">{row.kcal_text}</span>
        <span>{row.macros_text}</span>
        <span class="num">Target {row.target_text}</span>
      </div>
    {/if}
  </div>
{/if}

<style>
  .fine { margin: 0; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .legend { display: flex; flex-wrap: wrap; gap: 4px 16px; margin: 0 0 8px; padding: 0; list-style: none; }
  .legend li { display: flex; align-items: center; gap: 8px; }
  .key { display: inline-block; width: 10px; height: 10px; border-radius: 3px; }
  .key.protein { background: var(--protein); }
  .key.carbs { background: var(--carbs); }
  .key.fat { background: var(--fat); }
  .key.target { width: 14px; height: 0; border-radius: 0; border-top: 2px dashed var(--ink-2); }
  .wrap { position: relative; width: 100%; }
  svg { display: block; }
  .grid-line { stroke: var(--rule); stroke-width: 1; }
  .tick { fill: var(--ink-3); font-size: 13px; }
  .protein { fill: var(--protein); }
  .carbs { fill: var(--carbs); }
  .fat { fill: var(--fat); }
  .unknown { fill: var(--rule-strong); }
  .dim { opacity: 0.45; }
  .target { stroke: var(--ink-2); stroke-width: 2; stroke-dasharray: 3 2; }
  .hit { position: absolute; border-radius: 4px; touch-action: pan-y; } /* a vertical swipe scrolls the page; a sideways one picks a day */
  .tip {
    position: absolute;
    display: flex;
    flex-direction: column;
    padding: 8px 16px;
    border-radius: 9px;
    background: var(--card);
    color: var(--ink);
    box-shadow: var(--shadow-float);
    max-width: min(320px, 70%);
    transform: translateY(-50%);
    pointer-events: none;
  }
  .grid { width: 100%; border-collapse: collapse; }
  .grid th, .grid td { padding: 8px 16px; text-align: right; border-bottom: 1px solid var(--rule); }
  .grid th:first-child { text-align: left; padding-left: 0; font-weight: 600; }
  .grid thead th { font-size: 13px; color: var(--ink-2); }
</style>
