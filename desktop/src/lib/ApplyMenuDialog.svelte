<script lang="ts">
  import Dialog from './Dialog.svelte';
  import * as api from './api';
  import type { MenuOut } from './api';

  // Put a menu on the meal plan: Day 1 lands on the chosen date. Used from Menus and from Meals.
  let { menus, menuId = null, start = null, today, onclose, ondone, onerror }: {
    menus: MenuOut[];
    /** a fixed menu (from the Menus page); otherwise one is chosen here */
    menuId?: number | null;
    /** a suggested first day (Meals: the day or week shown) */
    start?: number | null;
    today: number;
    onclose: () => void;
    ondone: (msg: string) => void;
    onerror: (m: string) => void;
  } = $props();

  // svelte-ignore state_referenced_locally
  let id = $state(menuId ?? menus[0]?.id ?? 0);
  // svelte-ignore state_referenced_locally
  let day = $state(start ?? today);
  let busy = $state(false);
  const menu = $derived(menus.find((m) => m.id === id));
  const count = $derived(menu?.entries.filter((e) => e.day >= 1 && e.day <= menu.days).length ?? 0);
  const wd = (d: number) => (((d + 4) % 7) + 7) % 7;
  // quick picks: today, tomorrow, and the coming Sunday and Monday
  const picks = $derived(
    (
      [
        ['Today', today],
        ['Tomorrow', today + 1],
        ['Next Sunday', today + 7 - wd(today)],
        ['Next Monday', today + ((8 - wd(today)) % 7 || 7)],
      ] as [string, number][]
    ).filter(([, d], i, all) => all.findIndex(([, x]) => x === d) === i),
  );

  async function go() {
    busy = true;
    try {
      const n = await api.applyMenu(id, day);
      ondone(`Added ${n} meal${n === 1 ? '' : 's'} from “${menu?.name}”, starting ${api.dayLabel(day, { weekday: 'long', month: 'short', day: 'numeric' })}`);
    } catch (e) {
      onerror(String(e));
    }
    busy = false;
  }
</script>

<Dialog title="Add Menu to Meal Plan" {onclose}>
  {#if menuId == null}
    <label class="label">Menu
      <select bind:value={id}>{#each menus as m}<option value={m.id}>{m.name}</option>{/each}</select>
    </label>
  {:else}
    <p class="what">{menu?.name}</p>
  {/if}
  <label class="label">Day 1 is
    <input type="date" value={api.dayToIso(day)} onchange={(e) => { const d = api.isoToDay(e.currentTarget.value); if (d != null) day = d; }} />
  </label>
  <div class="picks" role="radiogroup" aria-label="Start">
    {#each picks as [label, d]}
      <button class="pick" class:on={day === d} role="radio" aria-checked={day === d} onclick={() => (day = d)}>{label}</button>
    {/each}
  </div>
  {#if menu}
    <p class="sum">
      <strong class="num">{count}</strong> meal{count === 1 ? '' : 's'} over {menu.days} day{menu.days === 1 ? '' : 's'}:
      {api.dayLabel(day)} – {api.dayLabel(day + menu.days - 1)}
    </p>
  {/if}
  {#snippet actions()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" onclick={go} disabled={busy || !count}>Add to Meal Plan</button>
  {/snippet}
</Dialog>

<style>
  .what { margin: -4px 0 0; font-size: 15px; font-weight: 600; color: var(--ink); }
  .picks { display: flex; flex-wrap: wrap; gap: 6px; }
  .pick { height: 28px; padding: 0 12px; border-radius: 14px; font-size: 13px; background: var(--well); box-shadow: inset 0 0 0 1px var(--rule); }
  .pick:hover { background: var(--rule); }
  .pick.on { color: #fff; background: color-mix(in srgb, var(--tile-meals) 80%, black); box-shadow: none; }
  .sum { margin: 0; color: var(--ink); }
  .sum strong { font-size: 18px; }
</style>
