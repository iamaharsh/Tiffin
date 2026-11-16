<script lang="ts">
  import Dialog from './Dialog.svelte';
  import * as api from './api';
  import type { Preview } from './api';

  let { id, name, onclose, ondone, onerror }: { id: number; name: string; onclose: () => void; ondone: (m: string) => void; onerror: (m: string) => void } = $props();

  let lists: { id: number; name: string }[] = $state([]);
  let list = $state(0);
  let times = $state(1);
  let skip = $state(true);
  let preview: Preview | null = $state(null);
  let busy = $state(false);

  $effect(() => {
    api.groceries(0).then((g) => {
      lists = g.lists;
      list = g.list;
    }, (e) => onerror(String(e)));
  });
  $effect(() => {
    const t = times, s = skip; // rerun when either changes
    api.groceryPreview(id, t, s).then((p) => (preview = p), () => (preview = null));
  });

  async function add() {
    busy = true;
    try {
      ondone(await api.addRecipeToList(id, list, times, skip));
    } catch (e) {
      onerror(String(e));
    }
    busy = false;
  }
</script>

<Dialog title="Add to Grocery List" {onclose}>
  <p class="what">{name}</p>
  {#if lists.length > 1}
    <label class="label">List
      <select bind:value={list}>{#each lists as l}<option value={l.id}>{l.name}</option>{/each}</select>
    </label>
  {/if}
  <div class="times">
    <span>Times I'll cook it</span>
    <div class="stepper">
      <button aria-label="Fewer" onclick={() => (times = Math.max(1, times - 1))} disabled={times <= 1}>−</button>
      <span class="num">{times}×</span>
      <button aria-label="More" onclick={() => (times = Math.min(50, times + 1))} disabled={times >= 50}>+</button>
    </div>
  </div>
  <label class="check"><input type="checkbox" bind:checked={skip} /> Leave out what's already in my pantry</label>
  {#if preview}
    <p class="sum">
      <strong class="num">{preview.items}</strong> item{preview.items === 1 ? '' : 's'} will be added, grouped by store section{times > 1 ? `, every amount × ${times}` : ''}.
    </p>
    {#if preview.in_pantry.length}<p class="fine">Already in your pantry: {preview.in_pantry.join(', ')}</p>{/if}
  {/if}
  {#snippet actions()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" onclick={add} disabled={busy || !preview?.items}>Add Items</button>
  {/snippet}
</Dialog>

<style>
  .what { margin: -4px 0 0; font-size: 15px; font-weight: 600; color: var(--ink); }
  .times { display: flex; align-items: center; justify-content: space-between; color: var(--ink); }
  .stepper { display: flex; align-items: center; gap: 2px; padding: 2px; border-radius: 10px; background: var(--well); box-shadow: inset 0 0 0 1px var(--rule); }
  .stepper button { width: 30px; height: 28px; border-radius: 8px; font-size: 17px; }
  .stepper button:hover:not(:disabled) { background: var(--card); box-shadow: var(--shadow-card); }
  .stepper button:disabled { color: var(--ink-3); }
  .stepper .num { min-width: 40px; text-align: center; font-size: 18px; color: var(--ink); }
  .sum { margin: 0; color: var(--ink); }
  .sum strong { font-size: 18px; color: var(--tile-groceries); }
</style>
