<script lang="ts">
  import Dialog from './Dialog.svelte';
  import Icon from './Icon.svelte';
  import * as api from './api';
  import type { BodyLogged, BodyRange, BodyRow, BodyView, DayValue, UnitIn } from './api';

  // Edit Measurements: every shown measure of one day in one save with one undo. Each box starts from the stored number
  // (BodyRow.values, never the rounded cell text, so saving never rounds what was logged). A cleared box removes that value.
  // Only the boxes that changed are sent. The weight goes through the same ask-once check as Log Weight.
  let { view, row, dateText, today, range, onclose, onsaved, onerror }: {
    view: BodyView;
    row: BodyRow;
    dateText: string;
    today: string;
    range: BodyRange;
    onclose: () => void;
    onsaved: (r: BodyLogged, day: string) => void;
    onerror: (m: string) => void;
  } = $props();

  const UNITS: UnitIn[] = ['kg', 'lb', 'cm', 'in'];
  // svelte-ignore state_referenced_locally
  const boxes: { id: string; name: string; unit: UnitIn; text: string; before: number | null; value: number | null }[] = $state(
    view.columns.map((c, i) => {
      const have = row.values[i] ?? null;
      return {
        id: c.id,
        name: c.name,
        unit: have?.unit ?? (UNITS.find((u) => u === c.unit_text) ?? 'none'),
        text: c.unit_text,
        before: have?.value ?? null,
        value: have?.value ?? null,
      };
    }),
  );
  let confirm = $state('');
  let invalid = $state('');
  let failure = $state('');
  let busy = false;

  function edited() {
    confirm = '';
    invalid = '';
  }

  async function save() {
    if (busy) return;
    // an emptied number box reads as null; NaN can't come from a number input, but never send it
    const values: DayValue[] = boxes
      .filter((b) => (b.value == null || Number.isNaN(b.value) ? null : b.value) !== b.before)
      .map((b) => ({ measure: b.id, value: b.value == null || Number.isNaN(b.value) ? null : b.value, unit: b.unit }));
    if (!values.length) {
      onclose();
      return;
    }
    busy = true;
    failure = '';
    try {
      const r = await api.editMeasurements(row.day, today, values, confirm !== '', range);
      if (r.confirm) {
        confirm = r.confirm;
        return;
      }
      onsaved(r, row.day);
      onclose();
    } catch (e) {
      const why = api.reason(e);
      if (why.startsWith('That ') || why.startsWith('A body measurement')) {
        invalid = `${why}.`;
      } else {
        failure = `Couldn't save that change: ${why}. Your body log is unchanged, so try again.`;
        onerror(String(e));
      }
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title="Measurements for {dateText}" wide {onclose}>
  <form id="edit-measurements-form" onsubmit={(e) => { e.preventDefault(); save(); }}>
    <p class="fine">Clear a box and save to remove that value.</p>
    <div class="grid">
      {#each boxes as b, i (b.id)}
        <div class="f">
          <label for="em-{i}" title={b.name}>{b.name}{#if b.text}<span class="unit">&nbsp;{b.text}</span>{/if}</label>
          <input
            id="em-{i}"
            class="num"
            bind:value={b.value}
            oninput={edited}
            type="number"
            min="0"
            step="any"
            inputmode="decimal"
            autocomplete="off"
          />
        </div>
      {/each}
    </div>
    {#if invalid}<p class="note" role="alert">{invalid}</p>{/if}
    {#if confirm}<p class="notice" role="alert"><Icon name="warn" size={16} />{confirm}</p>{/if}
  </form>
  {#if failure}
    <p class="notice" role="alert"><Icon name="warn" size={16} />{failure}</p>
  {/if}
  {#snippet actions()}
    <button type="button" class="btn" onclick={onclose}>Discard Edits</button>
    <button type="submit" form="edit-measurements-form" class="btn primary">{confirm ? 'Save Anyway' : 'Save Changes'}</button>
  {/snippet}
</Dialog>

<style>
  form { display: flex; flex-direction: column; gap: 16px; }
  p { margin: 0; }
  .fine { font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 16px; }
  .f { display: flex; flex-direction: column; gap: 4px; min-width: 0; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .f label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .unit { color: var(--ink-3); }
  form .f input.num { font-size: 14.5px; }
  .note { font-size: 13px; line-height: 1.4; color: var(--warn); }
  .notice { display: flex; align-items: flex-start; gap: 8px; padding: 8px 16px; border-radius: 9px; background: var(--warn-wash); color: var(--warn); }
  .notice :global(svg) { flex: none; margin-top: 4px; }
</style>
