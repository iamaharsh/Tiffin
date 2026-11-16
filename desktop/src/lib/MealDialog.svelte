<script lang="ts">
  import Dialog from './Dialog.svelte';
  import Icon from './Icon.svelte';
  import * as api from './api';
  import type { MealType } from './api';

  let { id, name, today, onclose, ondone, onerror }: { id: number; name: string; today: number; onclose: () => void; ondone: (m: string) => void; onerror: (m: string) => void } = $props();

  // svelte-ignore state_referenced_locally
  let day = $state(today);
  let kind: MealType = $state('Dinner');
  let busy = $state(false);
  const kinds: MealType[] = ['Breakfast', 'Lunch', 'Dinner', 'Snack'];
  // the next 7 days as quick picks; the arrows reach any other day
  // svelte-ignore state_referenced_locally
  const week = Array.from({ length: 7 }, (_, i) => today + i);

  async function add() {
    busy = true;
    try {
      await api.addMeal(id, day, kind);
      ondone(`Added to ${kind.toLowerCase()} on ${api.dayLabel(day, { weekday: 'long', month: 'short', day: 'numeric' })}`);
    } catch (e) {
      onerror(String(e));
    }
    busy = false;
  }
</script>

<Dialog title="Add to Meal Plan" {onclose}>
  <p class="what">{name}</p>
  <div class="days" role="radiogroup" aria-label="Day">
    <button class="arrow" aria-label="Earlier" onclick={() => day--}><Icon name="back" size={15} /></button>
    {#each week as d}
      <button class="day" class:on={day === d} role="radio" aria-checked={day === d} onclick={() => (day = d)}>
        <span class="wd">{d === today ? 'Today' : api.dayLabel(d, { weekday: 'short' })}</span>
        <span class="n num">{api.dayLabel(d, { day: 'numeric' })}</span>
      </button>
    {/each}
    <button class="arrow flip" aria-label="Later" onclick={() => day++}><Icon name="back" size={15} /></button>
  </div>
  {#if !week.includes(day)}<p class="fine">{api.dayLabel(day, { weekday: 'long', month: 'long', day: 'numeric', year: 'numeric' })}</p>{/if}
  <div class="kinds" role="radiogroup" aria-label="Meal">
    {#each kinds as k}
      <label class="check"><input type="radio" name="kind" value={k} bind:group={kind} /> {k}</label>
    {/each}
  </div>
  {#snippet actions()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" onclick={add} disabled={busy}>Add Meal</button>
  {/snippet}
</Dialog>

<style>
  .what { margin: -4px 0 0; font-size: 15px; font-weight: 600; color: var(--ink); }
  .days { display: flex; align-items: stretch; gap: 4px; }
  .day {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1px;
    padding: 6px 0;
    border-radius: 10px;
    background: var(--well);
    transition: background-color 140ms ease-out, color 140ms ease-out, transform 100ms ease-out;
  }
  .day:hover { background: var(--rule); }
  .day:active { transform: scale(0.95); }
  .day.on { background: var(--tile-meals); color: #fff; }
  .wd { font-size: 11.5px; }
  .n { font-size: 19px; color: var(--ink); }
  .day.on .n { color: #fff; }
  .arrow { display: grid; place-items: center; width: 24px; border-radius: 8px; color: var(--ink-2); }
  .arrow:hover { background: var(--well); }
  .flip { transform: scaleX(-1); }
  .kinds { display: flex; gap: 18px; flex-wrap: wrap; }
</style>
