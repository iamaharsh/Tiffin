<script lang="ts">
  import { tick, untrack } from 'svelte';
  import Dialog from './Dialog.svelte';
  import Icon from './Icon.svelte';
  import SourceTag from './SourceTag.svelte';
  import NewActivityDialog from './NewActivityDialog.svelte';
  import * as api from './api';
  import type { Activities, ActivityRef, ActivityRow, CustomActivity, DayView, Estimate } from './api';

  // Add Exercise: search the activities (the table is compiled in, so there is nothing to wait for), pick one, give the minutes.
  // The crate returns every figure and sentence; this dialog prints them. The dialog stays open, so several workouts go in during one visit.
  let { day, today, onclose, onlogged, onfailed, onerror }: {
    day: string;
    today: string;
    onclose: () => void;
    /** the day as the crate now holds it, so the Diary behind the dialog updates at once */
    onlogged: (v: DayView, name: string) => void;
    /** a write failed: the page reloads the day from the crate */
    onfailed?: () => void;
    onerror: (m: string) => void;
  } = $props();

  const NET_LABEL = "Calories are net: what the activity burns above resting, so your daily burn isn't counted twice.";

  let q = $state('');
  let acts: Activities = $state({ recent: [], mine: [], common: [], rows: [] });
  let loaded = $state(false);
  let searchFailure = $state('');
  let open: ActivityRow | null = $state(null);
  let minutes: number | null = $state(null);
  let own: number | null = $state(null);
  let est: Estimate | null = $state(null);
  let estError = $state('');
  let status: { text: string } | null = $state(null);
  let failure = $state('');
  let box: HTMLInputElement | undefined = $state();
  let durBox: HTMLInputElement | undefined = $state();
  let list: HTMLElement | undefined = $state();
  let bodyEl: HTMLElement | undefined = $state();
  let bodyW = $state(0);
  /** the New Activity / Edit Activity dialog, nested over this one */
  let editingActivity: { activity: CustomActivity | null } | null = $state(null);
  let busy = false;
  let seq = 0;
  let eseq = 0;
  /** Recent as it was when the dialog opened: it does not reorder under the pointer after an add */
  let recentSnap: ActivityRow[] | null = null;

  // measure the body itself, never the window: two panes at 680 px and up, stacked below
  const split = $derived(bodyW >= 680);
  $effect(() => {
    if (!bodyEl) return;
    bodyW = bodyEl.clientWidth;
    const ro = new ResizeObserver(([e]) => (bodyW = e.contentRect.width));
    ro.observe(bodyEl);
    return () => ro.disconnect();
  });

  const kid = (k: ActivityRef) => JSON.stringify(k);
  const trimmed = $derived(q.trim());

  async function search(text: string) {
    const mine = ++seq; // a slower older answer never replaces a newer one
    try {
      const a = await api.exerciseActivities(text, today);
      if (mine !== seq) return;
      if (!text.trim()) {
        recentSnap ??= a.recent;
        acts = { ...a, recent: recentSnap };
      } else {
        acts = a;
      }
      loaded = true;
      searchFailure = '';
    } catch (e) {
      if (mine === seq) searchFailure = api.reason(e);
      onerror(String(e));
    }
  }
  $effect(() => {
    box?.focus();
    untrack(() => search(''));
  });

  /** Read the lists again, taking a fresh Recent, and open `pick` when given (after New Activity saves). */
  export async function refresh(pick?: ActivityRef) {
    recentSnap = null;
    q = '';
    status = null;
    await search('');
    if (pick) {
      const row = [...acts.mine, ...acts.recent].find((r) => kid(r.key) === kid(pick));
      if (row) await choose(row);
    }
  }

  function typed(e: Event & { currentTarget: HTMLInputElement }) {
    q = e.currentTarget.value;
    status = null;
    search(q);
  }

  async function choose(r: ActivityRow) {
    open = r;
    failure = '';
    await tick();
    durBox?.focus();
  }

  /** the estimate is asked again for each duration, and never for a blank, zero or negative one */
  async function askEstimate() {
    const mine = ++eseq;
    if (!open || minutes == null || !(minutes > 0)) {
      est = null;
      estError = '';
      return;
    }
    try {
      const e = await api.exerciseEstimate(day, open.key, minutes);
      if (mine === eseq) { est = e; estError = ''; }
    } catch (e) {
      if (mine === eseq) { est = null; estError = `${api.reason(e)}.`; }
    }
  }
  $effect(() => {
    open; minutes; day;
    untrack(askEstimate);
  });

  const ready = $derived(open != null && minutes != null && minutes > 0);

  async function add() {
    if (busy || !open || minutes == null || !(minutes > 0)) return;
    busy = true;
    failure = '';
    try {
      const r = await api.logExercise(day, today, open.key, minutes, own);
      onlogged(r.view, r.name);
      // the next activity starts from a clean, focused search; the status line says what went in
      q = '';
      open = null;
      minutes = null;
      own = null;
      est = null;
      status = { text: `Added ${r.name}` };
      search('');
      await tick();
      box?.focus();
    } catch (e) {
      // the toast sits behind a modal, so the notice is inline; what was typed stays
      failure = `Couldn't save that change: ${api.reason(e)}. Your diary is unchanged, so try again.`;
      onfailed?.();
    } finally {
      busy = false;
    }
  }

  async function closeDetail() {
    const k = open ? kid(open.key) : null;
    open = null;
    await tick();
    if (k) list?.querySelector<HTMLElement>(`[data-key="${CSS.escape(k)}"]`)?.focus();
  }
  /** Esc: typed text first, then back to the list when stacked, then close */
  function cancel() {
    if (q) {
      q = '';
      status = null;
      search('');
      box?.focus();
    } else if (!split && open) closeDetail();
    else onclose();
  }

  function fieldKeys(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      if (trimmed && acts.rows[0]) choose(acts.rows[0]);
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      list?.querySelector<HTMLElement>('button.row')?.focus();
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
  function enterAdds(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      add();
    }
  }
