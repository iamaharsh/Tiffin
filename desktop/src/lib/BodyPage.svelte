<script lang="ts">
  import { tick, untrack } from 'svelte';
  import Icon from './Icon.svelte';
  import LineChart from './LineChart.svelte';
  import LogMeasurementDialog from './LogMeasurementDialog.svelte';
  import EditMeasurementsDialog from './EditMeasurementsDialog.svelte';
  import ManageMeasuresDialog from './ManageMeasuresDialog.svelte';
  import Menu, { type MenuItem } from './Menu.svelte';
  import Segmented from './Segmented.svelte';
  import * as api from './api';
  import type { BodyLogged, BodyRange, BodyRow, BodyView, MeasureOption } from './api';

  // The Body page prints the crate's answers and maps nothing but pixels: the trend, the rate, the forecast and every
  // table cell are the crate's numbers and sentences. No day is praised or warned about.
  let { onerror, onnote }: {
    onerror: (m: string) => void;
    onnote: (m: string, action?: { label: string; run: () => void } | null) => void;
  } = $props();

  // the page opens on today every time the section opens: a stale day is how a value lands on the wrong date
  let today = $state(api.localDay());
  let range: BodyRange = $state('3m');
  const ranges: BodyRange[] = ['2w', '1m', '3m', '6m', '1y', 'all'];
  const rangeShort: Record<BodyRange, string> = { '2w': '2W', '1m': '1M', '3m': '3M', '6m': '6M', '1y': '1Y', all: 'All' };
  const rangeName: Record<BodyRange, string> = { '2w': '2 weeks', '1m': '1 month', '3m': '3 months', '6m': '6 months', '1y': '1 year', all: 'All time' };
  let asTable = $state(false);
  let panel: HTMLElement | undefined = $state();
  let view: BodyView | null = $state(null);
  let busy = $state(false);
  let loadError = $state('');
  let logging = $state(false);
  let editing: BodyRow | null = $state(null);
  let managing = $state(false);
  let menu: { x: number; y: number; items: MenuItem[] } | null = $state(null);
  let opener: HTMLElement | undefined;
  let seq = 0;
  /** until when the page offers the crate's one undo (10 s, as the toast stays) */
  let pending: { until: number } | null = null;

  async function load() {
    const mine = ++seq; // a slower older answer never replaces a newer one
    busy = true;
    try {
      const v = await api.bodyView(range, today);
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
  /** back from a long absence (or past midnight): re-read today, so the chart's edge and the dialogs' day move on (WR-03) */
  function visible() {
    if (document.visibilityState !== 'visible') return;
    const now = api.localDay();
    if (now !== today) today = now;
  }

  /** "Nov 15", from the day key's own parts (never parsed as a date string, that is UTC), with the year when it isn't this one */
  function dateOf(key: string) {
    const [y, m, d] = key.split('-').map(Number);
    const opts: Intl.DateTimeFormatOptions = { month: 'short', day: 'numeric' };
    if (y !== Number(today.slice(0, 4))) opts.year = 'numeric';
    return new Date(y, m - 1, d).toLocaleDateString(undefined, opts);
  }

  async function undo() {
    if (!pending || Date.now() > pending.until) return;
    pending = null;
    try {
      seq++; // a load started before the undo never replaces its answer
      view = await api.undoBody(range, today);
      onnote('Put back');
    } catch (e) {
      onerror(`Couldn't undo that: ${api.reason(e)}. Your changes are unchanged.`);
      load();
    }
  }

  function logged(r: BodyLogged, m: MeasureOption, day: string) {
    seq++; // an older load never replaces what the save just showed
    if (r.view) view = r.view;
    else load();
    pending = { until: Date.now() + 10000 };
    onnote(`Logged ${m.name.toLowerCase()} ${r.text} for ${dateOf(day)}`, { label: 'Undo', run: undo });
  }

  function edited(r: BodyLogged, day: string) {
    seq++;
    if (r.view) view = r.view;
    else load();
    pending = { until: Date.now() + 10000 };
    onnote(`Updated measurements for ${dateOf(day)}`, { label: 'Undo', run: undo });
  }

  /** the row's date button gets focus back; the row is still there because a day is never removed by an edit */
  function closeEdit() {
    const day = editing?.day;
    editing = null;
    queueMicrotask(() => document.querySelector<HTMLElement>(`[data-day="${CSS.escape(day ?? '')}"]`)?.focus());
  }

  function more(e: MouseEvent) {
    const button = e.currentTarget as HTMLElement;
    const r = button.getBoundingClientRect();
    menu = { x: r.right - 220, y: r.bottom + 8, items: [{ label: 'Manage Measures…', icon: 'gear', disabled: !!view?.read_only, run: () => { opener = button; managing = true; } }] };
  }
  function closeManage() {
    managing = false;
    queueMicrotask(() => opener?.focus());
  }

  /** an archive has its own Undo: restore the measure (the diary's undo slot is not involved, so the weigh-in Undo stays valid) */
  function archived(v: BodyView, m: MeasureOption) {
    seq++;
    view = v;
    onnote(`Archived ${m.name}`, {
      label: 'Undo',
      run: async () => {
        try {
          seq++;
          view = await api.archiveMeasure(m.id, false, range, today);
          onnote('Put back');
        } catch (e) {
          onerror(`Couldn't undo that: ${api.reason(e)}. Your changes are unchanged.`);
          load();
        }
      },
    });
  }

  /** swap the chart and its table; focus moves to the swapped-in content so a keyboard user lands on it */
  async function toggleTable() {
    asTable = !asTable;
    await tick();
    panel?.focus();
  }

  function openLog(e: MouseEvent) {
    if (view?.read_only) return;
    opener = e.currentTarget as HTMLElement;
    logging = true;
  }
  function closeLog() {
    logging = false;
    queueMicrotask(() => opener?.focus()); // after the modal is gone: while it is open the page behind is inert
  }
</script>

<svelte:document onvisibilitychange={visible} />

<div class="body-page" aria-busy={busy}>
  <div class="title">
    <h1>Body</h1>
    <span class="actions">
      <button type="button" class="btn primary" aria-disabled={view?.read_only ? 'true' : undefined} title={view?.read_only ?? undefined} onclick={openLog}>Log Measurement</button>
      <button type="button" class="icon" aria-label="More body actions" aria-haspopup="menu" onclick={more}><Icon name="more" size={18} /></button>
    </span>
  </div>

  {#if loadError}
    <div class="notice" role="alert">
      <Icon name="warn" size={16} />
      <p>Couldn't open your body log: {loadError}. Press Try Again, or restart Tiffin if it keeps happening.</p>
      <button type="button" class="btn ghost" onclick={load}>Try Again</button>
    </div>
  {/if}
  {#if view?.read_only}
    <div class="notice" role="alert"><Icon name="warn" size={16} /><p>{view.read_only}</p></div>
  {/if}

  {#if view}
    {#if view.empty}
      <div class="empty">
        <span class="etile"><Icon name="trend" size={30} /></span>
        <p class="fine">Nothing logged yet.</p>
        <p>Log your weight to see your trend, or add a waist or body-fat measurement.</p>
      </div>
    {:else}
      {#if view.headline}
        <section class="headline" aria-label="Weight trend">
          <span class="caps fine" aria-hidden="true">Trend</span>
          <p class="figure" aria-hidden="true">
            <span class="num trend">{view.trend_text}</span>
            {#if view.rate_text}<span class="fine rate">{view.rate_text}</span>{/if}
          </p>
          <span class="sr">{view.headline}</span>
          {#if view.forecast}<p class="fine">{view.forecast}</p>{/if}
          <p class="fine quiet">{view.note}</p>
        </section>
      {/if}

      {#if view.chart}
        {@const c = view.chart}
        <section aria-label="Weight chart">
          <div class="controls">
            <Segmented options={ranges} value={range} onchange={(r) => (range = r)} label="Chart range" title="Chart range">
              {#snippet item(r)}<span aria-hidden="true" title={rangeName[r]}>{rangeShort[r]}</span><span class="sr">{rangeName[r]}</span>{/snippet}
            </Segmented>
            <button type="button" class="btn ghost" onclick={toggleTable}>{asTable ? 'Show Chart' : 'Show as Table'}</button>
          </div>
          <div class="panel" bind:this={panel} tabindex="-1">
            <LineChart
              points={c.points}
              xMin={c.x_min}
              xMax={c.x_max}
              yMin={c.y_min}
              yMax={c.y_max}
              goal={c.goal}
              goalText={c.goal_text}
              label={c.label}
              unitText={c.unit_text}
              emptyText={c.empty_text}
              table={c.table}
              {asTable}
            />
          </div>
        </section>
      {/if}

      <section>
        <h2 class="caps" id="measurements-h">Measurements</h2>
        <!-- a sideways-scrolling area has to take the keyboard, or a wide table can't be read without a mouse -->
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <div class="scroll" tabindex="0" role="region" aria-label="Measurements">
          <table>
            <thead>
              <tr>
                <th scope="col" class="date caps">Date</th>
                {#each view.columns as c (c.id)}
                  <th scope="col" class="caps">{c.name}{#if c.unit_text}<span class="unit">&nbsp;{c.unit_text}</span>{/if}</th>
                {/each}
              </tr>
            </thead>
            <tbody>
              {#each view.rows as r (r.day)}
                <!-- the whole row takes a click; the date button is its keyboard and screen-reader way in (Enter, Space) -->
                <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
                <tr onclick={() => { if (!view?.read_only) editing = r; }}>
                  <th scope="row" class="date"><button type="button" class="rowbtn" data-day={r.day} aria-disabled={view.read_only ? 'true' : undefined} title={view.read_only ?? undefined} aria-label="Edit measurements for {r.date_text}">{r.date_text}</button></th>
                  {#each r.cells as cell}
                    <td class="num">{#if cell != null}{cell}{:else}<span aria-hidden="true">–</span><span class="sr">not logged</span>{/if}</td>
                  {/each}
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      </section>
    {/if}
  {/if}
</div>

{#if editing && view}
  <EditMeasurementsDialog {view} row={editing} dateText={editing.date_text} {today} {range} onclose={closeEdit} onsaved={edited} {onerror} />
{/if}
{#if managing && view}
  <ManageMeasuresDialog {view} {range} {today} onclose={closeManage} onview={(v) => { seq++; view = v; }} onarchived={archived} />
{/if}
{#if menu}<Menu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />{/if}

{#if logging && view}
  <LogMeasurementDialog {view} {today} {range} onclose={closeLog} onlogged={logged} {onerror} />
{/if}

<style>
  /* the column is 840 px of content; the one layout threshold is this column's own width, never the window's */
  .body-page { container-type: inline-size; max-width: 904px; margin: 0 auto; padding: 0 32px 64px; font-size: 14.5px; line-height: 1.45; }
  .fine { margin: 0; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .quiet { color: var(--ink-3); }
  .title { display: flex; align-items: center; justify-content: space-between; gap: 16px; min-height: 64px; }
  h1 { margin: 0 8px 0 4px; font: 700 24px var(--font-display); letter-spacing: -0.018em; color: var(--body-ink); }
  section { margin-top: 32px; }
  h2.caps { margin: 0 0 16px; font-size: 14.5px; color: var(--ink); }
  p { margin: 0; }
  .btn[aria-disabled='true'], .rowbtn[aria-disabled='true'] { opacity: 0.5; }
  .actions { display: flex; align-items: center; gap: 8px; }
  .icon { display: grid; place-items: center; width: 32px; height: 32px; border-radius: 9px; color: var(--accent-text); }
  .icon:hover { background: var(--accent-wash); }

  .notice { display: flex; align-items: flex-start; gap: 16px; margin: 8px 0 0; padding: 8px 16px; border-radius: 14px; background: var(--warn-wash); color: var(--warn); }
  .notice :global(svg) { flex: none; margin-top: 4px; }
  .notice p { flex: 1; }

  .empty { display: flex; flex-direction: column; align-items: center; gap: 8px; margin-top: 64px; text-align: center; color: var(--ink-2); }
  .etile { display: inline-grid; place-items: center; width: 64px; height: 64px; margin-bottom: 8px; border-radius: 18px; color: #fff; background: var(--tile-body); box-shadow: var(--shadow-lift); }

  .headline { display: flex; flex-direction: column; align-items: flex-start; gap: 4px; }
  .figure { display: flex; flex-wrap: wrap; align-items: baseline; gap: 4px 16px; }
  .trend { font-size: 26px; line-height: 1.1; color: var(--body-ink); }

  .controls { display: flex; align-items: center; justify-content: space-between; gap: 16px; margin-bottom: 16px; }
  .panel { padding: 16px; border-radius: 14px; background: var(--card); box-shadow: inset 0 0 0 1px var(--rule); outline: none; }
  .panel:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  .scroll { overflow-x: auto; border-top: 1px solid var(--rule); }
  table { width: 100%; border-collapse: separate; border-spacing: 0; }
  th, td { min-width: 88px; height: 48px; padding: 8px 16px; text-align: right; white-space: nowrap; border-bottom: 1px solid var(--rule); }
  thead th { font-size: 13px; color: var(--ink-2); }
  .unit { font-family: var(--font-text); font-weight: 400; text-transform: none; letter-spacing: 0; color: var(--ink-3); }
  .date { position: sticky; left: 0; min-width: 112px; width: 112px; padding-left: 0; text-align: left; background: var(--page); font-weight: 400; }
  tbody th.date { font-weight: 600; }
  tbody tr:hover > * { background: var(--well); }
  tbody tr:hover th.date { background: var(--well); }
  .rowbtn { padding: 0; font: inherit; color: inherit; text-align: left; border-radius: 4px; }
  .rowbtn:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  td.num { font-size: 14.5px; }
</style>
