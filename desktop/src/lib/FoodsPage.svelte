<script lang="ts">
  import Segmented from './Segmented.svelte';
  import FoodSearch from './FoodSearch.svelte';
  import FoodForm from './FoodForm.svelte';
  import * as api from './api';
  import { holdsUnsaved } from './updates.svelte';
  import type { FoodDetail, MyFood } from './api';

  let { units = $bindable('metric'), onchanged, onerror, onnote }: {
    units?: 'metric' | 'us';
    /** a saved or deleted food re-estimates recipes, so the rest of the app reloads */
    onchanged?: () => void;
    onerror: (m: string) => void;
    onnote: (m: string, action?: { label: string; run: () => void } | null) => void;
  } = $props();

  let editing: MyFood | null = $state(null);
  holdsUnsaved(() => editing !== null);
  let search: { refresh: (closeDetail?: boolean) => void } | undefined = $state();
  let saving = $state(false);

  function saved(f: MyFood) {
    editing = null;
    onnote(`Saved ${f.name}. Recipes that use it are re-estimated.`);
    search?.refresh();
    onchanged?.();
  }
  function deleted(f: MyFood) {
    editing = null;
    onnote(`Removed ${f.name}`);
    search?.refresh(true);
    onchanged?.();
  }

  /** Open a food that is in My Foods for editing, found by what it is called or its id. */
  async function edit(find: (f: MyFood) => boolean) {
    try {
      const f = (await api.myFoods()).find(find);
      if (f) editing = f;
      else onerror("That food isn't in My Foods any more.");
    } catch (e) {
      onerror(String(e));
    }
  }

  /** Online result to My Foods: saved at once when the draft is ready and the name is free, else the form opens prefilled. */
  async function saveOnline(d: FoodDetail) {
    if (saving) return;
    saving = true;
    try {
      const { food, ready } = await api.onlineFoodDraft(d.key.slice(4));
      const name = food.name.trim().toLowerCase();
      // saving under a name already in My Foods would replace that food, so the person decides in the form
      const taken = (await api.myFoods()).some((f) => f.name.trim().toLowerCase() === name);
      if (!ready || taken) {
        editing = food;
        return;
      }
      await api.saveMyFood(food);
      search?.refresh();
      onchanged?.();
      onnote(`Saved “${food.name}” to My Foods.`, { label: 'Edit', run: () => edit((f) => f.name.trim().toLowerCase() === name) });
    } catch (e) {
      onerror(String(e));
    } finally {
      saving = false;
    }
  }
</script>

{#snippet actions(d: FoodDetail)}
  {#if d.online}
    <button type="button" class="act primary" aria-busy={saving} onclick={() => saveOnline(d)}>{saving ? 'Saving…' : 'Save to My Foods'}</button>
  {:else if d.key.startsWith('my:')}
    <button type="button" class="act" onclick={() => edit((f) => `my:${f.id}` === d.key)}>Edit Food</button>
  {/if}
{/snippet}

<div class="foods">
  <header class="top">
    <h1>Foods</h1>
    <Segmented label="Units" options={['metric', 'us'] as ('metric' | 'us')[]} value={units} onchange={(v) => (units = v)}>
      {#snippet item(v)}{v === 'us' ? 'US' : 'Metric'}{/snippet}
    </Segmented>
  </header>
  <FoodSearch bind:this={search} us={units === 'us'} {onerror} {actions} onadd={(name, barcode) => (editing = { ...api.blankFood(name), barcode: barcode ?? '' })} />
</div>

{#if editing}
  {#key editing}
    <FoodForm food={editing} onsaved={saved} onclose={() => (editing = null)} {onerror} ondelete={deleted} />
  {/key}
{/if}

<style>
  .foods { max-width: 980px; margin: 0 auto; padding: 0 32px 64px; }
  .top {
    position: sticky;
    top: 0;
    z-index: 4;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    min-height: 64px;
    margin: 0 -32px;
    padding: 12px 32px;
    background: var(--toolbar);
    backdrop-filter: blur(20px) saturate(170%);
  }
  .act { display: inline-flex; align-items: center; justify-content: center; height: 34px; padding: 0 16px; border-radius: 9px; font-size: 14px; font-weight: 500; background: var(--well); color: var(--ink); box-shadow: inset 0 0 0 1px var(--rule); }
  .act:hover { background: var(--rule); }
  .act.primary { background: var(--accent); color: white; box-shadow: inset 0 -1px 0 rgba(0, 0, 0, 0.15); }
  .act.primary:hover { background: var(--accent-press); }
  h1 { margin: 0; font: 700 24px var(--font-display); letter-spacing: -0.018em; color: color-mix(in srgb, var(--tile-foods), var(--ink) 35%); }
</style>