</script>

{#snippet rowItem(r: ActivityRow)}
  <li class="ri">
    <button type="button" class="row" data-key={kid(r.key)} aria-current={open && kid(open.key) === kid(r.key) ? 'true' : undefined} onclick={() => choose(r)} onkeydown={rowKeys}>
      <span class="l1">
        <span class="title" title={r.text}>{r.text}</span>
        {#if r.custom}<SourceTag tag="Custom" />{/if}
      </span>
      <span class="l2 fine"><span>{r.heading}</span><span class="num">{r.met_text}</span></span>
    </button>
    {#if r.custom && r.basis && 'custom' in r.key}
      {@const uid = r.key.custom}
      <button type="button" class="pencil" aria-label="Edit {r.text}" title="Edit {r.text}" onclick={() => { if (r.basis) editingActivity = { activity: { uid, name: r.text, basis: r.basis } }; }}><Icon name="pencil" size={16} /></button>
    {/if}
  </li>
{/snippet}

{#snippet results()}
  {#if searchFailure}
    <div class="notice">
      <p>Couldn't search the activity list: {searchFailure}. Press Retry Search, or restart Tiffin if it keeps happening.</p>
      <button type="button" class="btn ghost" onclick={() => search(q)}>Retry Search</button>
    </div>
  {:else if loaded && trimmed}
    {#if acts.rows.length}
      <ul class="rows">{#each acts.rows as r (kid(r.key))}{@render rowItem(r)}{/each}</ul>
    {:else}
      <p class="fine none">No activities match. Try a shorter word, or add your own with New Activity…</p>
    {/if}
  {:else if loaded}
    {#if acts.recent.length}
      <h3 class="caps head">Recent <span class="num">{acts.recent.length}</span></h3>
      <ul class="rows">{#each acts.recent as r (kid(r.key))}{@render rowItem(r)}{/each}</ul>
    {/if}
    {#if acts.mine.length}
      <h3 class="caps head">My Activities <span class="num">{acts.mine.length}</span></h3>
      <ul class="rows">{#each acts.mine as r (kid(r.key))}{@render rowItem(r)}{/each}</ul>
    {/if}
    {#if acts.common.length}
      <h3 class="caps head">Common Activities</h3>
      {#each acts.common as g (g.heading)}
        <h4 class="caps sub">{g.heading}</h4>
        <ul class="rows">{#each g.rows as r (kid(r.key))}{@render rowItem(r)}{/each}</ul>
      {/each}
    {:else if !acts.recent.length && !acts.mine.length}
      <p class="fine none">No activities are listed yet. Type to search, or add your own with New Activity…</p>
    {/if}
  {/if}
{/snippet}

<!-- Esc undoes typed text, then a stacked detail, and only then closes; a click outside and Done Adding close at once -->
<Dialog large title="Add Exercise" {onclose} oncancel={cancel}>
  <div class="search">
    <div class="bar">
      <Icon name="search" size={18} />
      <input bind:this={box} class="q" value={q} oninput={typed} onkeydown={fieldKeys} placeholder="Search activities" aria-label="Search activities" autocomplete="off" spellcheck="false" />
    </div>
    <p class="status fine" role="status" aria-live="polite">{#if status}<Icon name="check" size={16} />{status.text}{/if}</p>

    <div bind:this={bodyEl} class="dbody" class:split>
      {#if split || !open}
        <div bind:this={list} class="results pane">{@render results()}</div>
      {/if}
      {#if split || open}
        <aside class="dpanel pane" class:idle={!open} aria-label="Activity details">
          {#if !split}
            <button type="button" class="btn ghost back" onclick={closeDetail}><Icon name="back" size={16} />Back to Results</button>
          {/if}
          {#if open}
            <div class="detail">
              <p class="dname" title={open.text}>{open.text}</p>
              <p class="fine">{open.custom ? 'Your own activity' : 'From the 2024 Adult Compendium'}</p>
              <label class="f">
                <span>Duration</span>
                <span class="dur">
                  <input bind:this={durBox} bind:value={minutes} onkeydown={enterAdds} type="number" min="0.01" step="any" inputmode="decimal" autocomplete="off" />
                  <span class="fine">minutes</span>
                </span>
              </label>
              {#if est}
                <p class="est"><span class="num">{est.kcal_text}</span>{#if est.kcal != null}<span class="fine"> kcal</span>{/if}</p>
                {#if est.working}<p class="fine">{est.working}</p>{/if}
                {#if est.weight_text}<p class="fine">{est.weight_text}</p>{/if}
                {#if est.note}<p class="fine warn"><Icon name="warn" size={14} />{est.note}</p>{/if}
              {:else if estError}
                <p class="fine warn"><Icon name="warn" size={14} />{estError}</p>
              {/if}
              <p class="fine">{NET_LABEL}</p>
              <label class="f">
                <span>Calories burned (optional)</span>
                <input bind:value={own} onkeydown={enterAdds} type="number" min="0" step="any" inputmode="decimal" autocomplete="off" placeholder={est?.kcal != null ? est.kcal_text : ''} />
                <span class="fine">Leave empty to use the estimate.</span>
              </label>
            </div>
          {:else}
            <p class="fine">Pick an activity to see the calories.</p>
          {/if}
        </aside>
      {/if}
    </div>
  </div>
  {#if failure}
    <p class="notice" role="alert"><Icon name="warn" size={16} />{failure}</p>
  {/if}
  {#snippet actions()}
    <button type="button" class="btn newact" onclick={() => (editingActivity = { activity: null })}>New Activity…</button>
    <button type="button" class="btn" onclick={onclose}>Done Adding</button>
    <button type="button" class="btn primary" aria-disabled={ready ? undefined : 'true'} onclick={add}>Add Exercise</button>
  {/snippet}
</Dialog>

{#if editingActivity}
  <NewActivityDialog
    activity={editingActivity.activity}
    onclose={() => { editingActivity = null; queueMicrotask(() => box?.focus()); }}
    onsaved={(uid) => { editingActivity = null; refresh({ custom: uid }); }}
    ondeleted={async () => {
      const gone = editingActivity?.activity?.uid;
      editingActivity = null;
      if (open && 'custom' in open.key && open.key.custom === gone) open = null;
      await refresh();
      box?.focus();
    }}
  />
{/if}

<style>
  .search { display: flex; flex-direction: column; flex: 1 1 480px; min-height: 480px; font-size: 14.5px; line-height: 1.45; color: var(--ink); }
  .fine { font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .bar { display: flex; align-items: center; gap: 8px; height: 48px; margin-top: 4px; padding: 0 8px 0 16px; border-radius: 12px; background: var(--card); color: var(--ink-3); box-shadow: var(--shadow-card), inset 0 0 0 1px var(--rule); }
  .bar:focus-within { box-shadow: 0 0 0 3px var(--focus), inset 0 0 0 1px var(--accent); }
  /* input.q out-ranks the dialog's own field styles when this search sits inside a sheet */
  .bar input.q { flex: 1; width: auto; height: 100%; min-width: 0; padding: 0; border: 0; border-radius: 0; outline: none; background: none; box-shadow: none; color: var(--ink); font: inherit; font-size: 14.5px; -webkit-user-select: text; user-select: text; }
  .bar input.q::placeholder { color: var(--ink-3); }
  .status { display: flex; align-items: flex-start; gap: 4px; min-height: 24px; margin: 8px 0; padding: 0 4px; }
  .status :global(svg) { flex: none; margin-top: 2px; color: var(--ok); }

  .dbody { flex: 1; min-height: 0; display: grid; grid-template-columns: minmax(0, 1fr); grid-template-rows: minmax(0, 1fr); gap: 24px; }
  .dbody.split { grid-template-columns: minmax(0, 1fr) 320px; }
  .pane { min-height: 0; overflow-y: auto; }
  .dpanel { padding: 16px; border-radius: 14px; background: var(--card); box-shadow: inset 0 0 0 1px var(--rule); }
  .dpanel.idle { background: var(--well); box-shadow: none; text-align: center; }
  .dpanel.idle p { margin: 0; color: var(--ink-3); }
  .back { margin: 0 0 8px -8px; }

  .head { display: flex; align-items: baseline; gap: 8px; margin: 16px 0 8px; padding: 0 16px; font-size: 14.5px; color: var(--ink-2); }
  .head:first-child { margin-top: 0; }
  .head .num { color: var(--ink-3); }
  .sub { margin: 8px 0 4px; padding: 0 16px; font-size: 13px; color: var(--ink-3); }
  .none { margin: 8px 16px; }
  .rows { list-style: none; margin: 0; padding: 0; }
  .ri { display: flex; align-items: center; border-bottom: 1px solid var(--rule); }
  .ri:hover { background: var(--well); }
  .pencil { display: grid; place-items: center; flex: none; width: 32px; height: 32px; margin-right: 8px; border-radius: 9px; color: var(--accent-text); }
  .pencil:hover { background: var(--accent-wash); }
  .newact { margin-right: auto; }
  .row { flex: 1; min-width: 0; display: flex; flex-direction: column; justify-content: center; gap: 4px; width: 100%; min-height: 56px; padding: 8px 16px; text-align: left; }
  .ri:has(.row[aria-current='true']) { background: var(--accent-wash); }
  .l1 { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
  .l2 { display: flex; align-items: center; justify-content: space-between; gap: 16px; }

  .detail { display: flex; flex-direction: column; gap: 16px; }
  .detail p { margin: 0; }
  .dname { font-weight: 600; color: var(--ink); overflow-wrap: anywhere; }
  .f { display: flex; flex-direction: column; gap: 4px; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .f input { font-size: 14.5px; }
  .dur { display: flex; align-items: center; gap: 8px; }
  .dur input { width: 96px; }
  .est { font-size: 26px; line-height: 1.1; color: var(--ink); }
  .est .fine { font-size: 13px; }
  .warn { display: flex; align-items: flex-start; gap: 4px; color: var(--warn); }
  .warn :global(svg) { flex: none; margin-top: 2px; }

  .btn[aria-disabled='true'] { opacity: 0.5; }
  .notice { display: flex; align-items: flex-start; gap: 8px; margin: 0; padding: 8px 16px; border-radius: 9px; background: var(--warn-wash); color: var(--warn); }
  .notice p { flex: 1; margin: 0; }
  .notice :global(svg) { flex: none; margin-top: 4px; }
</style>
