<script lang="ts">
  import { tick, untrack, type Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  import Dialog from './Dialog.svelte';
  import FoodDetail from './FoodDetail.svelte';
  import OffEmailDialog from './OffEmailDialog.svelte';
  import SourceTag from './SourceTag.svelte';
  import * as api from './api';
  import type { FoodDetail as Info, FoodRow, MealType, OnlineResult } from './api';

  // The one food search: local results on every keystroke, Open Food Facts only when asked.
  // `layout="page"` is the Foods page; `layout="dialog"` is the diary's Add Food dialog: planned, recent and
  // frequent foods before typing, and two panes or a stacked list by its own width (never a nested sheet).
  let { us, onerror, onadd, actions: extra, onenter, layout = 'page', today = '', day = '', mealSlot = 'Breakfast' }: {
    us: boolean;
    onerror: (m: string) => void;
    onadd?: (name?: string) => void;
    actions?: Snippet<[Info]>;
    /** Enter in the Amount box: the host does what its primary button does */
    onenter?: (d: Info) => void;
    layout?: 'page' | 'dialog';
    /** the local day, for ranking recent and frequent foods (dialog layout) */
    today?: string;
    /** the diary day being added to, for the planned recipes listed first (dialog layout) */
    day?: string;
    /** the meal being added to: its planned recipes come first (dialog layout). Not `slot`: Svelte reserves that attribute on components. */
    mealSlot?: MealType;
  } = $props();
  const dialog = $derived(layout === 'dialog');

  let q = $state('');
  let rows: FoodRow[] = $state([]);
  let loaded = $state(false);
  let onlineReady = $state(false);
  let failure = $state('');
  let preparing = $state(false);
  let searching = $state(false);
  let cooldown = $state(0);
  /** no contact email is saved: ask for one before anything goes online */
  let askEmail = $state(false);
  let status: { text: string; warn: boolean; ok?: boolean } | null = $state(null);
  /** dialog layout: foods logged before, taken once when the dialog opens and not reordered while it stays open */
  let planned: FoodRow[] = $state([]);
  let recent: FoodRow[] = $state([]);
  let frequent: FoodRow[] = $state([]);
  let history = false;
  let open: { key: string; name: string } | null = $state(null);
  let box: HTMLInputElement | undefined = $state();
  let list: HTMLElement | undefined = $state();
  let bodyEl: HTMLElement | undefined = $state();
  let bodyW = $state(0);
  let panel: HTMLElement | undefined = $state();
  let detail: { focusAmount: () => void; food: () => Info | null } | undefined = $state();
  let seq = 0;
  let gen = $state(0);

  // page layout: two columns from 1100 px wide; below that the detail opens as a sheet
  const wideQuery = window.matchMedia('(min-width: 1100px)');
  let wide = $state(wideQuery.matches);
  $effect(() => {
    const f = () => (wide = wideQuery.matches);
    wideQuery.addEventListener('change', f);
    return () => wideQuery.removeEventListener('change', f);
  });

  // dialog layout: measure the body itself, never the window (the sidebar and the dialog's own width move it)
  const split = $derived(dialog ? bodyW >= 680 : wide);
  $effect(() => {
    if (!bodyEl) return;
    bodyW = bodyEl.clientWidth;
    const ro = new ResizeObserver(([e]) => (bodyW = e.contentRect.width));
    ro.observe(bodyEl);
    return () => ro.disconnect();
  });

  async function search(text: string) {
    const mine = ++seq; // a slower older answer never replaces a newer one
    const timer = setTimeout(() => { if (mine === seq) preparing = true; }, 300);
    try {
      let got: { rows: FoodRow[]; online_ready: boolean };
      if (dialog) {
        const d = await api.diaryFoods(text, today, day, mealSlot);
        if (mine === seq && !history && !text.trim()) { planned = d.planned; recent = d.recent; frequent = d.frequent; history = true; }
        got = d;
      } else {
        got = await api.searchFoods(text);
      }
      if (mine === seq) { rows = got.rows; onlineReady = got.online_ready; failure = ''; loaded = true; }
    } catch (e) {
      if (mine === seq) failure = String(e);
      onerror(String(e));
    } finally {
      clearTimeout(timer);
      if (mine === seq) preparing = false;
    }
  }
  /** Search again with what is typed (the host calls this after a save). */
  export function refresh(closeDetail = false) {
    if (closeDetail) open = null;
    gen++; // the open food is read again, so an edit shows
    search(q);
  }
  function typed(e: Event & { currentTarget: HTMLInputElement }) {
    q = e.currentTarget.value;
    status = null;
    search(q);
  }
  $effect(() => {
    box?.focus();
    untrack(() => search(''));
  });

  const canOnline = $derived(onlineReady && !searching && cooldown <= 0);
  const cooling = $derived(cooldown > 0);
  $effect(() => {
    if (!cooling) return;
    const t = setInterval(() => { cooldown = Math.max(0, cooldown - 1); }, 1000);
    return () => clearInterval(t);
  });

  function describe(r: OnlineResult, text: string): { text: string; warn: boolean } | null {
    switch (r.status) {
      case 'ok': return { text: `Added ${r.added} online result${r.added === 1 ? '' : 's'} from Open Food Facts.`, warn: false };
      case 'empty': return { text: `Open Food Facts has nothing for “${text}”. Your own results are still listed.`, warn: false };
      case 'offline': return { text: "Online results unavailable: this PC can't reach Open Food Facts. Search on this PC still works.", warn: true };
      case 'slow': return { text: "Open Food Facts didn't answer. Try again in a moment. Search on this PC still works.", warn: true };
      case 'limited': return { text: `Open Food Facts asked us to slow down. Online search will be ready in ${r.retry_in} s.`, warn: true };
      default: return null;
    }
  }
  /** The only place that talks to Open Food Facts: the button and Ctrl+Enter, never typing. */
  async function searchOnline() {
    // the crate says when a query is long enough to send; the page counts no letters
    if (!canOnline) return;
    const text = q;
    ++seq; // a local search still in flight must not overwrite the merged rows
    preparing = false;
    searching = true;
    try {
      const r = await api.offSearch(text);
      if (r.status === 'needs_email') { askEmail = true; return; } // nothing was sent
      if (r.retry_in > 0) cooldown = r.retry_in;
      if (text === q) {
        rows = r.rows;
        const s = describe(r, text);
        if (s) status = s; // a 'wait' answer has no new text
      }
    } catch (e) {
      onerror(String(e));
    } finally {
      searching = false;
    }
  }

  async function openRow(r: FoodRow, andFocus = false) {
    open = { key: r.key, name: r.name };
    // caching is optional; a failed cache write never blocks looking at the food
    if (r.key.startsWith('off:')) api.offOpen(r.key.slice(4)).catch(() => {});
    if (andFocus) {
      await tick();
      detail?.focusAmount();
    }
  }
  /** Empty the search, release the open food and put the cursor back in the box (after an add). */
  export async function clear() {
    q = '';
    status = null;
    open = null;
    search('');
    await tick();
    box?.focus();
  }
  /** Say something in the status line, with a check, until the next keystroke. */
  export function confirm(text: string) {
    status = { text, warn: false, ok: true };
  }
  /** Esc: clear the typed text, or go back to the results when stacked. False when there is nothing left to undo, so the host closes. */
  export function escape(): boolean {
    if (q) { clear(); return true; }
    if (dialog && !split && open) { closeDetail(); return true; }
    return false;
  }
  /** The open food with the amount and unit chosen, or null when none is open or it is still loading (for the host's Add button). */
  export function chosen(): Info | null {
    return open ? (detail?.food() ?? null) : null;
  }
  async function closeDetail() {
    const k = open?.key;
    open = null;
    await tick();
    if (k) list?.querySelector<HTMLElement>(`[data-key="${CSS.escape(k)}"]`)?.focus();
  }

  function fieldKeys(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      if (e.ctrlKey || e.metaKey) searchOnline();
      else if (q.trim() && rows[0]) openRow(rows[0], true);
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      list?.querySelector<HTMLElement>('button.row')?.focus();
    } else if (e.key === 'Escape' && q && !dialog) { // in the dialog, Esc goes through the host's escape()
      e.preventDefault();
      q = '';
      status = null;
      search('');
    }
  }
  function rowKeys(e: KeyboardEvent) {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    e.preventDefault();
    const all = [...(list?.querySelectorAll<HTMLElement>('button.row') ?? [])];
    const i = all.indexOf(e.currentTarget as HTMLElement);
    if (e.key === 'ArrowDown') all[i + 1]?.focus();
    else if (i > 0) all[i - 1].focus();
    else box?.focus();
  }
  // Escape inside the side panel hands focus back to the open row
  function windowKeys(e: KeyboardEvent) {
    if (e.key === 'Escape' && !dialog && wide && open && panel?.contains(document.activeElement)) {
      e.preventDefault();
      list?.querySelector<HTMLElement>('[aria-current="true"]')?.focus();
    }
  }

  const trimmed = $derived(q.trim());
