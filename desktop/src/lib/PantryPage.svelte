<script lang="ts">
  import { untrack } from 'svelte';
  import { fade, slide } from 'svelte/transition';
  import Icon from './Icon.svelte';
  import Segmented from './Segmented.svelte';
  import Menu, { type MenuItem } from './Menu.svelte';
  import Dialog from './Dialog.svelte';
  import * as api from './api';
  import { holdsUnsaved } from './updates.svelte';
  import type { Pantry, PantryEdit, PantryItem } from './api';

  let { onerror, onnote }: { onerror: (m: string) => void; onnote: (m: string, action?: { label: string; run: () => void } | null) => void } = $props();

  let p = $state<Pantry | null>(null);
  let show: 'all' | 'in' | 'out' = $state('all');
  const remembered = (k: string) => { try { return localStorage.getItem(k); } catch { return null; } };
  const remember = (k: string, v: string) => { try { localStorage.setItem(k, v); } catch { /* private mode */ } };
  /** one line per item: dates and tags hidden (remembered on this computer) */
  let compact = $state(remembered('pantry-compact') === '1');
  let quick = $state('');
  holdsUnsaved(() => quick.trim() !== '');
  let quickBox: HTMLInputElement;
  let menu: { x: number; y: number; items: MenuItem[] } | null = $state(null);
  let edit: PantryEdit | null = $state(null);
  let choose: { picks: Record<number, boolean>; list: number } | null = $state(null);
  let confirmClear = $state(false);

  async function load() {
    try {
      p = await api.pantry();
    } catch (e) {
      onerror(String(e));
    }
  }
  $effect(() => { untrack(load); });

  const sorts = ['Section', 'Added', 'Expires', 'In Stock'];
  const sortTips = ['Sort by store section', 'Sort by date added', 'Sort by expiration date', "Sort by what's in stock"];
  const items = $derived.by(() => {
    if (!p) return [];
    const v = p.items.filter((i) => (show === 'in' ? i.in_stock : show === 'out' ? !i.in_stock : true));
    const byName = (a: PantryItem, b: PantryItem) => a.name.localeCompare(b.name);
    switch (p.sort) {
      case 1: return v.sort((a, b) => (b.bought ?? -1e9) - (a.bought ?? -1e9) || byName(a, b));
      case 2: return v.sort((a, b) => (a.expires ?? 1e9) - (b.expires ?? 1e9) || byName(a, b));
      case 3: return v.sort((a, b) => Number(b.in_stock) - Number(a.in_stock) || byName(a, b));
      default: return v.sort((a, b) => a.rank - b.rank || a.aisle.localeCompare(b.aisle) || byName(a, b));
    }
  });
  const bandNames: Record<string, string> = { expired: 'Expired', today: 'Expires Today', week: 'Expires This Week', month: 'Expires This Month', later: 'Expires Later' };
  // bands by section, or by how soon things expire when sorted that way; none for the others
  const groups = $derived.by(() => {
    const key = (i: PantryItem) => (p?.sort === 0 ? i.aisle || 'Miscellaneous' : p?.sort === 2 ? (i.expiry ? bandNames[i.expiry] : 'No Expiration Date') : '');
    const out: { name: string; items: PantryItem[] }[] = [];
    for (const i of items) {
      const k = key(i);
      if (!out.length || out.at(-1)!.name !== k) out.push({ name: k, items: [] });
      out.at(-1)!.items.push(i);
    }
    return out;
  });
  // How urgent a date is, in days from today (the bands above follow the calendar like the classic
  // app; the reminder and tag colours count days, so Monday is "soon" on a Friday).
  const urgency = (i: PantryItem) => (i.expires == null || !p ? 'none' : i.expires < p.today ? 'expired' : i.expires - p.today <= 7 ? 'soon' : 'none');
  // what to use up first: in stock and expiring within a week
  const soon = $derived(p?.items.filter((i) => i.in_stock && urgency(i) !== 'none').sort((a, b) => a.expires! - b.expires!) ?? []);
  const outCount = $derived(p?.items.filter((i) => !i.in_stock).length ?? 0);

  const rel = new Intl.RelativeTimeFormat(undefined, { numeric: 'auto' });
  function expiryText(i: PantryItem) {
    if (i.expires == null || !p) return '';
    const d = i.expires - p.today;
    if (d < 0) return `Expired ${rel.format(d, 'day')}`;
    if (d <= 13) return `Expires ${rel.format(d, 'day')}`;
    return `Expires ${api.dayLabel(i.expires, { month: 'short', day: 'numeric' })}`;
  }

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

  const asEdit = (i: PantryItem): PantryEdit => ({ id: i.id, name: i.name, quantity: i.quantity, aisle: i.aisle, bought: i.bought, expires: i.expires, in_stock: i.in_stock });
  function toggleStock(i: PantryItem) {
    i.in_stock = !i.in_stock; // answer the click at once
    run(api.updatePantry(asEdit(i)));
  }
  async function addQuick() {
    const t = quick.trim();
    if (!t) return;
    quick = '';
    await run(api.addPantry(t));
    quickBox?.focus();
  }
  async function saveEdit(e: PantryEdit) {
    try {
      await api.updatePantry(e);
      edit = null;
    } catch (err) {
      onerror(String(err));
    }
    await load();
  }

  function itemMenu(e: MouseEvent, i: PantryItem) {
    e.preventDefault();
    menu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: 'Edit…', icon: 'pencil', run: () => (edit = asEdit(i)) },
        { label: i.in_stock ? 'Mark Out of Stock' : 'Mark In Stock', icon: 'jar', run: () => toggleStock(i) },
        { label: 'Add to Grocery List', icon: 'cart', run: () => addOne(i) },
        'sep',
        { label: 'Delete', icon: 'trash', danger: true, run: () => run(api.deletePantry(i.id), '', true) },
      ],
    };
  }
  function moreMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu = {
      x: r.right - 280,
      y: r.bottom + 6,
      items: [
        { label: 'Add Out of Stock Items to a Grocery List…', icon: 'cart', disabled: !outCount, run: openChooser },
        { label: 'Remove Out of Stock Items', icon: 'jar', disabled: !outCount, run: () => run(api.removeOutOfStock(), 'Nothing is marked out of stock', true) },
        { label: 'Remove Expired Items', icon: 'clock', run: () => run(api.removeExpired(), 'Nothing has expired', true) },
        'sep',
        { label: 'Clear All Items…', icon: 'trash', danger: true, disabled: !p?.items.length, run: () => (confirmClear = true) },
      ],
    };
  }
  function openChooser() {
    if (!p) return;
    choose = { picks: Object.fromEntries(p.items.filter((i) => !i.in_stock).map((i) => [i.id, true])), list: p.lists[0]?.[0] ?? 0 };
  }
  // "Add to Grocery List" from one item reports what happened
  async function addOne(i: PantryItem) {
    try {
      onnote(await api.pantryToList([i.id], p?.lists[0]?.[0] ?? 0));
    } catch (e) {
      onerror(String(e));
    }
  }
