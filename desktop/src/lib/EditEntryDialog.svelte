<script lang="ts">
  import Dialog from './Dialog.svelte';
  import FoodDetail from './FoodDetail.svelte';
  import Segmented from './Segmented.svelte';
  import Icon from './Icon.svelte';
  import * as api from './api';
  import type { DayView, EntryView, MealType } from './api';

  // Edit Entry: amount, unit and meal of one logged entry, measured against the basis it was logged with.
  // Every entry is alike here, quick adds included: the dialog never invents an amount or a unit, it saves what FoodDetail shows.
  let { day, today, entry, slot, us, onclose, onsaved, ondeleted, onfailed, onerror }: {
    day: string;
    today: string;
    entry: EntryView;
    slot: MealType;
    us: boolean;
    /** Esc, a click outside and Discard Edits: nothing was saved, nothing is asked */
    onclose: () => void;
    onsaved: (v: DayView, message: string) => void;
    ondeleted: (v: DayView, name: string) => void;
    /** a write failed: the page reloads the day from the crate */
    onfailed?: () => void;
    onerror: (m: string) => void;
  } = $props();

  const meals: MealType[] = ['Breakfast', 'Lunch', 'Dinner', 'Snack'];
  const word = (m: MealType) => (m === 'Snack' ? 'Snacks' : m);

  // svelte-ignore state_referenced_locally
  let mealSlot: MealType = $state(slot);
  let detail: FoodDetail | undefined = $state();
  let failure = $state('');
  let busy = false;

  // the amount and unit as first loaded, to tell a move from an edit in the message
  let first: { amount: number; unit: string } | null = null;
  $effect(() => {
    const c = detail?.current();
    if (!first && c && c.amount > 0) first = c;
  });

  const fail = (e: unknown) => {
    failure = `Couldn't save that change: ${api.reason(e)}. Your diary is unchanged, so try again.`;
    onfailed?.();
  };

  async function save() {
    const c = detail?.current();
    if (busy || !c) return;
    busy = true;
    failure = '';
    try {
      const r = await api.editEntry(day, today, entry.uid, c.amount, c.unit, mealSlot);
      const moved = mealSlot !== slot && first && c.amount === first.amount && c.unit === first.unit;
      onsaved(r.view, moved ? `Moved ${r.name} to ${word(mealSlot)}` : `Updated ${r.name}`);
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
      const r = await api.deleteEntry(day, today, entry.uid);
      ondeleted(r.view, r.name);
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title={entry.name} {onclose}>
  <FoodDetail bind:this={detail} key={`entry:${day}:${entry.uid}`} {us} {onerror} named={false} name={entry.name} />
  <p class="note">Values are kept as they were when you logged this, even if the food has changed since.</p>
  <div class="meal">
    <span class="lab">Meal</span>
    <Segmented label="Meal" options={meals} value={mealSlot} onchange={(v) => (mealSlot = v)}>
      {#snippet item(o)}{word(o)}{/snippet}
    </Segmented>
  </div>
  {#if failure}
    <p class="notice" role="alert"><Icon name="warn" size={16} />{failure}</p>
  {/if}
  {#snippet actions()}
    <button type="button" class="btn ghost del" onclick={remove}>Delete Entry</button>
    <button type="button" class="btn" onclick={onclose}>Discard Edits</button>
    <button type="button" class="btn primary" onclick={save}>Save Changes</button>
  {/snippet}
</Dialog>

<style>
  .note { margin: 0; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .meal { display: flex; flex-direction: column; align-items: flex-start; gap: 4px; }
  .lab { font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .btn.ghost.del { margin-right: auto; color: var(--heart); }
  .btn.ghost.del:hover { background: color-mix(in srgb, var(--heart) 10%, transparent); text-decoration: none; }
  .notice { display: flex; align-items: flex-start; gap: 8px; margin: 0; padding: 8px 16px; border-radius: 9px; background: var(--warn-wash); color: var(--warn); }
  .notice :global(svg) { flex: none; margin-top: 4px; }
</style>
