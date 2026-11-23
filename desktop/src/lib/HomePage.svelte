<script lang="ts">
  import { tick, untrack } from 'svelte';
  import Icon from './Icon.svelte';
  import LineChart from './LineChart.svelte';
  import Meter from './Meter.svelte';
  import Segmented from './Segmented.svelte';
  import StackedBars from './StackedBars.svelte';
  import * as api from './api';
  import type { BodyRange, HomeView, MacroGoal, Section } from './api';

  // Home: today at a glance, what's planned, what was logged lately, how often the macro goals were met, the streak and
  // two charts. Every number and sentence is the crate's; the page lays them out. Nothing judges a day: over is amber words.
  let { onerror, ongoto }: {
    onerror: (m: string) => void;
    /** opens another section (Diary, Targets, Meal Plan) */
    ongoto: (s: Section) => void;
  } = $props();

  // the page opens on today and re-reads it when the window comes back: a stale day shows yesterday's numbers as today's
  let today = $state(api.localDay());
  let range: BodyRange = $state('2w');
  const ranges: BodyRange[] = ['2w', '1m', '3m', '6m', '1y', 'all'];
  const rangeShort: Record<BodyRange, string> = { '2w': '2W', '1m': '1M', '3m': '3M', '6m': '6M', '1y': '1Y', all: 'All' };
  const rangeName: Record<BodyRange, string> = { '2w': '2 weeks', '1m': '1 month', '3m': '3 months', '6m': '6 months', '1y': '1 year', all: 'All time' };
  let view: HomeView | null = $state(null);
  let busy = $state(false);
  let loadError = $state('');
  let asTable = $state(false);
  let charts: HTMLElement | undefined = $state();
  let seq = 0;

  async function load() {
    const mine = ++seq; // a slower older answer never replaces a newer one
    busy = true;
    try {
      const v = await api.homeView(range, today);
      if (mine === seq) { view = v; loadError = ''; }
    } catch (e) {
      if (mine === seq) loadError = api.reason(e);
      onerror(String(e));
    } finally {
      if (mine === seq) busy = false;
    }
  }
  $effect(() => {
    range;
    today;
    untrack(load);
  });
  function visible() {
    if (document.visibilityState !== 'visible') return;
    const now = api.localDay();
    if (now !== today) today = now;
  }
  /** swap the charts and their tables; focus moves to the swapped-in content so a keyboard user lands on it */
  async function toggleTable() {
    asTable = !asTable;
    await tick();
    charts?.focus();
  }
  const macros = (b: NonNullable<HomeView['budget']>): [string, MacroGoal, 'protein' | 'carbs' | 'fat'][] => [
    ['Protein', b.protein, 'protein'],
    ['Net Carbs', b.carbs, 'carbs'],
    ['Fat', b.fat, 'fat'],
  ];
</script>

<svelte:document onvisibilitychange={visible} />

