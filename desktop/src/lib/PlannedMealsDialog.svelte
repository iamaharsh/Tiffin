<script lang="ts">
  import Dialog from './Dialog.svelte';
  import Icon from './Icon.svelte';
  import * as api from './api';
  import type { DayView, PlannedMeal } from './api';

  // Log Today's Planned Meals: each planned meal goes into the slot it was planned for, in one undoable batch.
  let { today, onclose, onlogged, onerror }: {
    today: string;
    /** Esc, Close Without Logging and Back to Diary: nothing is logged until the primary is pressed */
    onclose: () => void;
    onlogged: (v: DayView, n: number) => void;
    onerror: (m: string) => void;
  } = $props();

  type Row = { m: PlannedMeal; on: boolean; servings: number | null };
  let rows = $state<Row[] | null>(null);
  let failure = $state('');
  let busy = false;

  const count = $derived(rows?.filter((r) => r.on).length ?? 0);

  (async () => {
    try {
      const list = await api.plannedMeals(today);
      rows = list.map((m) => ({ m, on: m.loggable, servings: 1 }));
    } catch (e) {
      onerror(String(e));
      onclose();
    }
  })();

  async function log() {
    if (busy || !rows || count === 0) return; // aria-disabled, not disabled, so focus stays on the button
    busy = true;
    failure = '';
    try {
      const picks = rows.filter((r) => r.on).map((r) => ({ meal_id: r.m.meal_id, servings: r.servings ?? 0 }));
      const r = await api.logPlanned(today, picks);
      onlogged(r.view, r.count);
      onclose();
    } catch (e) {
      failure = `Couldn't save that change: ${api.reason(e)}. Your diary is unchanged, so try again.`;
    } finally {
      busy = false;
    }
  }
</script>

<Dialog wide title="Log Today's Planned Meals" {onclose}>
  {#if rows && rows.length}
    <p class="note">Each meal goes into the slot you planned it for.</p>
    <div class="list">
      <div class="row cols"><span></span><span></span><span></span><span class="note">Servings</span></div>
      {#each rows as r (r.m.meal_id)}
        <div class="row">
          <input type="checkbox" bind:checked={r.on} disabled={!r.m.loggable} aria-label="Log {r.m.name}" />
          <span class="what">
            <span class="name" title={r.m.name}>{r.m.name}</span>
            {#if r.m.note}
              <span class="note dim">{r.m.note}</span>
            {:else if r.m.kcal != null}
              <span class="note"><span class="num">{r.m.kcal}</span> kcal per serving</span>
            {/if}
          </span>
          <span class="tag caps">{r.m.slot_label}</span>
          <input class="serv num" type="number" min="0" step="any" inputmode="decimal" bind:value={r.servings} disabled={!r.m.loggable} aria-label="Servings of {r.m.name}" />
        </div>
      {/each}
    </div>
  {:else if rows}
    <p class="note">Nothing is planned for today.</p>
  {/if}
  {#if failure}
    <p class="notice" role="alert"><Icon name="warn" size={16} />{failure}</p>
  {/if}
  {#snippet actions()}
    {#if rows && !rows.length}
      <button type="button" class="btn" onclick={onclose}>Back to Diary</button>
    {:else}
      <button type="button" class="btn" onclick={onclose}>Close Without Logging</button>
      <button type="button" class="btn primary" aria-disabled={count === 0 ? 'true' : undefined} onclick={log}>
        {count === 0 ? 'Log Meals' : count === 1 ? 'Log 1 Meal' : `Log ${count} Meals`}
      </button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .note { margin: 0; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .note.dim { color: var(--ink-3); }
  .list { display: flex; flex-direction: column; }
  .row { display: grid; grid-template-columns: auto minmax(0, 1fr) auto 80px; align-items: center; gap: 16px; min-height: 56px; padding: 8px 0; border-bottom: 1px solid var(--rule); }
  .row.cols { min-height: 0; padding: 0 0 4px; }
  .what { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 14.5px; font-weight: 600; color: var(--ink); }
  .tag { display: inline-flex; align-items: center; height: 20px; padding: 0 8px; border-radius: 5px; background: var(--well); color: var(--ink-2); font-size: 13px; white-space: nowrap; }
  .row .serv { font-size: 14.5px; font-weight: 600; }
  .btn[aria-disabled='true'] { opacity: 0.5; }
  .notice { display: flex; align-items: flex-start; gap: 8px; margin: 0; padding: 8px 16px; border-radius: 9px; background: var(--warn-wash); color: var(--warn); }
  .notice :global(svg) { flex: none; margin-top: 4px; }
</style>