</script>

<svelte:window onkeydown={windowKeys} />

{#if askEmail}
  <OffEmailDialog onsaved={() => { askEmail = false; searchOnline(); }} oncancel={() => (askEmail = false)} />
{/if}

{#snippet val(x: number | null)}{#if x == null}<span aria-hidden="true">–</span><span class="sr">not reported</span>{:else}{x}{/if}{/snippet}

{#snippet rowItem(r: FoodRow)}
  <li>
    <button
      type="button"
      class="row"
      data-key={r.key}
      aria-current={open?.key === r.key ? 'true' : undefined}
      onclick={() => openRow(r)}
      onkeydown={rowKeys}
    >
      <span class="l1">
        <span class="title" title={r.brand ? `${r.name} · ${r.brand}` : r.name}>{r.name}{#if r.brand}<span class="brand"> · {r.brand}</span>{/if}</span>
        <SourceTag tag={r.tag} />
      </span>
      <span class="l2 fine">
        <span><span class="num">{@render val(r.kcal)}</span> kcal</span>
        <span class="m"><i class="dot p" aria-hidden="true"></i><span class="caps">P</span> <span class="num">{@render val(r.protein)}</span></span>
        <span class="m"><i class="dot c" aria-hidden="true"></i><span class="caps">C</span> <span class="num">{@render val(r.carbs)}</span></span>
        <span class="m"><i class="dot f" aria-hidden="true"></i><span class="caps">F</span> <span class="num">{@render val(r.fat)}</span></span>
        <span>{r.basis}</span>
      </span>
    </button>
  </li>
{/snippet}

{#snippet results()}
  {#if failure}
    <div class="notice">
      <p>Couldn't search the food list: {failure}. Press Retry Search, or restart Tiffin if it keeps happening.</p>
      <button type="button" class="online retry" onclick={() => search(q)}>Retry Search</button>
    </div>
  {:else if dialog && !trimmed}
    {#if loaded}
      {#if planned.length}
        <h2 class="caps mine">Planned <span class="num">{planned.length}</span></h2>
        <ul class="rows">{#each planned as r (r.key)}{@render rowItem(r)}{/each}</ul>
      {/if}
      {#if recent.length}
        <h2 class="caps mine">Recent <span class="num">{recent.length}</span></h2>
        <ul class="rows">{#each recent as r (r.key)}{@render rowItem(r)}{/each}</ul>
      {/if}
      {#if frequent.length}
        <h2 class="caps mine">Frequent <span class="num">{frequent.length}</span></h2>
        <ul class="rows">{#each frequent as r (r.key)}{@render rowItem(r)}{/each}</ul>
      {/if}
      {#if !planned.length && !recent.length && !frequent.length}
        <p class="fine first">Foods you log show up here, so the next one is a click away.</p>
        {#if rows.length}
          <h2 class="caps mine">My Foods <span class="num">{rows.length}</span></h2>
          <ul class="rows">{#each rows as r (r.key)}{@render rowItem(r)}{/each}</ul>
        {/if}
      {/if}
    {/if}
  {:else}
    {#if loaded && !rows.length}
      {#if trimmed}
        <div class="empty">
          <h2>No foods match “{trimmed}”</h2>
          <p>Check the spelling, try a shorter word, or press Search Online for packaged foods. You can also add it yourself.</p>
          {#if onadd}<button type="button" class="add ghost" onclick={() => onadd(trimmed)}>Add “{trimmed}” as a Food</button>{/if}
        </div>
      {:else}
        <div class="empty">
          <h2>Look up any food</h2>
          <p>Search about 18,000 foods stored on this PC, or add one from its label. Nothing is sent anywhere until you press Search Online.</p>
          {#if onadd}<button type="button" class="add primary" onclick={() => onadd()}>Add a Food</button>{/if}
        </div>
      {/if}
    {/if}

    {#if rows.length && !trimmed}
      <div class="minehead">
        <h2 class="caps mine">My Foods <span class="num">{rows.length}</span></h2>
        {#if onadd}<button type="button" class="add primary" onclick={() => onadd()}>Add a Food</button>{/if}
      </div>
    {/if}
    <ul class="rows">{#each rows as r (r.key)}{@render rowItem(r)}{/each}</ul>
    {#if rows.length >= 50 && trimmed}<p class="fine more">Showing the best 50. Type more to narrow it down.</p>{/if}
  {/if}
{/snippet}

<div class="search" class:dlg={dialog}>
  <div class="bar">
    <Icon name="search" size={18} />
    <input
      bind:this={box}
      class="q"
      value={q}
      oninput={typed}
      onkeydown={fieldKeys}
      placeholder={dialog ? 'Search foods and recipes' : 'Search foods'}
      aria-label={dialog ? 'Search foods and recipes' : 'Search foods'}
      autocomplete="off"
      spellcheck="false"
    />
    <button
      type="button"
      class="online"
      aria-busy={searching}
      aria-disabled={!canOnline && !searching}
      title="Search Open Food Facts (Ctrl+Enter)"
      onclick={searchOnline}
    >
      <Icon name="globe" size={16} />
      {searching ? 'Searching…' : cooling ? `Online search ready in ${cooldown} s` : 'Search Online'}
    </button>
  </div>

  <p class="status fine" class:warn={status?.warn && !preparing} role="status" aria-live="polite">
    {#if preparing}
      Getting the food list ready…
    {:else if status}
      {#if status.warn}<Icon name="warn" size={14} />{/if}{#if status.ok}<Icon name="check" size={16} />{/if}{status.text}
    {:else if trimmed}
      Search Online sends only the words you typed, and your contact email, to Open Food Facts.
    {/if}
  </p>

  {#if dialog}
    <div bind:this={bodyEl} class="dbody" class:split>
      {#if split || !open}
        <div bind:this={list} class="results pane">{@render results()}</div>
      {/if}
      {#if split || open}
        <aside bind:this={panel} class="dpanel pane" class:idle={!open} aria-label="Food amounts">
          {#if !split}
            <button type="button" class="btn ghost back" onclick={closeDetail}><Icon name="back" size={16} />Back to Results</button>
          {/if}
          {#if open}
            {#key `${open.key}/${gen}`}
              <FoodDetail bind:this={detail} key={open.key} name={open.name} {us} {onerror} actions={extra} {onenter} />
            {/key}
          {:else}
            <p class="fine">Pick a food to see its amounts.</p>
          {/if}
        </aside>
      {/if}
    </div>
  {:else}
    <div class="body" class:wide>
      <div bind:this={list} class="results">{@render results()}</div>

      {#if wide}
        <aside bind:this={panel} class="panel" class:idle={!open} aria-label="Food amounts">
          {#if open}
            {#key `${open.key}/${gen}`}
              <FoodDetail bind:this={detail} key={open.key} name={open.name} {us} {onerror} actions={extra} {onenter} />
            {/key}
          {:else}
            <p class="fine">Pick a food to see its amounts.</p>
          {/if}
        </aside>
      {:else if open}
        <Dialog wide title={open.name} onclose={closeDetail}>
          {#key `${open.key}/${gen}`}
            <FoodDetail bind:this={detail} key={open.key} name={open.name} {us} {onerror} named={false} actions={extra} {onenter} />
          {/key}
          {#snippet actions()}
            <button type="button" class="btn" onclick={closeDetail}>Close</button>
          {/snippet}
        </Dialog>
      {/if}
    </div>
  {/if}
</div>

<style>
  .search { font-size: 14.5px; line-height: 1.45; color: var(--ink); }
  /* a steady height, so the dialog does not jump while typing; the two panes scroll on their own */
  .search.dlg { display: flex; flex-direction: column; flex: 1 1 480px; min-height: 480px; }
  .dlg .status { margin: 8px 0; }
  .fine { font-size: 13px; line-height: 1.4; color: var(--ink-2); }

  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 48px;
    margin-top: 4px;
    padding: 0 8px 0 16px;
    border-radius: 12px;
    background: var(--card);
    color: var(--ink-3);
    box-shadow: var(--shadow-card), inset 0 0 0 1px var(--rule);
  }
  .bar:focus-within { box-shadow: 0 0 0 3px var(--focus), inset 0 0 0 1px var(--accent); }
  /* input.q out-ranks the dialog's own field styles when this search sits inside a sheet */
  .bar input.q { flex: 1; width: auto; height: 100%; min-width: 0; padding: 0; border: 0; border-radius: 0; outline: none; background: none; box-shadow: none; color: var(--ink); font: inherit; font-size: 15px; -webkit-user-select: text; user-select: text; }
  .bar input.q::placeholder { color: var(--ink-3); }

  /* secondary button: the label changes, the width does not, so the bar never shifts */
  .online {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
    gap: 6px;
    min-width: 212px;
    height: 34px;
    padding: 0 14px;
    border-radius: 9px;
    background: var(--well);
    color: var(--ink);
    box-shadow: inset 0 0 0 1px var(--rule);
    font-size: 13.5px;
    font-weight: 500;
    white-space: nowrap;
  }
  .online:hover { background: var(--rule); }
  .online[aria-disabled='true'] { opacity: 0.55; cursor: default; }
  .online[aria-disabled='true']:hover { background: var(--well); }
  .online.retry { min-width: 0; }

  .status { display: flex; align-items: flex-start; gap: 6px; min-height: 20px; margin: 8px 0 12px; padding: 0 4px; }
  .status.warn { color: var(--warn); }
  .status :global(svg) { flex: none; margin-top: 2px; }

  .body { display: grid; grid-template-columns: minmax(0, 1fr); gap: 24px; align-items: start; }
  .body.wide { grid-template-columns: minmax(0, 1fr) 360px; }

  .dbody { flex: 1; min-height: 0; display: grid; grid-template-columns: minmax(0, 1fr); grid-template-rows: minmax(0, 1fr); gap: 24px; }
  .dbody.split { grid-template-columns: minmax(0, 1fr) 320px; }
  .pane { min-height: 0; overflow-y: auto; }
  .dpanel { padding: 16px; border-radius: 14px; background: var(--card); box-shadow: inset 0 0 0 1px var(--rule); }
  .dpanel.idle { background: var(--well); box-shadow: none; text-align: center; }
  .dpanel.idle p { margin: 0; }
  .back { margin: 0 0 8px -8px; }
  .first { margin: 8px 16px; }
  .results > .mine { margin: 16px 0 8px; padding: 0 16px; }
  .results > .mine:first-child { margin-top: 0; }

  .rows { list-style: none; margin: 0; padding: 0; }
  .row {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 2px;
    width: 100%;
    min-height: 56px;
    padding: 8px 16px;
    border-bottom: 1px solid var(--rule);
    text-align: left;
  }
  .row:hover { background: var(--well); }
  .row[aria-current='true'] { background: var(--accent-wash); }
  .l1 { display: flex; align-items: center; gap: 4px; min-width: 0; }
  .title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
  .brand { font-weight: 400; color: var(--ink-2); }
  .l2 { display: flex; flex-wrap: wrap; align-items: center; gap: 4px 12px; }
  .m { display: inline-flex; align-items: center; gap: 4px; }
  .dot { width: 8px; height: 8px; border-radius: 50%; flex: none; }
  .dot.p { background: var(--protein); }
  .dot.c { background: var(--carbs); }
  .dot.f { background: var(--fat); }
  .more { margin: 12px 16px 0; }
  .minehead { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin: 8px 0; padding: 0 16px; }
  .mine { display: flex; align-items: baseline; gap: 8px; margin: 0; font-size: 14.5px; color: var(--ink-2); }
  .mine .num { color: var(--ink-3); }

  .empty { padding: 32px 16px; text-align: center; color: var(--ink-2); }
  .empty h2 { margin: 0 0 8px; font: 600 21px/1.2 var(--font-display); color: color-mix(in srgb, var(--tile-foods), var(--ink) 35%); }
  .empty p { margin: 0 auto; max-width: 52ch; }
  .add { display: inline-flex; align-items: center; justify-content: center; height: 34px; padding: 0 16px; border-radius: 9px; font-size: 14px; font-weight: 500; }
  .add.primary { background: var(--accent); color: white; box-shadow: inset 0 -1px 0 rgba(0, 0, 0, 0.15); }
  .add.primary:hover { background: var(--accent-press); }
  .add.ghost { color: var(--accent-text); font-weight: 600; }
  .add.ghost:hover { text-decoration: underline; }
  .empty .add { margin-top: 16px; }
  .notice { display: flex; align-items: center; gap: 16px; margin: 0 0 12px; padding: 12px 16px; border-radius: 12px; background: var(--warn-wash); color: var(--warn); }
  .notice p { flex: 1; margin: 0; }

  .panel {
    position: sticky;
    top: 80px;
    max-height: calc(100vh - 96px);
    overflow-y: auto;
    padding: 24px;
    border-radius: 14px;
    background: var(--card);
    box-shadow: inset 0 0 0 1px var(--rule);
  }
  .panel.idle { background: var(--well); box-shadow: none; text-align: center; }
  .panel.idle p { margin: 0; }
</style>
