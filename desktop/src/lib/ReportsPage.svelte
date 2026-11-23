<script lang="ts">
  import { untrack } from 'svelte';
  import Donut from './Donut.svelte';
  import Icon from './Icon.svelte';
  import Meter from './Meter.svelte';
  import Segmented from './Segmented.svelte';
  import * as api from './api';
  import type { BodyRange, Report, Section } from './api';

  // Reports: averages over finished days that have food logged. Every number and sentence is the crate's; the page lays
  // them out. Over a goal is amber words, never red.
  let { onerror, ongoto }: {
    onerror: (m: string) => void;
    /** opens another section (Diary, Targets) */
    ongoto: (s: Section) => void;
  } = $props();

  let today = $state(api.localDay());
  let range: BodyRange = $state('1m');
  const ranges: BodyRange[] = ['2w', '1m', '3m', '6m', '1y', 'all'];
  const rangeShort: Record<BodyRange, string> = { '2w': '2W', '1m': '1M', '3m': '3M', '6m': '6M', '1y': '1Y', all: 'All' };
  const rangeName: Record<BodyRange, string> = { '2w': '2 weeks', '1m': '1 month', '3m': '3 months', '6m': '6 months', '1y': '1 year', all: 'All time' };
  let r: Report | null = $state(null);
  let busy = $state(false);
  let loadError = $state('');
  let seq = 0;

  async function load() {
    const mine = ++seq; // a slower older answer never replaces a newer one
    busy = true;
    try {
      const v = await api.reportView(range, today);
      if (mine === seq) { r = v; loadError = ''; }
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
</script>

<svelte:document onvisibilitychange={visible} />

<div class="reports-page" aria-busy={busy}>
  <div class="title">
    <h1>Reports</h1>
    <Segmented options={ranges} value={range} onchange={(v) => (range = v)} label="Report range" title="Report range">
      {#snippet item(v)}<span aria-hidden="true" title={rangeName[v]}>{rangeShort[v]}</span><span class="sr">{rangeName[v]}</span>{/snippet}
    </Segmented>
  </div>

  {#if loadError}
    <div class="notice" role="alert">
      <Icon name="warn" size={16} />
      <p>Couldn't make the report: {loadError}. Press Try Again, or restart Tiffin if it keeps happening.</p>
      <button type="button" class="btn ghost" onclick={load}>Try Again</button>
    </div>
  {/if}

  {#if r}
    <p class="fine">{r.range_text} · {r.days_text}</p>

    {#if r.empty_text}
      <div class="empty">
        <span class="etile"><Icon name="report" size={30} /></span>
        <p>{r.empty_text}</p>
        <button type="button" class="btn ghost" onclick={() => ongoto('diary')}>Open Diary</button>
      </div>
    {:else}
      <div class="cols">
        <section aria-labelledby="eaten-h">
          <h2 class="caps" id="eaten-h">Calories Eaten</h2>
          {#if r.split.length}
            <Donut slices={r.split} label={r.split_label} center={r.consumed_figure} centerUnit={r.consumed_unit} />
          {:else}
            <p class="big">{r.consumed_text}</p>
            <p class="fine">{r.split_label}</p>
          {/if}
          {#if r.consumed_note}<p class="fine quiet">{r.consumed_note}</p>{/if}
          {#if r.split_note}<p class="fine quiet">{r.split_note}</p>{/if}
        </section>

        <!-- one column, two sections: Remaining is its own region, not a heading inside Burned -->
        <div>
          <section aria-labelledby="burned-h">
            <h2 class="caps" id="burned-h">Calories Burned</h2>
            {#if r.burned.length}
              <dl class="sum">
                {#each r.burned as row (row.label)}
                  <div class:total={row.total}><dt>{row.label}</dt><dd class="num">{row.value_text}</dd></div>
                {/each}
              </dl>
            {/if}
            {#if r.burned_note}<p class="fine quiet">{r.burned_note}</p>{/if}
          </section>

          <section aria-labelledby="left-h">
            <h2 class="caps" id="left-h">Remaining</h2>
            <p class="left" class:over={r.remaining_over}>{r.remaining_text}</p>
            {#if r.remaining_note}<p class="fine quiet">{r.remaining_note}</p>{/if}
            {#if !r.remaining_known}<button type="button" class="btn ghost inline" onclick={() => ongoto('targets')}>Set Your Targets</button>{/if}
          </section>
        </div>
      </div>

      <section aria-labelledby="macros-h">
        <h2 class="caps" id="macros-h">Macros Against Targets</h2>
        <ul class="macros">
          {#each r.macros as m (m.name)}
            <li>
              <span class="caps lab">{m.name}</span>
              <span class="fine" class:over={m.over}>{m.text}</span>
              {#if m.goal != null}<Meter value={m.eaten} goal={m.goal} label={m.name} tone={m.tone} said={m.text} over={m.over} />{/if}
              {#if m.note}<span class="fine quiet">{m.note}</span>{/if}
            </li>
          {/each}
        </ul>
      </section>

      <section aria-labelledby="nutrients-h">
        <h2 class="caps" id="nutrients-h">Nutrients, Daily Average</h2>
        <table>
          <caption class="sr">Nutrients, daily average over {r.range_text}</caption>
          <thead>
            <tr><th scope="col" class="caps">Nutrient</th><th scope="col" class="caps">Daily average</th><th scope="col" class="caps">Days counted</th></tr>
          </thead>
          <tbody>
            {#each r.nutrients as n (n.name)}
              <tr><th scope="row">{n.name}</th><td class="num">{n.value_text}</td><td class="fine">{n.note ?? ''}</td></tr>
            {/each}
          </tbody>
        </table>
      </section>
    {/if}
  {/if}
</div>

<style>
  /* the column is 840 px of content; the one layout threshold is this column's own width, never the window's */
  .reports-page { container-type: inline-size; max-width: 904px; margin: 0 auto; padding: 0 32px 64px; font-size: 14.5px; line-height: 1.45; }
  .fine { margin: 0; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .quiet { color: var(--ink-3); margin-top: 8px; }
  .title { display: flex; align-items: center; justify-content: space-between; gap: 16px; min-height: 64px; padding-top: 16px; flex-wrap: wrap; }
  h1 { margin: 0 8px 0 4px; font: 700 24px var(--font-display); letter-spacing: -0.018em; color: var(--reports-ink); }
  section { margin-top: 32px; }
  h2.caps { margin: 0 0 16px; font-size: 14.5px; color: var(--ink); }
  p { margin: 0; }
  .lab { font-size: 13px; color: var(--ink-2); }

  .notice { display: flex; align-items: flex-start; gap: 16px; margin: 8px 0 0; padding: 8px 16px; border-radius: 14px; background: var(--warn-wash); color: var(--warn); }
  .notice :global(svg) { flex: none; margin-top: 4px; }
  .notice p { flex: 1; }
  .empty { display: flex; flex-direction: column; align-items: center; gap: 8px; margin-top: 64px; text-align: center; color: var(--ink-2); }
  .etile { display: inline-grid; place-items: center; width: 64px; height: 64px; margin-bottom: 8px; border-radius: 18px; color: #fff; background: var(--tile-reports); box-shadow: var(--shadow-lift); }

  .cols { display: grid; grid-template-columns: 1fr 1fr; gap: 0 32px; }
  .big, .left { font: 700 22px var(--font-display); line-height: 1.2; color: var(--ink); }
  .left.over, .fine.over { color: var(--warn); }
  .sum { margin: 0; border-top: 1px solid var(--rule); }
  .sum div { display: flex; justify-content: space-between; gap: 16px; padding: 8px 0; border-bottom: 1px solid var(--rule); }
  .sum dt { color: var(--ink-2); }
  .sum dd { margin: 0; }
  .sum .total dt, .sum .total dd { font-weight: 600; color: var(--ink); }
  .inline { height: auto; min-height: 0; padding: 0; margin-top: 8px; }

  .macros { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 16px; margin: 0; padding: 0; list-style: none; }
  .macros li { display: flex; flex-direction: column; gap: 4px; min-width: 0; }

  table { width: 100%; border-collapse: collapse; border-top: 1px solid var(--rule); }
  th, td { padding: 8px 16px 8px 0; text-align: left; border-bottom: 1px solid var(--rule); }
  th { font-weight: 600; width: 30%; }
  thead th { font-size: 13px; font-weight: 400; color: var(--ink-2); }
  td.num { width: 25%; }

  @container (max-width: 639px) {
    .cols { grid-template-columns: 1fr; }
    .macros { grid-template-columns: 1fr; }
  }
</style>
