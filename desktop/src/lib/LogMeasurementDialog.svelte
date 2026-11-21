<script lang="ts">
  import { untrack } from 'svelte';
  import Dialog from './Dialog.svelte';
  import Icon from './Icon.svelte';
  import Segmented from './Segmented.svelte';
  import * as api from './api';
  import type { BodyLogged, BodyRange, BodyView, MeasureOption, UnitIn } from './api';

  // Log Measurement: one value for one day. The crate validates, converts the unit and decides when a weight is worth asking about;
  // the dialog keeps what was typed, shows the question, and sends the same call again with `confirmed` on Save Anyway.
  let { view, today, range, onclose, onlogged, onerror }: {
    view: BodyView;
    today: string;
    range: BodyRange;
    onclose: () => void;
    /** the crate's answer (its `view` is the refreshed page), the measure that was logged, and the day it was logged for */
    onlogged: (r: BodyLogged, measure: MeasureOption, day: string) => void;
    onerror: (m: string) => void;
  } = $props();

  // svelte-ignore state_referenced_locally
  let day = $state(today); // opens on the page's today, then it is the person's to change
  let measureId = $state('weight');
  let value: number | null = $state(null);
  let unit: UnitIn = $state('kg');
  let confirm = $state('');
  let invalid = $state('');
  let failure = $state('');
  let busy = false;
  let box: HTMLInputElement | undefined = $state();

  const measure = $derived(view.measures.find((m) => m.id === measureId) ?? view.measures[0]);
  const units = $derived<UnitIn[]>(measure.kind === 'mass' ? ['kg', 'lb'] : measure.kind === 'length' ? ['cm', 'in'] : []);
  // shortcut: only days inside the page's range are known here, so an older day asks nothing and the crate simply replaces the value, upgrade when the view carries every day
  const replaces = $derived.by(() => {
    const at = view.columns.findIndex((c) => c.id === measureId);
    return at >= 0 && view.rows.some((r) => r.day === day && r.cells[at] != null);
  });

  // the unit picker starts on the person's own unit for the measure picked
  $effect(() => {
    const m = measure;
    unit = m.kind === 'mass' ? (m.unit_text === 'lb' ? 'lb' : 'kg') : m.kind === 'length' ? (m.unit_text === 'in' ? 'in' : 'cm') : 'none';
  });
  $effect(() => { box?.focus(); });

  function edited() {
    confirm = '';
    invalid = '';
  }
  // a new day under an open dialog: an untouched date follows it, and a question asked about the old day is dropped (WR-03)
  let dayTouched = false;
  $effect(() => {
    const t = today;
    untrack(() => {
      if (!dayTouched) day = t;
      edited();
    });
  });

  async function save() {
    if (busy) return;
    if (value == null || Number.isNaN(value)) {
      invalid = 'Enter a number for the value.';
      box?.focus();
      return;
    }
    busy = true;
    failure = '';
    try {
      const r = await api.logMeasure(day, today, measure.id, value, unit, confirm !== '', range);
      if (r.confirm) {
        confirm = r.confirm;
        return;
      }
      onlogged(r, measure, day);
      onclose();
    } catch (e) {
      const why = api.reason(e);
      if (why.startsWith('That ') || why.startsWith('A body measurement')) {
        invalid = `${why}.`;
        box?.focus();
      } else {
        failure = `Couldn't save that change: ${why}. Your body log is unchanged, so try again.`;
        onerror(String(e));
      }
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title="Log Measurement" {onclose}>
  <form id="log-measurement-form" onsubmit={(e) => { e.preventDefault(); save(); }}>
    <label class="f">
      <span>Date</span>
      <input type="date" bind:value={day} oninput={() => { dayTouched = true; edited(); }} max={today} min="1900-01-01" required />
    </label>
    <label class="f">
      <span>Measure</span>
      <select bind:value={measureId} onchange={edited}>
        {#each view.measures as m (m.id)}<option value={m.id}>{m.name}</option>{/each}
      </select>
    </label>
    <div class="f">
      <label for="log-value">Value{measure.unit_text && !units.length ? ` (${measure.unit_text})` : ''}</label>
      <div class="row">
        <input
          id="log-value"
          class="num"
          bind:this={box}
          bind:value
          oninput={edited}
          type="number"
          min="0"
          step="any"
          inputmode="decimal"
          autocomplete="off"
          aria-invalid={invalid ? 'true' : undefined}
          aria-describedby={invalid ? 'log-invalid' : undefined}
        />
        {#if units.length}
          <Segmented options={units} value={unit} onchange={(u) => { unit = u; edited(); }} label="{measure.name} unit">
            {#snippet item(u)}{u}{/snippet}
          </Segmented>
        {/if}
      </div>
    </div>
    {#if invalid}<p id="log-invalid" class="note warn">{invalid}</p>{/if}
    {#if replaces}<p class="fine">Already logged for this day. Saving changes it.</p>{/if}
    {#if confirm}
      <p class="notice" role="alert"><Icon name="warn" size={16} />{confirm}</p>
    {/if}
  </form>
  {#if failure}
    <p class="notice" role="alert"><Icon name="warn" size={16} />{failure}</p>
  {/if}
  {#snippet actions()}
    <button type="button" class="btn" onclick={onclose}>Close Without Saving</button>
    <button type="submit" form="log-measurement-form" class="btn primary">{confirm ? 'Save Anyway' : 'Save Measurement'}</button>
  {/snippet}
</Dialog>

<style>
  form { display: flex; flex-direction: column; gap: 16px; }
  .f { display: flex; flex-direction: column; gap: 4px; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .row { display: flex; align-items: center; gap: 8px; }
  .row input { flex: 1 1 auto; min-width: 0; }
  form .f input.num { font-size: 14.5px; }
  form p.note { margin: 0; font-size: 13px; line-height: 1.4; color: var(--warn); }
  form p.fine { font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .notice { display: flex; align-items: flex-start; gap: 8px; margin: 0; padding: 8px 16px; border-radius: 9px; background: var(--warn-wash); color: var(--warn); }
  .notice :global(svg) { flex: none; margin-top: 4px; }
  input[aria-invalid='true'] { box-shadow: inset 0 0 0 1px var(--warn); }
</style>
