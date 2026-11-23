<script lang="ts">
  import { scaleLinear } from 'd3-scale';
  import { dayLabel } from './api';

  // A line chart that only maps the crate's numbers to pixels: x is a day number (days since 1970-01-01), y is a value in the
  // person's unit. The trend, the rate and the sentences are the crate's; nothing here computes them. Dates are printed from day
  // numbers, never parsed from strings. Nothing animates: a new range swaps the marks in place.
  type Point = { day: number; raw: number | null; trend: number | null };
  type Row = { date_text: string; raw_text: string; trend_text: string };
  let { points, xMin, xMax, yMin, yMax, goal, goalText, label, unitText, emptyText, table, asTable, rawHeading = 'Weight', trendHeading = 'Trend' }: {
    points: Point[];
    xMin: number;
    xMax: number;
    yMin: number;
    yMax: number;
    goal: number | null;
    goalText: string | null;
    /** the aria-label summary */
    label: string;
    unitText: string;
    emptyText: string | null;
    /** one row per point, in the same order, for Show as Table and the tooltip */
    table: Row[];
    asTable: boolean;
    /** the table's column names; a null raw heading drops that column and its tooltip line (a chart of trend values only) */
    rawHeading?: string | null;
    trendHeading?: string;
  } = $props();

  const LEFT = 48, RIGHT = 16, TOP = 16, BOTTOM = 32;
  let wrapWidth = $state(0);
  const width = $derived(wrapWidth || 640); // 0 before the first measure and in a server render
  const height = $derived(width < 640 ? 224 : 280);

  const x = $derived(scaleLinear().domain([xMin, xMax]).range([LEFT, width - RIGHT]));
  const y = $derived(scaleLinear().domain([yMin, yMax]).range([height - BOTTOM, TOP]));
  const yTicks = $derived(y.ticks(4));
  // whole days only: a short range would otherwise ask for half a day
  const xTicks = $derived([...new Set(x.ticks(5).filter(Number.isInteger))]);

  const dots = $derived(points.filter((p) => p.raw != null));
  const trendPoints = $derived(points.filter((p) => p.trend != null));
  // the trend joins the points that exist, in order; a day without a weigh-in is bridged, never given a value
  const trendPath = $derived(trendPoints.length > 1 ? trendPoints.map((p, i) => `${i ? 'L' : 'M'}${x(p.day)},${y(p.trend as number)}`).join(' ') : '');

  let active: number | null = $state(null); // index into points
  let keyboard = $state(false); // the status line speaks only for the arrow keys, never for every mouse move
  const at = $derived(active != null ? points[active] : null);
  const said = $derived(active != null && table[active] ? [table[active].date_text, rawHeading == null ? null : table[active].raw_text, `${trendHeading.toLowerCase()} ${table[active].trend_text}`].filter((s) => s != null).join(', ') : '');
  const side = $derived(at != null && x(at.day) > width / 2 ? 'left' : 'right');

  function nearest(day: number) {
    let best = 0;
    points.forEach((p, i) => { if (Math.abs(p.day - day) < Math.abs(points[best].day - day)) best = i; });
    return best;
  }
  function move(e: PointerEvent) {
    keyboard = false; // the mouse took over: the status line goes quiet until an arrow key
    if (!points.length) return;
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    active = nearest(x.invert(e.clientX - r.left + LEFT)); // the hit area starts at LEFT, the scale at 0
  }
  function keys(e: KeyboardEvent) {
    if (!points.length) return;
    const last = points.length - 1;
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
      <tr><th scope="col" class="caps">Date</th>{#if rawHeading != null}<th scope="col" class="caps">{rawHeading}</th>{/if}<th scope="col" class="caps">{trendHeading}</th></tr>
    </thead>
    <tbody>
      {#each table as r}
        <tr><th scope="row">{r.date_text}</th>{#if rawHeading != null}<td class="num">{r.raw_text}</td>{/if}<td class="num">{r.trend_text}</td></tr>
      {/each}
    </tbody>
  </table>
  {#if !table.length && emptyText}<p class="fine">{emptyText}</p>{/if}
{:else if !points.length}
  <p class="fine">{emptyText ?? ''}</p>
{:else}
  <div class="wrap" bind:clientWidth={wrapWidth} style:height="{height}px">
    <svg {width} {height} viewBox="0 0 {width} {height}" role="img" aria-label={label}>
      {#each yTicks as t (t)}
        <line class="grid-line" x1={LEFT} x2={width - RIGHT} y1={y(t)} y2={y(t)} />
        <text class="tick num" x={LEFT - 8} y={y(t)} text-anchor="end" dominant-baseline="middle">{String(t).replace('-', '−')} {unitText}</text>
      {/each}
      {#each xTicks as t (t)}
        <text class="tick num" x={x(t)} y={height - BOTTOM + 16} text-anchor="middle" dominant-baseline="middle">{dayLabel(t, { month: 'short', day: 'numeric' })}</text>
      {/each}
      {#if goal != null}
        <line class="goal" x1={LEFT} x2={width - RIGHT} y1={y(goal)} y2={y(goal)} stroke-dasharray="4 4" />
        <!-- a goal line at the very top keeps its label under it, inside the chart -->
        {#if goalText}<text class="tick num" x={width - RIGHT} y={y(goal) - TOP < 16 ? y(goal) + 16 : y(goal) - 8} text-anchor="end">{goalText}</text>{/if}
      {/if}
      {#if trendPath}<path class="trend" d={trendPath} fill="none" />{/if}
      {#each dots as p (p.day)}
        <circle class="dot" cx={x(p.day)} cy={y(p.raw as number)} r="3" />
      {/each}
      {#if at}<line class="hair" x1={x(at.day)} x2={x(at.day)} y1={TOP} y2={height - BOTTOM} />{/if}
    </svg>
    <!-- the pointer and the arrow keys pick the nearest point; the table behind Show as Table is the screen-reader path to every one -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div
      class="hit"
      role="application"
      tabindex="0"
      aria-label="Chart points. Left and right arrow keys move between weigh-ins."
      style:left="{LEFT}px"
      style:right="{RIGHT}px"
      style:top="{TOP}px"
      style:bottom="{BOTTOM}px"
      onpointermove={move}
      onpointerleave={() => { if (!keyboard) active = null; }}
      onfocus={(e) => { keyboard = e.currentTarget.matches(':focus-visible'); active ??= points.length - 1; }}
      onblur={() => { keyboard = false; active = null; }}
      onkeydown={keys}
    ></div>
    <span class="sr" role="status">{keyboard ? said : ''}</span>
    {#if at && table[active as number]}
      {@const row = table[active as number]}
      {@const py = Math.min(Math.max(y(at.raw ?? at.trend ?? yMin), 48), height - BOTTOM - 48)}
      <div class="tip fine" style:top="{py}px" style:left={side === 'right' ? `${x(at.day) + 16}px` : undefined} style:right={side === 'left' ? `${width - x(at.day) + 16}px` : undefined}>
        <span>{row.date_text}</span>
        {#if rawHeading != null}<span class="num">{row.raw_text}</span>{/if}
        <span class="num">{trendHeading} {row.trend_text}</span>
      </div>
    {/if}
  </div>
{/if}

<style>
  .fine { margin: 0; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .wrap { position: relative; width: 100%; }
  svg { display: block; }
  .grid-line { stroke: var(--rule); stroke-width: 1; }
  .goal { stroke: var(--ink-3); stroke-width: 1; }
  .hair { stroke: var(--ink-3); stroke-width: 1; }
  .trend { stroke: var(--body-ink); stroke-width: 2; stroke-linejoin: round; stroke-linecap: round; }
  .dot { fill: var(--ink-3); }
  .tick { fill: var(--ink-3); font-size: 13px; }
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
    transform: translateY(-50%);
    pointer-events: none;
    white-space: nowrap;
  }
  .grid { width: 100%; border-collapse: collapse; }
  .grid th, .grid td { padding: 8px 16px; text-align: right; border-bottom: 1px solid var(--rule); }
  .grid th:first-child { text-align: left; padding-left: 0; font-weight: 600; }
  .grid thead th { font-size: 13px; color: var(--ink-2); }
</style>