<div class="home-page" aria-busy={busy}>
  <div class="title">
    <h1>Home</h1>
    {#if view}<p class="fine">{view.date_text}</p>{/if}
  </div>

  {#if loadError}
    <div class="notice" role="alert">
      <Icon name="warn" size={16} />
      <p>Couldn't open your dashboard: {loadError}. Press Try Again, or restart Tiffin if it keeps happening.</p>
      <button type="button" class="btn ghost" onclick={load}>Try Again</button>
    </div>
  {/if}

  {#if view}
    {@const b = view.budget}
    <section class="bar" aria-labelledby="today-h">
      <h2 class="sr" id="today-h">Today</h2>
      {#if b}
        <div class="today">
          <div class="rem" class:over={b.over}>
            <span class="caps lab">{b.label}</span>
            <p class="figure">{#if b.figure_text == null}<span aria-hidden="true">–</span>{:else}<span class="num">{b.figure_text}</span>{/if}{#if b.figure != null}<span class="unit">&nbsp;kcal</span>{/if}</p>
            <span class="sr">{b.said}</span>
          </div>
          <div class="meters">
            {#each macros(b) as [name, g, tone] (name)}
              <div class="macro" class:over={g.over}>
                <span class="caps lab">{name}</span>
                <span class="fine">{#if g.eaten == null}<span aria-hidden="true">–</span><span class="sr">not reported for some foods</span>{:else}<span class="num">{g.eaten.toLocaleString('en-US', { maximumFractionDigits: 20 })}</span>{/if}&nbsp;{g.unit_text}</span>
                <Meter value={g.eaten} goal={g.goal} label={name} {tone} said={g.said} over={g.over} />
              </div>
            {/each}
          </div>
        </div>
        <p class="fine formula">{b.formula}</p>
      {:else}
        <p class="fine">{view.budget_note ?? "Set a daily calorie target to see what's left each day."}</p>
      {/if}
      <div class="links">
        <button type="button" class="btn ghost" onclick={() => ongoto('diary')}>Open Diary</button>
        {#if !b && view.budget_state !== 'unavailable'}<button type="button" class="btn ghost" onclick={() => ongoto('targets')}>Set Your Targets</button>{/if}
      </div>
    </section>

    {#if b}
      <section aria-labelledby="fits-h">
        <h2 class="caps" id="fits-h">Fits What's Left</h2>
        {#if view.fits.note}
          <p class="fine">{view.fits.note}</p>
        {:else}
          <div class="cols">
            {#each [['Foods', view.fits.foods], ['Your Recipes', view.fits.recipes]] as const as [heading, items] (heading)}
              <div>
                <h3 class="caps lab sub">{heading}</h3>
                {#if items.length}
                  <ul class="list">
                    {#each items as f (f.key)}
                      <li>
                        <span class="name" title={f.name}>{f.name}</span>
                        <span class="fine"><span class="num">{f.amount_text}</span> · {f.macros_text}{heading === 'Foods' ? ` · ${f.tag}` : ''}</span>
                      </li>
                    {/each}
                  </ul>
                {:else}
                  <p class="fine">{heading === 'Foods' ? "No food fits what's left." : "No recipe with a nutrition estimate fits what's left."}</p>
                {/if}
              </div>
            {/each}
          </div>
          <p class="fine quiet fits-note">One ordinary portion at most, keeping protein, net carbs and fat within what's left today. Add one from the Diary.</p>
        {/if}
      </section>
    {/if}

    <div class="cols">
      <section aria-labelledby="planned-h">
        <h2 class="caps" id="planned-h">Planned Today</h2>
        {#if view.planned.length}
          <ul class="list">
            {#each view.planned as m (m.meal_id)}
              <li>
                <span class="name" title={m.name}>{m.name}</span>
                <span class="fine">{m.slot_label}{#if m.kcal_text} · <span class="num">{m.kcal_text}</span>{/if}</span>
                {#if m.note}<span class="fine quiet">{m.note}</span>{/if}
              </li>
            {/each}
          </ul>
        {:else}
          <p class="fine">Nothing planned for today.</p>
          <button type="button" class="btn ghost inline" onclick={() => ongoto('meals')}>Open Meal Plan</button>
        {/if}
      </section>

      <section aria-labelledby="recent-h">
        <h2 class="caps" id="recent-h">Recent</h2>
        {#if view.recent.length}
          <ul class="list">
            {#each view.recent as r, i (i)}
              <li><span class="name" title={r.title}>{r.title}</span><span class="fine">{r.detail} · {r.when_text}</span></li>
            {/each}
          </ul>
        {:else}
          <p class="fine">Nothing logged in the last week.</p>
          <button type="button" class="btn ghost inline" onclick={() => ongoto('diary')}>Open Diary</button>
        {/if}
      </section>
    </div>

    <div class="cols">
      <section aria-labelledby="goals-h">
        <h2 class="caps" id="goals-h">Macro Goals Met</h2>
        {#if view.rates_note}
          <p class="fine">{view.rates_note}</p>
        {:else}
          <ul class="rates">
            {#each view.hit_rates as r (r.id)}
              <li>
                <span class="caps lab">{r.label}</span>
                <!-- no rate yet: the dash is for the eye; the text beside it says why ("No days to judge yet") -->
                <span class="pct num" aria-hidden={r.pct == null ? 'true' : undefined}>{r.pct == null ? '–' : `${r.pct}%`}</span>
                <span class="fine">{r.text}</span>
              </li>
            {/each}
          </ul>
          <p class="fine quiet">Days with protein, net carbs and fat each within 20% of that day's goal. Today counts once it's over.</p>
        {/if}
      </section>

      <section aria-labelledby="streak-h">
        <h2 class="caps" id="streak-h">Streak</h2>
        <p class="streak num" aria-hidden="true">{view.streak}</p>
        <p class="fine">{view.streak_text}</p>
      </section>
    </div>

    <section aria-label="Charts">
      <div class="controls">
        <Segmented options={ranges} value={range} onchange={(r) => (range = r)} label="Chart range" title="Chart range">
          {#snippet item(r)}<span aria-hidden="true" title={rangeName[r]}>{rangeShort[r]}</span><span class="sr">{rangeName[r]}</span>{/snippet}
        </Segmented>
        <button type="button" class="btn ghost" onclick={toggleTable}>{asTable ? 'Show Charts' : 'Show as Tables'}</button>
      </div>
      <div class="panels" bind:this={charts} tabindex="-1">
        <div class="panel">
          <h2 class="caps">Calories</h2>
          <StackedBars bars={view.energy.bars} xMin={view.energy.x_min} xMax={view.energy.x_max} yMax={view.energy.y_max} label={view.energy.label} emptyText={view.energy.empty_text} table={view.energy.table} {asTable} />
        </div>
        <div class="panel">
          <h2 class="caps">Weight Change{#if view.weight_change_text}<span class="unit">&nbsp;{view.weight_change_text}</span>{/if}</h2>
          <LineChart points={view.weight.points} xMin={view.weight.x_min} xMax={view.weight.x_max} yMin={view.weight.y_min} yMax={view.weight.y_max} goal={view.weight.goal} goalText={view.weight.goal_text} label={view.weight.label} unitText={view.weight.unit_text} emptyText={view.weight.empty_text} table={view.weight.table} {asTable} rawHeading={null} trendHeading="Change" />
        </div>
      </div>
    </section>
  {/if}
</div>

<style>
  /* the column is 840 px of content; the one layout threshold is this column's own width, never the window's */
  .home-page { container-type: inline-size; max-width: 904px; margin: 0 auto; padding: 0 32px 64px; font-size: 14.5px; line-height: 1.45; }
  .fine { margin: 0; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .quiet { color: var(--ink-3); }
  .rates + .quiet, .fits-note { margin-top: 16px; }
  .sub { margin: 0 0 8px; font-weight: 600; }
  .title { display: flex; align-items: baseline; justify-content: space-between; gap: 16px; min-height: 64px; padding-top: 16px; }
  h1 { margin: 0 8px 0 4px; font: 700 24px var(--font-display); letter-spacing: -0.018em; color: var(--home-ink); }
  section { margin-top: 32px; }
  h2.caps { margin: 0 0 16px; font-size: 14.5px; color: var(--ink); }
  p { margin: 0; }
  .unit { font-family: var(--font-text); font-weight: 400; text-transform: none; letter-spacing: 0; color: var(--ink-3); font-size: 13px; }
  .lab { font-size: 13px; color: var(--ink-2); }

  .notice { display: flex; align-items: flex-start; gap: 16px; margin: 8px 0 0; padding: 8px 16px; border-radius: 14px; background: var(--warn-wash); color: var(--warn); }
  .notice :global(svg) { flex: none; margin-top: 4px; }
  .notice p { flex: 1; }

  .bar { margin-top: 8px; padding: 16px 24px; border-radius: 14px; background: var(--card); box-shadow: var(--shadow-card), inset 0 0 0 1px var(--rule); }
  .today { display: flex; flex-wrap: wrap; gap: 16px 32px; }
  .rem { display: flex; flex-direction: column; gap: 4px; min-width: 160px; }
  .figure { font: 700 30px var(--font-display); line-height: 1.1; color: var(--ink); }
  .rem.over .lab, .rem.over .figure, .macro.over .fine { color: var(--warn); }
  .meters { flex: 1; display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 16px; min-width: 280px; }
  .macro { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .formula { margin-top: 16px; }
  .links { display: flex; flex-wrap: wrap; gap: 8px 16px; margin-top: 8px; }
  .links .btn, .inline { height: auto; min-height: 0; padding: 0; }
  .inline { margin-top: 8px; }

  .cols { display: grid; grid-template-columns: 1fr 1fr; gap: 0 32px; }
  .cols > div { min-width: 0; } /* a long food name ellipsizes instead of widening its column */
  .list { display: flex; flex-direction: column; margin: 0; padding: 0; list-style: none; border-top: 1px solid var(--rule); }
  .list li { display: flex; flex-direction: column; gap: 2px; padding: 8px 0; border-bottom: 1px solid var(--rule); min-width: 0; }
  .name { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .rates { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 16px; margin: 0; padding: 0; list-style: none; }
  .rates li { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .pct { font: 700 22px var(--font-display); line-height: 1.2; color: var(--ink); }
  .streak { font: 700 30px var(--font-display); line-height: 1.1; color: var(--home-ink); margin-bottom: 4px; }

  .controls { display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-bottom: 16px; }
  .panels { display: flex; flex-direction: column; gap: 16px; outline: none; border-radius: 14px; }
  .panels:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  .panel { padding: 16px; border-radius: 14px; background: var(--card); box-shadow: inset 0 0 0 1px var(--rule); }

  /* under 640 px of its own width the two-column rows stack and the three-across grids drop to two across */
  @container (max-width: 639px) {
    .cols { grid-template-columns: 1fr; }
    .meters, .rates { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  }
</style>
