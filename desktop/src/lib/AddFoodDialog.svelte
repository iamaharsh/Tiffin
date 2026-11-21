<script lang="ts">
  import Dialog from './Dialog.svelte';
  import FoodSearch from './FoodSearch.svelte';
  import QuickAddDialog from './QuickAddDialog.svelte';
  import Icon from './Icon.svelte';
  import * as api from './api';
  import type { DayView, FoodDetail as Info, MealType } from './api';

  // Add Food to {Meal}: the Foods search and detail. Add to {Meal} is the footer's primary button, so it never scrolls away; it logs the
  // amount and unit chosen in the panel. Done Adding is the secondary. The dialog stays open, so a whole meal goes in during one visit.
  let { day, today, slot, label, us, onclose, onlogged, onfailed, onerror }: {
    day: string;
    today: string;
    slot: MealType;
    label: string;
    us: boolean;
    onclose: () => void;
    /** the day as the crate now holds it, so the totals behind the dialog update at once */
    onlogged: (v: DayView, name: string) => void;
    /** a write failed: the page reloads the day from the crate */
    onfailed?: () => void;
    onerror: (m: string) => void;
  } = $props();

  let failure = $state('');
  let quick = $state(false);
  let busy = false;
  let search: { clear: () => void; confirm: (text: string) => void; escape: () => boolean; chosen: () => Info | null } | undefined = $state();

  async function addFood(d: Info) {
    if (busy) return;
    busy = true;
    failure = '';
    try {
      const r = await api.logFood(day, today, slot, d.key, d.amount, d.unit);
      onlogged(r.view, r.name);
      // the next food starts from a clean, focused search; the status line says what went in
      search?.clear();
      search?.confirm(`Added ${r.name} to ${label}`);
    } catch (e) {
      // the toast sits behind a modal, so the notice is inline
      failure = `Couldn't save that change: ${api.reason(e)}. Your diary is unchanged, so try again.`;
      onfailed?.();
    } finally {
      busy = false;
    }
  }
</script>

<!-- Esc undoes typed text, then a stacked detail, and only then closes; a click outside and Done Adding close at once -->
<Dialog large title="Add Food to {label}" {onclose} oncancel={() => { if (!search?.escape()) onclose(); }}>
  <FoodSearch bind:this={search} layout="dialog" {today} {day} mealSlot={slot} {us} {onerror} onenter={addFood} />
  {#if failure}
    <p class="notice" role="alert"><Icon name="warn" size={16} />{failure}</p>
  {/if}
  {#snippet actions()}
    <button type="button" class="btn quick" onclick={() => (quick = true)}>Quick Add…</button>
    <button type="button" class="btn" onclick={onclose}>Done Adding</button>
    <button
      type="button"
      class="btn primary"
      aria-disabled={search?.chosen() ? undefined : 'true'}
      onclick={() => { const d = search?.chosen(); if (d) addFood(d); }}
    >Add to {label}</button>
  {/snippet}
</Dialog>

{#if quick}
  <QuickAddDialog
    {day}
    {today}
    {slot}
    {label}
    onclose={() => (quick = false)}
    onlogged={(v, name) => {
      onlogged(v, name);
      search?.clear();
      search?.confirm(`Added ${name} to ${label}`);
    }}
    {onfailed}
    {onerror}
  />
{/if}

<style>
  .quick { margin-right: auto; }
  .btn[aria-disabled='true'] { opacity: 0.5; }
  .notice { display: flex; align-items: flex-start; gap: 8px; margin: 0; padding: 8px 16px; border-radius: 9px; background: var(--warn-wash); color: var(--warn); }
  .notice :global(svg) { flex: none; margin-top: 4px; }
</style>