</script>

<div class="pantry" class:compact>
  <header class="top">
    <div class="heading">
      <h1>Pantry</h1>
      <span class="count num">{p?.items.length ?? 0} items{#if outCount}{` · ${outCount} out of stock`}{/if}</span>
    </div>
    <div class="tools">
      <Segmented label="Show" options={['all', 'in', 'out'] as ('all' | 'in' | 'out')[]} value={show} onchange={(v) => (show = v)}>
        {#snippet item(v)}{({ all: 'All', in: 'In Stock', out: 'Out of Stock' })[v]}{/snippet}
      </Segmented>
      <Segmented label="Sort by" title="Sort by" options={[0, 1, 2, 3]} value={p?.sort ?? 0} status={(i) => `Sorted by ${sorts[i]}`} onchange={(v) => { if (p) p.sort = v; run(api.setPantrySort(v)); }}>
        {#snippet item(i)}<span title={sortTips[i]}>{sorts[i]}</span>{/snippet}
      </Segmented>
      <button class="icon" class:on={compact} aria-pressed={compact} aria-label="Compact rows" title={compact ? 'Show details' : 'Compact rows'} onclick={() => { compact = !compact; remember('pantry-compact', compact ? '1' : ''); }}><Icon name="list" size={18} /></button>
        <button class="icon" aria-label="More pantry actions" aria-haspopup="menu" onclick={moreMenu}><Icon name="more" size={18} /></button>
    </div>
  </header>

  <form class="quick" onsubmit={(e) => { e.preventDefault(); addQuick(); }}>
    <span class="plus"><Icon name="plus" size={16} /></span>
    <input bind:this={quickBox} bind:value={quick} placeholder="Add what you have, like “2 lbs flour”" aria-label="Add a pantry item" autocomplete="off" />
    {#if quick.trim()}<button class="addbtn" transition:fade={{ duration: 120 }}>Add</button>{/if}
  </form>

  {#if soon.length && show !== 'out'}
    <section class="soon" transition:slide={{ duration: 200 }} aria-label="Use soon">
      <h2 class="caps"><Icon name="clock" size={15} /> Use soon</h2>
      <div class="chips">
        {#each soon as i (i.id)}
          <button class="soonchip" class:expired={i.expiry === 'expired'} onclick={() => (edit = asEdit(i))}>
            <strong>{i.name}</strong><span>{expiryText(i)}</span>
          </button>
        {/each}
      </div>
    </section>
  {/if}

  {#if p && !items.length}
    <div class="empty">
      <span class="etile"><Icon name="jar" size={30} /></span>
      <h2>{p.items.length ? 'Nothing to show' : 'Your pantry is empty'}</h2>
      <p>{p.items.length ? 'No items match the Show choice above.' : 'Type what you have above, or tick items on a grocery list and use ⋯ › Move Bought Items to the Pantry.'}</p>
    </div>
  {/if}

  {#each groups as grp (grp.name)}
    <section class="group">
      {#if grp.name}<h2 class="caps sect">{grp.name}</h2>{/if}
      <ul>
        {#each grp.items as i (i.id)}
          <li class="item" class:out={!i.in_stock} transition:slide={{ duration: 160 }}>
            <button class="stock" role="switch" aria-checked={i.in_stock} aria-label="{i.name} in stock" title={i.in_stock ? 'In stock: click when it runs out' : 'Out of stock: click when you have it again'} onclick={() => toggleStock(i)}>
              <span class="jar"></span>
            </button>
            <button class="body" onclick={() => (edit = asEdit(i))} oncontextmenu={(e) => itemMenu(e, i)}>
              <span class="name">{i.name}{#if i.quantity}<span class="qty">{i.quantity}</span>{/if}</span>
              <span class="meta">
                {#if !i.in_stock}<span class="tag outtag">Out of stock</span>{/if}
                {#if i.expires != null}<span class="tag {urgency(i)}">{expiryText(i)}</span>{/if}
                {#if i.bought != null}<span class="when">Bought {api.dayLabel(i.bought, { month: 'short', day: 'numeric' })}</span>{/if}
              </span>
            </button>
            <button class="mini" aria-label="Add {i.name} to the grocery list" title="Add to the grocery list" onclick={() => addOne(i)}><Icon name="cart" size={15} /></button>
          </li>
        {/each}
      </ul>
    </section>
  {/each}
</div>

{#if menu}<Menu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />{/if}

{#if edit && p}
  {@const e = edit}
  {@const plus = (n: number) => (e.expires = p!.today + n)}
  <Dialog title="Pantry Item" onclose={() => (edit = null)}>
    <form id="pantry-form" class="form" onsubmit={(ev) => { ev.preventDefault(); saveEdit(e); }}>
      <label class="label">Name<input bind:value={e.name} autocomplete="off" /></label>
      <div class="two">
        <label class="label">Amount<input bind:value={e.quantity} placeholder="2 lbs" autocomplete="off" /></label>
        <label class="label">Section<select bind:value={e.aisle}>{#each p.aisles as a}<option value={a}>{a}</option>{/each}</select></label>
      </div>
      <div class="two">
        <label class="label">Bought
          <input type="date" value={api.dayToIso(e.bought)} onchange={(ev) => (e.bought = api.isoToDay(ev.currentTarget.value))} />
        </label>
        <label class="label">Expires
          <input type="date" value={api.dayToIso(e.expires)} onchange={(ev) => (e.expires = api.isoToDay(ev.currentTarget.value))} />
        </label>
      </div>
      <div class="quickdates" aria-label="Expiry shortcuts">
        <span>Expires in</span>
        <button type="button" onclick={() => plus(3)}>3 days</button>
        <button type="button" onclick={() => plus(7)}>1 week</button>
        <button type="button" onclick={() => plus(30)}>1 month</button>
        <button type="button" onclick={() => plus(365)}>1 year</button>
        {#if e.expires != null}<button type="button" onclick={() => (e.expires = null)}>No date</button>{/if}
      </div>
      <label class="check"><input type="checkbox" bind:checked={e.in_stock} /> In stock</label>
    </form>
    {#snippet actions()}
      <button class="btn danger left" onclick={() => { const id = e.id; edit = null; run(api.deletePantry(id), '', true); }}>Delete</button>
      <button class="btn" onclick={() => (edit = null)}>Cancel</button>
      <button class="btn primary" form="pantry-form" disabled={!e.name.trim()}>Save</button>
    {/snippet}
  </Dialog>
{/if}

{#if choose && p}
  {@const c = choose}
  {@const outs = p.items.filter((i) => !i.in_stock)}
  {@const n = outs.filter((i) => c.picks[i.id]).length}
  <Dialog title="Add to a Grocery List" onclose={() => (choose = null)}>
    <div class="allnone"><span>Choose the out of stock items to buy</span><button onclick={() => outs.forEach((i) => (c.picks[i.id] = true))}>All</button><button onclick={() => outs.forEach((i) => (c.picks[i.id] = false))}>None</button></div>
    <div class="picks">
      {#each outs as i (i.id)}
        <label class="check"><input type="checkbox" bind:checked={c.picks[i.id]} /> {i.name}{#if i.quantity}<span class="fine">&nbsp;({i.quantity})</span>{/if}</label>
      {/each}
    </div>
    {#if p.lists.length > 1}
      <label class="label">List<select bind:value={c.list}>{#each p.lists as [id, name]}<option value={id}>{name}</option>{/each}</select></label>
    {/if}
    {#snippet actions()}
      <button class="btn" onclick={() => (choose = null)}>Cancel</button>
      <button class="btn primary" disabled={!n} onclick={() => { const job = api.pantryToList(outs.filter((i) => c.picks[i.id]).map((i) => i.id), c.list); choose = null; job.then((m) => onnote(m), (e) => onerror(String(e))); }}>Add {n} Item{n === 1 ? '' : 's'}</button>
    {/snippet}
  </Dialog>
{/if}

{#if confirmClear}
  <Dialog title="Clear the Pantry?" onclose={() => (confirmClear = false)}>
    <p style="margin:0;color:var(--ink)">Remove all {p?.items.length} items from the pantry?</p>
    <p class="fine">You can undo this right after.</p>
    {#snippet actions()}
      <button class="btn" onclick={() => (confirmClear = false)}>Cancel</button>
      <button class="btn danger" onclick={() => { confirmClear = false; run(api.clearItems(0, false, true, 0), '', true); }}>Clear Pantry</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  /* amber text dark enough to read on both themes; a deeper amber behind white text */
  .pantry {
    --pantry-ink: color-mix(in srgb, var(--tile-pantry), var(--ink) 35%);
    --pantry-deep: color-mix(in srgb, var(--tile-pantry) 70%, black);
    max-width: 980px;
    margin: 0 auto;
    padding: 0 32px 64px;
  }
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
    margin: 0 -32px;
    padding-inline: 32px;
    background: var(--toolbar);
    backdrop-filter: blur(20px) saturate(170%);
  }
  .heading { flex: 1; min-width: 180px; display: flex; align-items: baseline; gap: 12px; }
  h1 { margin: 0; font: 700 26px var(--font-display); letter-spacing: -0.02em; color: var(--pantry-ink); }
  .count { font-size: 16px; color: var(--ink-3); white-space: nowrap; }
  .tools { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .icon { display: grid; place-items: center; width: 32px; height: 30px; border-radius: 9px; color: var(--accent-text); }
  .icon:hover { background: var(--accent-wash); }
  .icon :global(svg) { stroke-width: 2.6; }

  .quick { display: flex; align-items: center; gap: 10px; height: 46px; margin: 4px 0 18px; padding: 0 6px 0 12px; border-radius: 12px; background: var(--card); box-shadow: var(--shadow-card), inset 0 0 0 1px var(--rule); }
  .quick:focus-within { box-shadow: 0 0 0 3px var(--focus), inset 0 0 0 1px var(--accent); }
  .plus { display: grid; place-items: center; width: 24px; height: 24px; border-radius: 50%; color: #fff; background: var(--pantry-deep); }
  .plus :global(svg) { stroke-width: 2.4; }
  .quick input { flex: 1; min-width: 0; border: 0; outline: none; background: none; color: var(--ink); font: inherit; font-size: 15px; -webkit-user-select: text; user-select: text; }
  .quick input::placeholder { color: var(--ink-3); }
  .addbtn { height: 34px; padding: 0 16px; border-radius: 9px; font-weight: 600; color: #fff; background: var(--pantry-deep); }

  .soon { margin-bottom: 24px; padding: 14px 16px; border-radius: 14px; background: color-mix(in srgb, var(--tile-pantry) 12%, var(--page)); }
  .soon h2 { display: flex; align-items: center; gap: 6px; margin: 0 0 10px; font-size: 15px; color: var(--pantry-ink); }
  .chips { display: flex; flex-wrap: wrap; gap: 8px; }
  .soonchip {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    padding: 7px 12px;
    border-radius: 11px;
    background: var(--card);
    box-shadow: var(--shadow-card);
    text-align: left;
    transition: transform 140ms var(--ease-out), box-shadow 200ms var(--ease-out);
  }
  .soonchip:hover { transform: translateY(-1px); box-shadow: var(--shadow-lift); }
  .soonchip:active { transform: scale(0.97); }
  .soonchip strong { font-size: 14px; font-weight: 600; }
  .soonchip span { font-size: 12px; color: var(--warn); }
  .soonchip.expired span { color: var(--heart); }

  .group { margin-bottom: 20px; }
  ul { list-style: none; margin: 0; padding: 0; }
  .sect { margin: 0 0 4px; padding: 0 0 6px 42px; font-size: 14.5px; color: var(--ink-2); border-bottom: 1px solid var(--rule); }
  .item { display: flex; align-items: center; gap: 4px; min-height: 52px; border-bottom: 1px solid var(--rule); }
  .item:hover { background: color-mix(in srgb, var(--well) 60%, transparent); }
  .stock { display: grid; place-items: center; width: 40px; height: 46px; flex: none; }
  /* a jar that's full (in stock) or empty (out) */
  .jar {
    width: 16px;
    height: 20px;
    border-radius: 4px 4px 5px 5px;
    box-shadow: inset 0 0 0 1.8px var(--tile-pantry);
    background: linear-gradient(to top, var(--tile-pantry) 70%, transparent 70%);
    transition: background-size 260ms var(--ease-out), opacity 200ms ease-out;
    background-size: 100% 100%;
    background-repeat: no-repeat;
    background-position: bottom;
  }
  .out .jar { background-size: 100% 0%; opacity: 0.7; }
  .stock:hover .jar { box-shadow: inset 0 0 0 1.8px var(--pantry-ink); }
  .stock:active .jar { transform: scale(0.9); }
  .body { flex: 1; min-width: 0; display: flex; flex-direction: column; align-items: flex-start; gap: 3px; padding: 7px 0; text-align: left; }
  .name { font-size: 15px; }
  .qty { margin-left: 8px; color: var(--ink-2); font-size: 14px; }
  .out .name { color: var(--ink-2); }
  .meta { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; font-size: 12.5px; }
  .when { color: var(--ink-3); }
  .tag { padding: 1px 7px; border-radius: 6px; font-weight: 500; color: var(--ink-2); background: var(--well); }
  .tag.expired, .tag.outtag { color: var(--heart); background: color-mix(in srgb, var(--heart) 10%, transparent); }
  .tag.soon { color: var(--warn); background: var(--warn-wash); }
  .mini { display: grid; place-items: center; width: 32px; height: 32px; border-radius: 8px; color: var(--ink-3); opacity: 0; transition: opacity 120ms ease-out; }
  .item:hover .mini, .mini:focus-visible { opacity: 1; }
  .mini:hover { background: var(--rule); color: var(--ink); }
  .icon.on { background: var(--accent-wash); }
  .compact .item { min-height: 36px; }
  .compact .stock { height: 34px; }
  .compact .body { padding: 3px 0; }
  .compact .when { display: none; }
  .compact .meta { display: contents; }
  .compact .body { flex-direction: row; align-items: center; gap: 8px; }
  .compact .tag { font-size: 12px; }

  .empty { margin-top: 10vh; text-align: center; color: var(--ink-2); }
  .etile { display: inline-grid; place-items: center; width: 64px; height: 64px; border-radius: 18px; color: #fff; background: var(--tile-pantry); box-shadow: var(--shadow-lift); }
  .empty h2 { margin: 16px 0 6px; font: 700 20px var(--font-display); color: var(--ink); }
  .empty p { margin: 0 auto; max-width: 46ch; }

  .form { display: flex; flex-direction: column; gap: 12px; }
  .two { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
  .quickdates { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; font-size: 12.5px; }
  .quickdates button { height: 26px; padding: 0 10px; border-radius: 13px; font-size: 12.5px; background: var(--well); box-shadow: inset 0 0 0 1px var(--rule); }
  .quickdates button:hover { background: var(--rule); }
  :global(.btn.left) { margin-right: auto; }
  .allnone { display: flex; align-items: center; gap: 8px; }
  .allnone span { flex: 1; }
  .allnone button { font-size: 13px; color: var(--accent-text); }
  .picks { display: flex; flex-direction: column; gap: 8px; max-height: 260px; overflow-y: auto; padding: 2px; }

  @media (max-width: 760px) {
    .pantry { padding: 0 18px 48px; }
    .top { margin: 0 -18px; padding-inline: 18px; }
  }
</style>
