<script lang="ts">
  import { untrack, type Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  import SourceTag from './SourceTag.svelte';
  import * as api from './api';
  import type { Amount, FoodDetail as Info } from './api';

  // One food: pick an amount and a unit, see calories and macros. The crate does all unit math
  // and rounding; this page only shows its answers. `actions` is where later plans add buttons.
  let { key, us, onerror, named = true, name = '', actions, onenter }: {
    key: string;
    us: boolean;
    onerror: (m: string) => void;
    /** false when the surrounding dialog already shows the name */
    named?: boolean;
    /** the name the row already showed, so the panel is never blank while the food loads */
    name?: string;
    actions?: Snippet<[Info]>;
    /** Enter in the Amount box: the host does what its primary button does */
    onenter?: (d: Info) => void;
  } = $props();

  let detail: Info | null = $state(null);
  let unit = $state('g');
  let amount: number | null = $state(null);
  let result = $state<Amount | null>(null);
  let amountBox: HTMLInputElement | undefined = $state();
  let wantFocus = false;
  let loadedKey = '';
  let loadSeq = 0;
  let amountSeq = 0;

  /** Focus the Amount box with its text selected, so typing replaces it (waits for the food to load). */
  export function focusAmount() {
    if (amountBox) { amountBox.focus(); amountBox.select(); } else wantFocus = true;
  }
  /** The amount and unit now in the box (0 while empty or still loading), for hosts that keep their own buttons, like Edit Entry. */
  export function current(): { amount: number; unit: string } {
    return { amount: amount ?? 0, unit };
  }
  /** The open food with the amount and unit now chosen, or null while loading, for a host that keeps its own Add button. */
  export function food(): Info | null {
    return chosen;
  }
  $effect(() => {
    if (amountBox && wantFocus) { wantFocus = false; amountBox.focus(); amountBox.select(); }
  });

  async function load(k: string, u: boolean) {
    const mine = ++loadSeq;
    try {
      const d = await api.foodDetail(k, u);
      if (mine !== loadSeq) return; // a slower older answer never replaces a newer one
      detail = d;
      if (loadedKey !== k) {
        // first time for this food: its own default amount and unit (the units switch only reorders later)
        loadedKey = k;
        unit = d.unit;
        amount = d.amount;
        // its own sequence (not loadSeq): a Metric/US toggle during this call starts another load that skips the amount, so this answer must still land
        const my = ++amountSeq;
        const a = await api.foodAmount(k, d.amount, d.unit);
        if (my === amountSeq) result = a;
      }
    } catch (e) {
      onerror(String(e));
    }
  }
  $effect(() => {
    const k = key, u = us;
    untrack(() => load(k, u));
  });

  async function measure(value: number | null) {
    const mine = ++amountSeq;
    try {
      const a = await api.foodAmount(key, value, unit);
      if (mine === amountSeq) result = a;
    } catch (e) {
      onerror(String(e));
    }
  }
  function typed(e: Event & { currentTarget: HTMLInputElement }) {
    const v = e.currentTarget.valueAsNumber;
    measure(Number.isNaN(v) || v < 0 ? null : v);
  }
  async function changeUnit(next: string) {
    if (next === unit) return;
    const from = unit;
    unit = next;
    const mine = ++amountSeq;
    try {
      const a = await api.foodAmount(key, amount ?? null, next, from);
      if (mine !== amountSeq) return;
      result = a;
      amount = a.amount; // the crate converts and rounds; the page only shows its answer
    } catch (e) {
      unit = from;
      onerror(String(e));
    }
  }

  // actions get the food with the amount and unit now chosen, not its defaults
  const chosen = $derived.by(() => {
    const d = detail;
    return d ? { ...d, amount: amount ?? 0, unit } : null;
  });
  function amountKeys(e: KeyboardEvent) {
    if (e.key !== 'Enter' || !chosen) return;
    e.preventDefault();
    onenter?.(chosen);
  }

  // an empty or zero amount has no figures at all
  const live = $derived(amount ? result : null);
  const n = $derived(live?.nutrients ?? {});
  const macros = [['Protein', 'protein_g', 'p'], ['Carbs', 'carbs_g', 'c'], ['Fat', 'fat_g', 'f']] as const;
  const others = [['Fiber', 'fiber_g', 'g'], ['Sugar', 'sugar_g', 'g'], ['Sodium', 'sodium_mg', 'mg']] as const;
</script>

{#snippet val(x: number | null | undefined)}{#if x == null}<span aria-hidden="true">–</span><span class="sr">not reported</span>{:else}{x}{/if}{/snippet}

<div class="detail">
  {#if detail}
    <header>
      {#if named}<h3 title={detail.name}>{detail.name}</h3>{/if}
      <div class="meta">
        <SourceTag tag={detail.tag} />
        {#if detail.brand}<span class="fine">{detail.brand}</span>{/if}
      </div>
      {#if detail.online}
        <p class="fine caution"><Icon name="globe" size={14} /> Online data entered by contributors. Check it against the pack.</p>
      {/if}
    </header>

    {#if detail.warning}
      <p class="fine caution"><Icon name="warn" size={16} />{detail.warning}</p>
    {/if}
    {#if detail.note}<p class="fine note">{detail.note}</p>{/if}

    <div class="amountrow">
      <label class="field amt">Amount
        <input
          bind:this={amountBox}
          bind:value={amount}
          oninput={typed}
          onkeydown={amountKeys}
          class="num"
          type="number"
          min="0"
          step="any"
          inputmode="decimal"
          autocomplete="off"
        />
      </label>
      <label class="field">Unit
        <select value={unit} onchange={(e) => changeUnit(e.currentTarget.value)}>
          {#each detail.units as u (u.id)}<option value={u.id}>{u.label}</option>{/each}
        </select>
      </label>
    </div>

    {#if unit !== 'g' && live?.grams != null}
      <p class="fine weight">
        <span class="num">= {live.grams} g</span>
        <button type="button" class="ghost" onclick={() => changeUnit('g')}>Use grams</button>
      </p>
    {/if}

    <section class="nut" aria-label="Nutrition">
      <p class="energy"><span class="kcal num">{@render val(n.calories)}</span><span class="fine">kcal</span></p>
      <ul class="macros">
        {#each macros as [label, k, dot]}
          <li><i class="dot {dot}" aria-hidden="true"></i><span class="lab">{label}</span><span class="v"><span class="num">{@render val(n[k])}</span>{#if n[k] != null} g{/if}</span></li>
        {/each}
      </ul>
      <ul class="others">
        {#each others as [label, k, u]}
          <li><span class="lab">{label}</span><span class="v"><span class="num">{@render val(n[k])}</span>{#if n[k] != null} {u}{/if}</span></li>
        {/each}
      </ul>
    </section>

    {#if actions && chosen}<div class="actions">{@render actions(chosen)}</div>{/if}
  {:else}
    {#if named && name}<h3 title={name}>{name}</h3>{/if}
    <p class="fine">Loading amounts…</p>
  {/if}
</div>

<style>
  .detail { display: flex; flex-direction: column; gap: 16px; font-size: 14.5px; line-height: 1.45; color: var(--ink); }
  header { display: flex; flex-direction: column; gap: 8px; }
  h3 {
    margin: 0;
    font: 600 21px/1.2 var(--font-display);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    overflow-wrap: anywhere;
  }
  .meta { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; }
  .fine { font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .caution { display: flex; align-items: flex-start; gap: 6px; margin: 0; color: var(--warn); }
  .caution :global(svg) { flex: none; margin-top: 2px; }
  .note { margin: 0; }

  .amountrow { display: flex; gap: 8px; align-items: flex-end; }
  .field { display: flex; flex-direction: column; gap: 4px; flex: 1; min-width: 0; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .field.amt { flex: none; width: 96px; }
  .field input, .field select {
    width: 100%;
    height: 34px;
    padding: 0 10px;
    border: 0;
    border-radius: 9px;
    background: var(--well);
    box-shadow: inset 0 0 0 1px var(--rule);
    color: var(--ink);
    font: inherit;
    font-size: 14.5px;
    outline: none;
  }
  .field input.num { font-size: 14.5px; font-weight: 600; font-family: var(--font-label); }
  .field input:focus, .field select:focus { box-shadow: 0 0 0 3px var(--focus), inset 0 0 0 1px var(--accent); }

  .weight { display: flex; align-items: center; gap: 8px; margin: -4px 0 0; }
  .ghost { color: var(--accent-text); font-size: 13px; font-weight: 600; }
  .ghost:hover { text-decoration: underline; }

  .nut { display: flex; flex-direction: column; gap: 12px; }
  .energy { display: flex; align-items: baseline; gap: 6px; margin: 0; }
  .kcal { font-size: 26px; line-height: 1.1; }
  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  li { display: flex; align-items: center; gap: 8px; }
  .lab { flex: 1; }
  .v { text-align: right; }
  .others { padding-top: 12px; border-top: 1px solid var(--rule); color: var(--ink-2); }
  .dot { width: 8px; height: 8px; border-radius: 50%; flex: none; }
  .dot.p { background: var(--protein); }
  .dot.c { background: var(--carbs); }
  .dot.f { background: var(--fat); }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
</style>
