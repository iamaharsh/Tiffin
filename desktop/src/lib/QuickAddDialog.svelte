<script lang="ts">
  import Dialog from './Dialog.svelte';
  import Icon from './Icon.svelte';
  import * as api from './api';
  import type { DayView, MealType } from './api';

  // Quick Add to {Meal}: calories and, if known, macros, for something with no entry in the food list.
  // A blank box is unknown, never zero.
  let { day, today, slot, label, onclose, onlogged, onfailed, onerror }: {
    day: string;
    today: string;
    slot: MealType;
    label: string;
    /** Back to Search, and Esc: nothing is kept, nothing asked (four short fields) */
    onclose: () => void;
    onlogged: (v: DayView, name: string) => void;
    onfailed?: () => void;
    onerror: (m: string) => void;
  } = $props();

  let cal: number | null = $state(null);
  let protein: number | null = $state(null);
  let carbs: number | null = $state(null);
  let fat: number | null = $state(null);
  let name = $state('');
  let calBox: HTMLInputElement | undefined = $state();
  let missing = $state(false);
  let warning = $state('');
  let failure = $state('');
  let busy = false;
  let checkSeq = 0;

  $effect(() => { calBox?.focus(); });

  /** The crate says when a figure is too large to be right; it never stops the add. */
  async function check() {
    missing = false;
    const mine = ++checkSeq;
    if (cal == null) { warning = ''; return; }
    try {
      const w = await api.quickAddCheck(cal);
      if (mine === checkSeq) warning = w ?? '';
    } catch (e) {
      onerror(String(e));
    }
  }

  async function add() {
    if (busy) return;
    if (cal == null) {
      // the crate refuses this too; the form keeps what was typed and says where
      missing = true;
      calBox?.focus();
      return;
    }
    busy = true;
    failure = '';
    try {
      const r = await api.quickAdd(day, today, slot, name.trim(), cal, protein, carbs, fat);
      onlogged(r.view, r.name);
      onclose();
    } catch (e) {
      failure = `Couldn't save that change: ${api.reason(e)}. Your diary is unchanged, so try again.`;
      onfailed?.();
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title="Quick Add to {label}" {onclose}>
  <form id="quick-add-form" onsubmit={(e) => { e.preventDefault(); add(); }}>
    <label class="f">
      <span>Calories <span class="req">required</span></span>
      <input
        bind:this={calBox}
        bind:value={cal}
        oninput={check}
        type="number"
        min="0"
        step="any"
        inputmode="decimal"
        autocomplete="off"
        aria-invalid={missing ? 'true' : undefined}
        aria-describedby={missing ? 'qa-missing' : warning ? 'qa-warning' : undefined}
      />
    </label>
    {#if missing}<p id="qa-missing" class="note warn">Enter the calories for this entry.</p>{/if}
    {#if warning}<p id="qa-warning" class="note warn">{warning}</p>{/if}

    <div class="grid">
      <label class="f"><span>Protein (g)</span><input bind:value={protein} type="number" min="0" step="any" inputmode="decimal" autocomplete="off" placeholder="–" /></label>
      <label class="f"><span>Carbs (g)</span><input bind:value={carbs} type="number" min="0" step="any" inputmode="decimal" autocomplete="off" placeholder="–" /></label>
      <label class="f"><span>Fat (g)</span><input bind:value={fat} type="number" min="0" step="any" inputmode="decimal" autocomplete="off" placeholder="–" /></label>
    </div>
    <p class="note">Leave a box empty if you don't know it. Empty means unknown, not zero.</p>

    <label class="f">
      <span>Label (optional)</span>
      <input bind:value={name} maxlength="80" autocomplete="off" placeholder="Office cake" />
    </label>
  </form>
  {#if failure}
    <p class="notice" role="alert"><Icon name="warn" size={16} />{failure}</p>
  {/if}
  {#snippet actions()}
    <button type="button" class="btn" onclick={onclose}>Back to Search</button>
    <button type="submit" form="quick-add-form" class="btn primary">Add to {label}</button>
  {/snippet}
</Dialog>

<style>
  form { display: flex; flex-direction: column; gap: 16px; }
  .f { display: flex; flex-direction: column; gap: 4px; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .req { color: var(--ink-3); }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(120px, 1fr)); gap: 16px; }
  /* more specific than the sheet's own field and .fine rules, which share its specificity */
  form p.note { margin: 0; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  form p.note.warn { color: var(--warn); }
  form .f input { font-size: 14.5px; }
  .notice { display: flex; align-items: flex-start; gap: 8px; margin: 0; padding: 8px 16px; border-radius: 9px; background: var(--warn-wash); color: var(--warn); }
  .notice :global(svg) { flex: none; margin-top: 4px; }
</style>
