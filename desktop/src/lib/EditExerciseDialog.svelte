<script lang="ts">
  import Dialog from './Dialog.svelte';
  import Icon from './Icon.svelte';
  import SourceTag from './SourceTag.svelte';
  import * as api from './api';
  import type { DayView, ExerciseView } from './api';

  // Edit Exercise: the duration and the calories of one logged workout. The estimate is worked again by the crate from the
  // MET and weight kept at log time; the person's own number stays until Use estimate. The page does no calorie maths.
  let { day, today, entry, onclose, onsaved, ondeleted, onfailed }: {
    day: string;
    today: string;
    entry: ExerciseView;
    /** Esc, a click outside and Discard Edits: nothing was saved, nothing is asked */
    onclose: () => void;
    onsaved: (v: DayView, message: string) => void;
    ondeleted: (v: DayView, name: string) => void;
    /** a write failed: the page reloads the day from the crate */
    onfailed?: () => void;
  } = $props();

  // svelte-ignore state_referenced_locally
  let minutes: number | null = $state(entry.minutes);
  // the person's own number, or empty when the figure is the estimate (the box then shows it as a hint)
  // svelte-ignore state_referenced_locally
  let own: number | null = $state(entry.edited ? entry.kcal : null);
  let failure = $state('');
  let busy = false;

  const fail = (e: unknown) => {
    failure = `Couldn't save that change: ${api.reason(e)}. Your diary is unchanged, so try again.`;
    onfailed?.();
  };

  async function save() {
    if (busy) return;
    busy = true;
    failure = '';
    try {
      // an own number that was cleared goes back to the estimate; an untouched estimate stays an estimate
      const r = await api.editExercise(day, today, entry.uid, minutes ?? 0, own, entry.edited && own == null);
      onsaved(r.view, `Updated ${r.name}`);
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
  }
  async function remove() {
    if (busy) return;
    busy = true;
    failure = '';
    try {
      const r = await api.deleteExercise(day, today, entry.uid);
      ondeleted(r.view, r.name);
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title={entry.name} {onclose}>
  <form id="edit-exercise-form" onsubmit={(e) => { e.preventDefault(); save(); }}>
    <label class="f">
      <span>Duration</span>
      <span class="dur">
        <input class="num" bind:value={minutes} type="number" min="0.01" step="any" inputmode="decimal" autocomplete="off" />
        <span class="note">minutes</span>
      </span>
    </label>

    <div class="f">
      <span class="labrow"><label for="ex-kcal">Calories burned</label>{#if own != null}<SourceTag tag="edited" />{/if}</span>
      <span class="kc">
        <input
          id="ex-kcal"
          class="num"
          bind:value={own}
          type="number"
          min="0"
          step="any"
          inputmode="decimal"
          autocomplete="off"
          placeholder={entry.edited ? 'Estimated when you save' : (entry.kcal != null ? String(entry.kcal) : '–')}
        />
        {#if own != null}<button type="button" class="btn ghost" onclick={() => { own = null; document.getElementById('ex-kcal')?.focus(); }}>Use estimate</button>{/if}
      </span>
      {#if own != null}<span class="note">Changing the duration changes the estimate. Your own number stays until you press Use estimate.</span>{/if}
    </div>

    <p class="note">{entry.frozen_text}</p>
  </form>
  {#if failure}
    <p class="notice" role="alert"><Icon name="warn" size={16} />{failure}</p>
  {/if}
  {#snippet actions()}
    <button type="button" class="btn ghost del" onclick={remove}>Delete Entry</button>
    <button type="button" class="btn" onclick={onclose}>Discard Edits</button>
    <button type="submit" form="edit-exercise-form" class="btn primary">Save Changes</button>
  {/snippet}
</Dialog>

<style>
  form { display: flex; flex-direction: column; gap: 16px; }
  .f { display: flex; flex-direction: column; gap: 4px; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .labrow { display: flex; align-items: center; gap: 8px; }
  .dur { display: flex; align-items: center; gap: 8px; }
  .kc { display: flex; align-items: center; gap: 8px; }
  /* more specific than the sheet's own field rules, which share the sheet's specificity */
  form .f input { font-size: 14.5px; }
  form .dur input, form .kc input { width: 96px; }
  form .btn.ghost { min-width: 0; padding: 0 8px; }
  form .note, p.note { margin: 0; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .btn.ghost.del { margin-right: auto; color: var(--heart); }
  .btn.ghost.del:hover { background: color-mix(in srgb, var(--heart) 10%, transparent); text-decoration: none; }
  .notice { display: flex; align-items: flex-start; gap: 8px; margin: 0; padding: 8px 16px; border-radius: 9px; background: var(--warn-wash); color: var(--warn); }
  .notice :global(svg) { flex: none; margin-top: 4px; }
</style>
