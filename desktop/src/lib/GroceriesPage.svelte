<script lang="ts">
  import { untrack } from 'svelte';
  import { fade, slide } from 'svelte/transition';
  import Icon from './Icon.svelte';
  import Segmented from './Segmented.svelte';
  import Menu, { type MenuItem } from './Menu.svelte';
  import Dialog from './Dialog.svelte';
  import * as api from './api';
  import { holdsUnsaved } from './updates.svelte';
  import type { Groceries, GroceryItem, Preview } from './api';

  let { plan = null, onerror, onnote }: {
    /** open "Add from the Meal Plan" for these days straight away (from the Meals page) */
    plan?: [number, number] | null;
    onerror: (m: string) => void;
    onnote: (m: string, action?: { label: string; run: () => void } | null) => void;
  } = $props();

  const remembered = (k: string) => { try { return localStorage.getItem(k); } catch { return null; } };
  const remember = (k: string, v: string) => { try { localStorage.setItem(k, v); } catch { /* private mode */ } };

  let g = $state<Groceries | null>(null);
  let listId = $state(Number(remembered('grocery-list') ?? 0));
  let show: 'all' | 'buy' | 'bought' = $state('all');
  /** one line per item: no recipe names or notes (remembered on this computer) */
  let compact = $state(remembered('grocery-compact') === '1');
  let recipeFilter = $state('');
  let quick = $state('');
  holdsUnsaved(() => quick.trim() !== '');
  let quickBox: HTMLInputElement;
  let menu: { x: number; y: number; items: MenuItem[] } | null = $state(null);
  let dialog: null | { kind: 'clear'; all: boolean; pantry: boolean; which: number } | { kind: 'name'; id: number; name: string } | { kind: 'plan'; from: number; to: number; skip: boolean } | { kind: 'aisle'; item: GroceryItem; aisle: string } = $state(null);
  let planPreview: Preview | null = $state(null);

  async function load() {
    try {
      g = await api.groceries(listId);
      listId = g.list;
      remember('grocery-list', String(listId));
      if (recipeFilter && !g.recipes.includes(recipeFilter)) recipeFilter = '';
    } catch (e) {
      onerror(String(e));
    }
  }
  $effect(() => {
    untrack(load); // once on open; later loads follow each change
    if (plan) untrack(() => openPlan(plan[0], plan[1]));
  });

  const list = $derived(g?.lists.find((l) => l.id === listId));
  const items = $derived.by(() => {
    if (!g) return [];
    return g.items
      .filter((i) => (show === 'buy' ? !i.purchased : show === 'bought' ? i.purchased : true))
      .filter((i) => !recipeFilter || i.recipe.split(', ').includes(recipeFilter));
  });
  // grouped by store section (in store-walk order) or by recipe
  const groups = $derived.by(() => {
    const byRecipe = g?.by_recipe;
    const key = (i: GroceryItem) => (byRecipe ? i.recipe || 'Other' : i.aisle || 'Miscellaneous');
    const sorted = [...items].sort((a, b) =>
      byRecipe ? key(a).localeCompare(key(b)) || a.name.localeCompare(b.name) : a.rank - b.rank || a.aisle.localeCompare(b.aisle) || a.name.localeCompare(b.name),
    );
    const out: { name: string; items: GroceryItem[] }[] = [];
    for (const i of sorted) {
      const k = key(i);
      if (out.at(-1)?.name !== k) out.push({ name: k, items: [] });
      out.at(-1)!.items.push(i);
    }
    return out;
  });
  const toBuy = $derived(g?.items.filter((i) => !i.purchased).length ?? 0);

  /** Change, then reload. Only removals pass `canUndo`: their message comes with Undo. */
  async function run(job: Promise<string | null | void>, fallback = '', canUndo = false) {
    try {
      const msg = await job;
      if (typeof msg === 'string') onnote(msg, canUndo ? { label: 'Undo', run: undo } : null);
      else if (fallback) onnote(fallback);
    } catch (e) {
      onerror(String(e));
    }
    await load();
  }
  async function undo() {
    try {
      await api.undoGroceries();
      onnote('Put back');
    } catch (e) {
      onerror(String(e));
    }
    await load();
  }

  function toggle(i: GroceryItem) {
    i.purchased = !i.purchased; // answer the click at once
    run(api.updateItem(listId, i.id, { purchased: i.purchased }));
  }

  async function addQuick() {
    const t = quick.trim();
    if (!t) return;
    quick = '';
    await run(api.addItem(listId, t));
    quickBox?.focus();
  }

  function itemMenu(e: MouseEvent, i: GroceryItem) {
    e.preventDefault();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu = {
      x: e.clientX || r.left + 40,
      y: e.clientY || r.bottom,
      items: [
        { label: i.purchased ? 'Mark as Not Bought' : 'Mark as Bought', icon: 'check', run: () => toggle(i) },
        'sep',
        { label: 'Increase Amount by 1', icon: 'plus', run: () => run(api.updateItem(listId, i.id, { step: 1 })) },
        { label: 'Decrease Amount by 1', icon: 'minus', run: () => run(api.updateItem(listId, i.id, { step: -1 })) },
        { label: 'Move to Section…', icon: 'cart', run: () => (dialog = { kind: 'aisle', item: i, aisle: i.aisle }) },
        'sep',
        { label: 'Remove', icon: 'trash', danger: true, run: () => run(api.removeItem(listId, i.id), '', true) },
      ],
    };
  }

  function listMenu(x: number, y: number, id: number, name: string) {
    menu = {
      x,
      y,
      items: [
        { label: 'Rename…', icon: 'pencil', run: () => (dialog = { kind: 'name', id, name }) },
        { label: 'Delete List', icon: 'trash', danger: true, disabled: (g?.lists.length ?? 0) < 2, run: () => run(api.deleteList(id), '', true).then(() => { if (listId === id) { listId = 0; load(); } }) },
      ],
    };
  }

  function moreMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const wk = g!.today - new Date(g!.today * 86400000).getUTCDay();
    menu = {
      x: r.right - 250,
      y: r.bottom + 6,
      items: [
        { label: 'Add from the Meal Plan…', icon: 'calendar', run: () => openPlan(wk, wk + 6) },
        'sep',
        { label: 'Remove Items Already in the Pantry', icon: 'jar', run: () => run(api.removePantryItems(listId), 'Nothing on this list is in the pantry', true) },
        { label: 'Move Bought Items to the Pantry', icon: 'jar', run: () => run(api.moveBoughtToPantry(listId), 'Tick the items you bought first', true) },
        { label: 'Clear Bought Items…', icon: 'check', run: () => (dialog = { kind: 'clear', all: false, pantry: false, which: 1 }) },
        { label: 'Clear All Items…', icon: 'trash', run: () => (dialog = { kind: 'clear', all: false, pantry: false, which: 0 }) },
        'sep',
        { label: 'Rename This List…', icon: 'pencil', run: () => (dialog = { kind: 'name', id: listId, name: list?.name ?? '' }) },
        { label: 'Delete This List', icon: 'trash', danger: true, disabled: (g?.lists.length ?? 0) < 2, run: () => run(api.deleteList(listId), '', true).then(() => { listId = 0; load(); }) },
      ],
    };
  }

  function openPlan(from: number, to: number) {
    dialog = { kind: 'plan', from, to, skip: true };
  }
  $effect(() => {
    if (dialog?.kind !== 'plan') return;
    const { from, to, skip } = dialog;
    api.planPreview(from, to, skip).then((p) => (planPreview = p), () => (planPreview = null));
  });

  async function saveName(d: { id: number; name: string }) {
    dialog = null;
    if (d.id === 0) {
      try {
        listId = await api.newList(d.name);
      } catch (e) {
        onerror(String(e));
      }
      await load();
    } else await run(api.renameList(d.id, d.name));
  }
