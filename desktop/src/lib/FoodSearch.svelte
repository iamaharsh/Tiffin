<script lang="ts">
  import { tick, untrack, type Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  import Dialog from './Dialog.svelte';
  import FoodDetail from './FoodDetail.svelte';
  import OffEmailDialog from './OffEmailDialog.svelte';
  import SourceTag from './SourceTag.svelte';
  import * as api from './api';
  import type { FoodDetail as Info, FoodRow, OnlineResult } from './api';

  // The one food search: local results on every keystroke, Open Food Facts only when asked.
  // A standalone page now; Phase 3 reuses it inside the diary's add-food dialog.
  let { us, onerror, onadd, actions: extra }: { us: boolean; onerror: (m: string) => void; onadd?: (name?: string) => void; actions?: Snippet<[Info]> } = $props();

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
  let status: { text: string; warn: boolean } | null = $state(null);
  let open: { key: string; name: string } | null = $state(null);
  let box: HTMLInputElement | undefined = $state();
  let list: HTMLUListElement | undefined = $state();
  let panel: HTMLElement | undefined = $state();
  let detail: { focusAmount: () => void } | undefined = $state();
  let seq = 0;
  let gen = $state(0);

  // two columns from 1100 px wide; below that the detail opens as a sheet
  const wideQuery = window.matchMedia('(min-width: 1100px)');
  let wide = $state(wideQuery.matches);
  $effect(() => {
    const f = () => (wide = wideQuery.matches);
    wideQuery.addEventListener('change', f);
    return () => wideQuery.removeEventListener('change', f);
  });

  async function search(text: string) {
    const mine = ++seq; // a slower older answer never replaces a newer one
    const timer = setTimeout(() => { if (mine === seq) preparing = true; }, 300);
    try {
      const got = await api.searchFoods(text);
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
    } else if (e.key === 'Escape' && q) {
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
    if (e.key === 'Escape' && wide && open && panel?.contains(document.activeElement)) {
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

<div class="search">
  <div class="bar">
    <Icon name="search" size={18} />
    <input
      bind:this={box}
      value={q}
      oninput={typed}
      onkeydown={fieldKeys}
      placeholder="Search foods"
      aria-label="Search foods"
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
      {#if status.warn}<Icon name="warn" size={14} />{/if}{status.text}
    {:else if trimmed}
      Search Online sends only the words you typed, and your contact email, to Open Food Facts.
    {/if}
  </p>

  <div class="body" class:wide>
    <div class="results">
      {#if failure}
        <div class="notice">
          <p>Couldn't search the food list: {failure}. Press Retry Search, or restart Tiffin if it keeps happening.</p>
          <button type="button" class="online retry" onclick={() => search(q)}>Retry Search</button>
        </div>
      {:else if loaded && !rows.length}
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
      <ul bind:this={list} class="rows">
        {#each rows as r (r.key)}
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
        {/each}
      </ul>
      {#if rows.length >= 50}<p class="fine more">Showing the best 50. Type more to narrow it down.</p>{/if}
    </div>

    {#if wide}
      <aside bind:this={panel} class="panel" class:idle={!open} aria-label="Food amounts">
        {#if open}
          {#key `${open.key}/${gen}`}
            <FoodDetail bind:this={detail} key={open.key} {us} {onerror} actions={extra} />
          {/key}
        {:else}
          <p class="fine">Pick a food to see its amounts.</p>
        {/if}
      </aside>
    {:else if open}
      <Dialog wide title={open.name} onclose={closeDetail}>
        {#key `${open.key}/${gen}`}
          <FoodDetail bind:this={detail} key={open.key} {us} {onerror} named={false} actions={extra} />
        {/key}
        {#snippet actions()}
          <button type="button" class="btn" onclick={closeDetail}>Close</button>
        {/snippet}
      </Dialog>
    {/if}
  </div>
</div>

<style>
  .search { font-size: 14.5px; line-height: 1.45; }
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
  .bar input { flex: 1; min-width: 0; border: 0; outline: none; background: none; color: var(--ink); font: inherit; font-size: 15px; -webkit-user-select: text; user-select: text; }
  .bar input::placeholder { color: var(--ink-3); }

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
