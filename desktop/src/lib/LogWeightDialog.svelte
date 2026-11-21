<script lang="ts">
  import Dialog from './Dialog.svelte';
  import Icon from './Icon.svelte';
  import Segmented from './Segmented.svelte';
  import * as api from './api';
  import type { Drift, UnitIn } from './api';

  // Log Weight: one weigh-in for the day the Diary is showing. The crate validates, converts and decides when a weight is worth
  // asking about; when the new trend has drifted from the target's weight the dialog stays open on the crate's offer.
  // Update Target is the only thing here that moves the target: Not Now and Esc never call it.
  let { day, today, dateText, onclose, onlogged, onupdated, onerror }: {
    day: string;
    today: string;
    dateText: string;
    onclose: () => void;
    /** a weigh-in was saved and the person did not take the target offer (or there was none): the page offers its Undo */
    onlogged: (text: string) => void;
    /** the target was updated: a message for the toast, no Undo */
    onupdated: (message: string) => void;
    onerror: (m: string) => void;
  } = $props();

  let value: number | null = $state(null);
  let unit: UnitIn = $state('kg');
  let replaces = $state(false);
  let readOnly = $state('');
  let confirm = $state('');
  let invalid = $state('');
  let failure = $state('');
  let saved: { text: string; drift: Drift } | null = $state(null);
  let busy = false;
  let box: HTMLInputElement | undefined = $state();
  let notNowBtn: HTMLButtonElement | undefined = $state();

  // the person's own unit and the day's existing weight (exact, never the rounded table text), read once on open
  $effect(() => {
    api.bodyView('all', today).then((v) => {
      readOnly = v.read_only ?? '';
      const w = v.measures.find((m) => m.id === 'weight');
      if (w) unit = w.unit_text === 'lb' ? 'lb' : 'kg';
      const at = v.columns.findIndex((c) => c.id === 'weight');
      const have = at >= 0 ? v.rows.find((r) => r.day === day)?.values[at] : null;
      if (have && value == null) {
        value = have.value;
        unit = have.unit;
        replaces = true;
      }
    }).catch((e) => { failure = `Couldn't read your weight log: ${api.reason(e)}.`; });
  });
  $effect(() => { box?.focus(); });
  // the form is replaced by the result step, so focus would fall to nowhere: it lands on Not Now (the safe choice), which reads the result out
  $effect(() => { if (saved) notNowBtn?.focus(); });

  function edited() {
    confirm = '';
    invalid = '';
  }

  async function save() {
    if (busy) return;
    if (readOnly) return;
    if (value == null || Number.isNaN(value)) {
      invalid = 'Enter a number for the weight.';
      box?.focus();
      return;
    }
    busy = true;
    failure = '';
    try {
      const r = await api.logMeasure(day, today, 'weight', value, unit, confirm !== '', null);
      if (r.confirm) {
        confirm = r.confirm;
        return;
      }
      if (r.drift) {
        saved = { text: r.text, drift: r.drift };
        return;
      }
      onlogged(r.text);
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

  function notNow() {
    if (saved) onlogged(saved.text);
    onclose();
  }

  async function updateTarget() {
    if (busy) return;
    busy = true;
    failure = '';
    try {
      const v = await api.acceptTargetUpdate(today);
      onupdated(`Target updated to ${v.result.kcal_text} kcal from today.`);
      onclose();
    } catch (e) {
      failure = `Couldn't save that change: ${api.reason(e)}. Your target is unchanged, so try again.`;
      onerror(String(e));
    } finally {
      busy = false;
    }
  }
</script>

<!-- Esc and a click outside are Not Now once the weight is saved: the Undo for the weigh-in is still owed -->
<Dialog title="Log Weight" onclose={saved ? notNow : onclose}>
  {#if saved}
    <p id="lw-saved" class="saved"><span class="ok"><Icon name="check" size={16} /></span>Saved {saved.text} for {day === today ? 'today' : dateText}.</p>
    <p id="lw-drift" class="drift">{saved.drift.text}</p>
  {:else}
    <form id="log-weight-form" onsubmit={(e) => { e.preventDefault(); save(); }}>
      <p class="fine">For {day === today ? 'today' : dateText}</p>
      <div class="f">
        <label for="weight-value">Weight</label>
        <div class="row">
          <input
            id="weight-value"
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
            aria-describedby={invalid ? 'weight-invalid' : undefined}
          />
          <Segmented options={['kg', 'lb'] as UnitIn[]} value={unit} onchange={(u) => { unit = u; edited(); }} label="Weight unit">
            {#snippet item(u)}{u}{/snippet}
          </Segmented>
        </div>
      </div>
      {#if invalid}<p id="weight-invalid" class="note">{invalid}</p>{/if}
      {#if replaces}<p class="fine">Already logged for this day. Saving changes it.</p>{/if}
      {#if readOnly}<p class="notice" role="alert"><Icon name="warn" size={16} />{readOnly}</p>{/if}
      {#if confirm}<p class="notice" role="alert"><Icon name="warn" size={16} />{confirm}</p>{/if}
    </form>
  {/if}
  {#if failure}
    <p class="notice" role="alert"><Icon name="warn" size={16} />{failure}</p>
  {/if}
  {#snippet actions()}
    {#if saved}
      <button type="button" class="btn" bind:this={notNowBtn} aria-describedby="lw-saved lw-drift" onclick={notNow}>Not Now</button>
      <button type="button" class="btn primary" onclick={updateTarget}>Update Target</button>
    {:else}
      <button type="button" class="btn" onclick={onclose}>Close Without Saving</button>
      <button type="submit" form="log-weight-form" class="btn primary" aria-disabled={readOnly ? 'true' : undefined} title={readOnly || undefined}>{confirm ? 'Save Anyway' : 'Save Weight'}</button>
    {/if}
  {/snippet}
</Dialog>

<style>
  form { display: flex; flex-direction: column; gap: 16px; }
  .f { display: flex; flex-direction: column; gap: 4px; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .row { display: flex; align-items: center; gap: 8px; }
  .row input { flex: 1 1 auto; min-width: 0; }
  form .f input.num { font-size: 14.5px; }
  p { margin: 0; }
  .fine { font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .note { font-size: 13px; line-height: 1.4; color: var(--warn); }
  .saved { display: flex; align-items: center; gap: 8px; font-size: 14.5px; color: var(--ink); }
  .ok { display: inline-flex; color: var(--ok); }
  .drift { padding: 16px; border-radius: 14px; background: var(--well); font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .notice { display: flex; align-items: flex-start; gap: 8px; padding: 8px 16px; border-radius: 9px; background: var(--warn-wash); color: var(--warn); }
  .notice :global(svg) { flex: none; margin-top: 4px; }
  input[aria-invalid='true'] { box-shadow: inset 0 0 0 1px var(--warn); }
  .btn[aria-disabled='true'] { opacity: 0.5; }
</style>