</script>

<div class="groceries">
  <aside class="lists" aria-label="Grocery lists">
    <div class="lhead">
      <h2>Lists</h2>
      <button class="icon" aria-label="New grocery list" title="New list" onclick={() => (dialog = { kind: 'name', id: 0, name: '' })}><Icon name="plus" size={17} /></button>
    </div>
    <ul>
      {#each g?.lists ?? [] as l (l.id)}
        <li>
          <button
            class="lrow"
            class:sel={l.id === listId}
            aria-current={l.id === listId ? 'page' : undefined}
            onclick={() => { listId = l.id; recipeFilter = ''; load(); }}
            oncontextmenu={(e) => { e.preventDefault(); listMenu(e.clientX, e.clientY, l.id, l.name); }}
          >
            <span class="ldot"><Icon name="cart" size={14} /></span>
            <span class="lname">{l.name}</span>
            {#if l.to_buy}<span class="lcount num">{l.to_buy}</span>{/if}
          </button>
        </li>
      {/each}
    </ul>
  </aside>

  <section class="main" class:compact>
    <header class="top">
      <div class="heading">
        <h1>{list?.name ?? 'Groceries'}</h1>
        <span class="count num">{toBuy} to buy</span>
      </div>
      <div class="tools">
        <Segmented label="Show" options={['all', 'buy', 'bought'] as ('all' | 'buy' | 'bought')[]} value={show} onchange={(v) => (show = v)}>
          {#snippet item(v)}{({ all: 'All', buy: 'To Buy', bought: 'Bought' })[v]}{/snippet}
        </Segmented>
        <Segmented label="Group by" options={['section', 'recipe']} value={g?.by_recipe ? 'recipe' : 'section'} onchange={(v) => { const on = v === 'recipe'; if (g) g.by_recipe = on; run(api.setGroceryByRecipe(on)); }}>
          {#snippet item(v)}{v === 'recipe' ? 'By Recipe' : 'By Section'}{/snippet}
        </Segmented>
        {#if g?.recipes.length}
          <label class="select" title="Show one recipe's items">
            <Icon name="book" size={15} />
            <select bind:value={recipeFilter} aria-label="Recipe">
              <option value="">All recipes</option>
              {#each g.recipes as r}<option value={r}>{r}</option>{/each}
            </select>
          </label>
        {/if}
        <button class="icon" class:on={compact} aria-pressed={compact} aria-label="Compact rows" title={compact ? 'Show details' : 'Compact rows'} onclick={() => { compact = !compact; remember('grocery-compact', compact ? '1' : ''); }}><Icon name="list" size={18} /></button>
        <button class="icon" aria-label="More list actions" aria-haspopup="menu" onclick={moreMenu}><Icon name="more" size={18} /></button>
      </div>
    </header>

    <form class="quick" onsubmit={(e) => { e.preventDefault(); addQuick(); }}>
      <span class="plus"><Icon name="plus" size={16} /></span>
      <input bind:this={quickBox} bind:value={quick} placeholder="Add an item, like “2 cups milk”" aria-label="Add an item" autocomplete="off" />
      {#if quick.trim()}<button class="addbtn" transition:fade={{ duration: 120 }}>Add</button>{/if}
    </form>

    {#if g && !items.length}
      <div class="empty">
        <span class="etile"><Icon name="cart" size={30} /></span>
        <h2>{g.items.length ? 'Nothing matches these filters' : 'This list is empty'}</h2>
        <p>{g.items.length ? 'Change Show or the recipe filter above.' : 'Type an item above, add a recipe’s ingredients from its page, or use ⋯ › Add from the Meal Plan.'}</p>
      </div>
    {/if}

    {#each groups as grp (grp.name)}
      <section class="group" transition:slide={{ duration: 180 }}>
        <h2 class="caps sect">{grp.name}</h2>
        <ul>
          {#each grp.items as i (i.id)}
            <li class="item label-scope" class:done={i.purchased} style:--l={api.labelVar(i.label)} transition:slide={{ duration: 160 }}>
              <button class="tick" role="checkbox" aria-checked={i.purchased} aria-label="{i.name}{i.quantity ? `, ${i.quantity}` : ''}" onclick={() => toggle(i)} oncontextmenu={(e) => itemMenu(e, i)}>
                <span class="box"><Icon name="check" size={13} /></span>
              </button>
              <button class="body" onclick={() => toggle(i)} oncontextmenu={(e) => itemMenu(e, i)} tabindex="-1">
                <span class="name">{i.name}{#if i.note}<span class="note">{i.note}</span>{/if}</span>
                {#if i.recipe && i.recipe !== 'My Items' && !g?.by_recipe}<span class="from">{#if i.label != null}<span class="sw"></span>{/if}{i.recipe}</span>{/if}
              </button>
              {#if i.quantity}<span class="qty num">{i.quantity}</span>{/if}
              <div class="hover">
                <button class="mini" aria-label="Less {i.name}" onclick={() => run(api.updateItem(listId, i.id, { step: -1 }))}><Icon name="minus" size={13} /></button>
                <button class="mini" aria-label="More {i.name}" onclick={() => run(api.updateItem(listId, i.id, { step: 1 }))}><Icon name="plus" size={13} /></button>
                <button class="mini" aria-label="Remove {i.name}" onclick={() => run(api.removeItem(listId, i.id), '', true)}><Icon name="close" size={13} /></button>
              </div>
            </li>
          {/each}
        </ul>
      </section>
    {/each}
  </section>
</div>

{#if menu}<Menu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />{/if}

{#if dialog?.kind === 'name'}
  {@const d = dialog}
  <Dialog title={d.id ? 'Rename List' : 'New Grocery List'} onclose={() => (dialog = null)}>
    <form id="name-form" onsubmit={(e) => { e.preventDefault(); if (d.name.trim()) saveName(d); }}>
      <!-- svelte-ignore a11y_autofocus -->
      <input bind:value={d.name} placeholder="Weekend shop" aria-label="List name" autofocus />
    </form>
    {#snippet actions()}
      <button class="btn" onclick={() => (dialog = null)}>Cancel</button>
      <button class="btn primary" form="name-form" disabled={!d.name.trim()}>{d.id ? 'Rename' : 'Create'}</button>
    {/snippet}
  </Dialog>
{:else if dialog?.kind === 'clear'}
  {@const d = dialog}
  {@const n = g?.items.filter((i) => (d.which === 1 ? i.purchased : d.which === 2 ? !i.purchased : true)).length ?? 0}
  <Dialog title={d.which === 1 ? 'Clear Bought Items?' : 'Clear Items?'} onclose={() => (dialog = null)}>
    <label class="check"><input type="radio" name="where" checked={!d.all} onchange={() => (d.all = false)} /> This list, “{list?.name}” ({n} item{n === 1 ? '' : 's'})</label>
    <label class="check"><input type="radio" name="where" checked={d.all} onchange={() => (d.all = true)} /> Every grocery list</label>
    <div class="which" role="radiogroup" aria-label="Which items">
      {#each ['All items', 'Only bought', 'Only still to buy'] as w, k}
        <label class="check"><input type="radio" name="which" value={k} bind:group={d.which} /> {w}</label>
      {/each}
    </div>
    <label class="check"><input type="checkbox" bind:checked={d.pantry} /> Also empty the pantry ({g?.pantry ?? 0} items)</label>
    <p class="fine">The lists themselves stay. You can undo this right after.</p>
    {#snippet actions()}
      <button class="btn" onclick={() => (dialog = null)}>Cancel</button>
      <button class="btn danger" onclick={() => { const job = api.clearItems(listId, d.all, d.pantry, d.which); dialog = null; run(job, 'Nothing to clear', true); }}>Clear Items</button>
    {/snippet}
  </Dialog>
{:else if dialog?.kind === 'plan' && g}
  {@const d = dialog}
  {@const wk = g.today - new Date(g.today * 86400000).getUTCDay()}
  {@const ranges = [['This Week', wk, wk + 6], ['Next 7 Days', g.today, g.today + 6], ['Next Week', wk + 7, wk + 13]] as [string, number, number][]}
  <Dialog title="Add from the Meal Plan" onclose={() => (dialog = null)}>
    <div class="ranges" role="radiogroup" aria-label="Days">
      {#each ranges as [label, from, to]}
        <button class="range" class:on={d.from === from && d.to === to} role="radio" aria-checked={d.from === from && d.to === to} onclick={() => { d.from = from; d.to = to; }}>{label}</button>
      {/each}
    </div>
    <p style="margin:0">{api.dayLabel(d.from)} – {api.dayLabel(d.to)}: <strong class="num">{planPreview?.recipes ?? 0}</strong> planned recipe{planPreview?.recipes === 1 ? '' : 's'}</p>
    <label class="check"><input type="checkbox" bind:checked={d.skip} /> Leave out what’s already in my pantry</label>
    {#if planPreview}
      <p style="margin:0;color:var(--ink)"><strong class="num">{planPreview.items}</strong> item{planPreview.items === 1 ? '' : 's'} will be added to “{list?.name}”, combined and grouped by store section.</p>
      {#if planPreview.in_pantry.length}<p class="fine">Skipped (in pantry): {planPreview.in_pantry.join(', ')}</p>{/if}
    {/if}
    {#snippet actions()}
      <button class="btn" onclick={() => (dialog = null)}>Cancel</button>
      <button class="btn primary" disabled={!planPreview?.items} onclick={() => { const job = api.addPlanToList(listId, d.from, d.to, d.skip); dialog = null; run(job, ''); }}>Add to List</button>
    {/snippet}
  </Dialog>
{:else if dialog?.kind === 'aisle' && g}
  {@const d = dialog}
  <Dialog title="Move to Section" onclose={() => (dialog = null)}>
    <p style="margin:0;color:var(--ink)">{d.item.name}</p>
    <select bind:value={d.aisle} aria-label="Store section">{#each g.aisles as a}<option value={a}>{a}</option>{/each}</select>
    {#snippet actions()}
      <button class="btn" onclick={() => (dialog = null)}>Cancel</button>
      <button class="btn primary" onclick={() => { const job = api.updateItem(listId, d.item.id, { aisle: d.aisle }); dialog = null; run(job); }}>Move</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  /* white text needs a deeper green than the sidebar tile (4.5:1) */
  .groceries { --groc-deep: color-mix(in srgb, var(--tile-groceries) 72%, black); display: flex; min-height: 100%; }
  .lists {
    position: sticky;
    top: 0;
    align-self: flex-start;
    width: 220px;
    flex: none;
    height: 100vh;
    padding: 18px 10px;
    border-right: 1px solid var(--rule);
    overflow-y: auto;
  }
  .lhead { display: flex; align-items: center; justify-content: space-between; padding: 0 6px 0 10px; margin-bottom: 8px; }
  .lhead h2 { margin: 0; font: 700 15px var(--font-display); color: var(--ink-2); }
  ul { list-style: none; margin: 0; padding: 0; }
  .lrow { display: flex; align-items: center; gap: 9px; width: 100%; height: 34px; padding: 0 10px; border-radius: 9px; text-align: left; transition: background-color 120ms ease-out; }
  .lrow:hover { background: var(--sidebar-hover); }
  .lrow.sel { background: var(--groc-deep); color: #fff; }
  .ldot { display: grid; place-items: center; width: 22px; height: 22px; border-radius: 6px; color: #fff; background: var(--tile-groceries); flex: none; }
  .lrow.sel .ldot { background: rgba(255, 255, 255, 0.22); }
  .lname { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-weight: 500; }
  .lcount { font-size: 14px; color: var(--ink-3); }
  .lrow.sel .lcount { color: rgba(255, 255, 255, 0.85); }
  .icon { display: grid; place-items: center; width: 32px; height: 30px; border-radius: 9px; color: var(--accent-text); transition: background-color 120ms ease-out; }
  .icon:hover { background: var(--accent-wash); }
  .icon :global(svg) { stroke-width: 2; }

  .main { flex: 1; min-width: 0; padding: 0 32px 64px; }
  .top {
    position: sticky;
    top: 0;
    z-index: 4;
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 10px 16px;
    min-height: 64px;
    padding: 12px 0;
    background: var(--toolbar);
    backdrop-filter: blur(20px) saturate(170%);
  }
  .heading { flex: 1; min-width: 180px; display: flex; align-items: baseline; gap: 12px; }
  h1 { margin: 0; font: 700 26px var(--font-display); letter-spacing: -0.02em; color: var(--ok); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .count { font-size: 16px; color: var(--ink-3); white-space: nowrap; }
  .tools { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .select { max-width: 230px; display: flex; align-items: center; gap: 6px; height: 30px; padding: 0 10px; border-radius: 9px; background: var(--well); box-shadow: inset 0 0 0 1px var(--rule); color: var(--ink-2); }
  .select select { max-width: 160px; border: 0; background: none; color: var(--ink); font: inherit; font-size: 13px; outline: none; appearance: none; }
  .select select option { color: #1f1b18; background: #fff; }
  .select:focus-within { box-shadow: 0 0 0 3px var(--focus), inset 0 0 0 1px var(--accent); }

  .quick { display: flex; align-items: center; gap: 10px; height: 46px; margin: 4px 0 18px; padding: 0 6px 0 12px; border-radius: 12px; background: var(--card); box-shadow: var(--shadow-card), inset 0 0 0 1px var(--rule); }
  .quick:focus-within { box-shadow: 0 0 0 3px var(--focus), inset 0 0 0 1px var(--accent); }
  .plus { display: grid; place-items: center; width: 24px; height: 24px; border-radius: 50%; color: #fff; background: var(--tile-groceries); }
  .plus :global(svg) { stroke-width: 2.4; }
  .quick input { flex: 1; min-width: 0; border: 0; outline: none; background: none; color: var(--ink); font: inherit; font-size: 15px; -webkit-user-select: text; user-select: text; }
  .quick input::placeholder { color: var(--ink-3); }
  .addbtn { height: 34px; padding: 0 16px; border-radius: 9px; font-weight: 600; color: #fff; background: var(--groc-deep); }

  .group { margin-bottom: 20px; }
  .sect { margin: 0 0 4px; padding: 0 0 6px 42px; font-size: 14.5px; color: var(--ink-2); border-bottom: 1px solid var(--rule); }
  .item { position: relative; display: flex; align-items: center; gap: 4px; min-height: 48px; border-bottom: 1px solid var(--rule); }
  .item:hover { background: color-mix(in srgb, var(--well) 60%, transparent); }
  .tick { display: grid; place-items: center; width: 40px; height: 44px; flex: none; }
  .box {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    color: transparent;
    box-shadow: inset 0 0 0 1.8px var(--rule-strong);
    transition: background-color 180ms ease-out, box-shadow 180ms ease-out, transform 120ms ease-out;
  }
  .box :global(svg) { stroke-width: 2.8; transform: scale(0.3); transition: transform 280ms var(--ease-spring); }
  .tick:hover .box { box-shadow: inset 0 0 0 1.8px var(--tile-groceries); }
  .tick:active .box { transform: scale(0.88); }
  .done .box { background: var(--tile-groceries); color: #fff; box-shadow: none; }
  .done .box :global(svg) { transform: none; }
  .body { flex: 1; min-width: 0; display: flex; flex-direction: column; align-items: flex-start; padding: 6px 0; text-align: left; }
  .name { font-size: 15px; transition: color 200ms ease-out; }
  .note { margin-left: 0.3em; color: var(--ink-2); }
  .done .name { color: var(--ink-3); text-decoration: line-through; text-decoration-color: var(--ink-3); }
  .from { display: inline-flex; align-items: center; gap: 6px; font-size: 12.5px; color: var(--ink-2); font-style: italic; }
  .sw { width: 8px; height: 8px; border-radius: 2px; background: var(--l); }
  .qty { padding: 0 6px; font-size: 16px; color: var(--ink-2); white-space: nowrap; }
  .done .qty { color: var(--ink-3); }
  .hover { display: flex; gap: 2px; padding-right: 4px; opacity: 0; transition: opacity 120ms ease-out; }
  .item:hover .hover, .item:focus-within .hover { opacity: 1; }
  .mini { display: grid; place-items: center; width: 26px; height: 26px; border-radius: 7px; color: var(--ink-2); }
  .mini:hover { background: var(--rule); color: var(--ink); }
  .icon.on { background: var(--accent-wash); }
  .compact .item { min-height: 36px; }
  .compact .tick { height: 34px; }
  .compact .body { padding: 3px 0; }
  .compact .from, .compact .note { display: none; }
  .compact .qty { font-size: 14.5px; }

  .empty { margin-top: 10vh; text-align: center; color: var(--ink-2); }
  .etile { display: inline-grid; place-items: center; width: 64px; height: 64px; border-radius: 18px; color: #fff; background: var(--tile-groceries); box-shadow: var(--shadow-lift); }
  .empty h2 { margin: 16px 0 6px; font: 700 20px var(--font-display); color: var(--ink); }
  .empty p { margin: 0 auto; max-width: 46ch; }

  .which { display: flex; gap: 16px; flex-wrap: wrap; padding-left: 26px; }
  .ranges { display: flex; gap: 6px; }
  .range { flex: 1; height: 34px; border-radius: 9px; font-size: 13.5px; font-weight: 500; background: var(--well); box-shadow: inset 0 0 0 1px var(--rule); }
  .range.on { background: var(--tile-meals); color: #fff; box-shadow: none; }

  @media (max-width: 900px) {
    .lists { width: 170px; }
    .main { padding: 0 18px 48px; }
  }
</style>
