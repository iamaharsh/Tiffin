<script lang="ts">
  import { untrack } from 'svelte';
  import { fade } from 'svelte/transition';
  import Icon from './Icon.svelte';
  import Photo from './Photo.svelte';
  import Segmented from './Segmented.svelte';
  import Menu, { type MenuItem } from './Menu.svelte';
  import ApplyMenuDialog from './ApplyMenuDialog.svelte';
  import PlanRecipeDialog from './PlanRecipeDialog.svelte';
  import * as api from './api';
  import type { Card, Category, Meal, MealType, MenuOut } from './api';

  let { today, categories, onopen, onplan, onerror, onnote }: {
    today: number;
    categories: Category[];
    onopen: (recipeId: number) => void;
    /** make a grocery list for these days */
    onplan: (from: number, to: number) => void;
    onerror: (m: string) => void;
    onnote: (m: string, action?: { label: string; run: () => void } | null) => void;
  } = $props();

  const remembered = (k: string) => { try { return localStorage.getItem(k); } catch { return null; } };
  const remember = (k: string, v: string) => { try { localStorage.setItem(k, v); } catch { /* private mode */ } };

  type View = 'day' | 'week' | 'month';
  let view: View = $state((remembered('meal-view') as View) || 'week');
  // svelte-ignore state_referenced_locally
  let anchor = $state(today);
  let meals: Meal[] = $state([]);
  let menu: { x: number; y: number; items: MenuItem[] } | null = $state(null);
  // "Add a Menu…": the saved menus, and the first day suggested for Day 1
  let applying: { menus: MenuOut[]; start: number } | null = $state(null);
  // the recipe picker sheet, opened from an empty slot or a day's "+"
  let choosing: { day: number; kind: MealType | null } | null = $state(null);

  // the recipe picker on the right
  let shelf = $state('all');
  let search = $state('');
  let recipes: Card[] = $state([]);
  let pick: Card | null = $state(null);
  let dropDay: number | null = $state(null);
  let dropKind: MealType | null = $state(null);

  const kinds: MealType[] = ['Breakfast', 'Lunch', 'Dinner', 'Snack'];
  const DAY = 86400000;
  const wd = (d: number) => (((d + 4) % 7) + 7) % 7; // 0 = Sunday, like the classic app
  const ymd = (d: number) => { const x = new Date(d * DAY); return [x.getUTCFullYear(), x.getUTCMonth(), x.getUTCDate()]; };
  const dayOf = (y: number, m: number, dd: number) => Math.round(Date.UTC(y, m, dd) / DAY);

  const grid = $derived.by(() => {
    if (view === 'day') return { from: anchor, to: anchor, days: [anchor] };
    if (view === 'week') { const s = anchor - wd(anchor); return { from: s, to: s + 6, days: Array.from({ length: 7 }, (_, i) => s + i) }; }
    const [y, m] = ymd(anchor);
    const first = dayOf(y, m, 1);
    const count = new Date(Date.UTC(y, m + 1, 0)).getUTCDate();
    const start = first - wd(first);
    const rows = Math.ceil((wd(first) + count) / 7);
    return { from: start, to: start + rows * 7 - 1, days: Array.from({ length: rows * 7 }, (_, i) => start + i), month: m, first, last: first + count - 1 };
  });
  const title = $derived.by(() => {
    if (view === 'day') return api.dayLabel(anchor, { weekday: 'long', month: 'long', day: 'numeric' });
    if (view === 'week') {
      const [, m1] = ymd(grid.from), [, m2] = ymd(grid.to);
      return `${api.dayLabel(grid.from, { month: 'long', day: 'numeric' })} – ${api.dayLabel(grid.to, m1 === m2 ? { day: 'numeric' } : { month: 'long', day: 'numeric' })}`;
    }
    return api.dayLabel(anchor, { month: 'long', year: 'numeric' });
  });
  // the days a "grocery list for what's shown" or "clear" covers (a month means its own days)
  const shown = $derived(view === 'month' && grid.first != null ? [grid.first, grid.last!] : [grid.from, grid.to]);

  async function load() {
    try {
      meals = await api.meals(grid.from, grid.to);
    } catch (e) {
      onerror(String(e));
    }
  }
  $effect(() => { grid.from; grid.to; untrack(load); });
  $effect(() => { remember('meal-view', view); });
  $effect(() => {
    const v = shelf, q = search;
    api.list(v, q, 0).then((c) => (recipes = c), (e) => onerror(String(e)));
  });

  const on = (d: number) => meals.filter((m) => m.day === d);
  function step(dir: number) {
    if (view === 'day') anchor += dir;
    else if (view === 'week') anchor += 7 * dir;
    else { const [y, m] = ymd(anchor); anchor = dayOf(y, m + dir, 1); }
  }

  async function act(job: Promise<unknown>, msg = '', action: { label: string; run: () => void } | null = null) {
    try {
      await job;
      if (msg) onnote(msg, action);
    } catch (e) {
      onerror(String(e));
    }
    await load();
  }
  const add = (recipe: { id: number; name: string }, day: number, kind: MealType | null = null) => act(api.addMeal(recipe.id, day, kind));

  async function remove(ids: number[], from: number | null = null, to: number | null = null) {
    try {
      const gone = await api.removeMeals(ids, from, to);
      if (gone.length) onnote(gone.length === 1 ? `Removed ${gone[0].name}` : `Cleared ${gone.length} meals`, { label: 'Undo', run: () => act(api.restoreMeals(gone), 'Put back') });
      else onnote('No meals to clear here');
    } catch (e) {
      onerror(String(e));
    }
    await load();
  }

  // ---- dragging: a recipe from the picker, or a meal to another day ----
  function dragRecipe(e: DragEvent, c: Card) {
    e.dataTransfer!.setData('application/x-recipe', String(c.id));
    e.dataTransfer!.effectAllowed = 'copy';
  }
  function dragMeal(e: DragEvent, m: Meal) {
    e.dataTransfer!.setData('application/x-meal', String(m.id));
    e.dataTransfer!.effectAllowed = 'move';
  }
  function over(e: DragEvent, day: number, kind: MealType | null = null) {
    const t = e.dataTransfer?.types ?? [];
    if (!t.includes('application/x-recipe') && !t.includes('application/x-meal')) return;
    e.preventDefault();
    e.dataTransfer!.dropEffect = t.includes('application/x-meal') ? 'move' : 'copy';
    dropDay = day;
    dropKind = kind;
  }
  function drop(e: DragEvent, day: number, kind: MealType | null = null) {
    e.preventDefault();
    dropDay = dropKind = null;
    const r = Number(e.dataTransfer?.getData('application/x-recipe'));
    const m = Number(e.dataTransfer?.getData('application/x-meal'));
    if (r) add({ id: r, name: '' }, day, kind);
    else if (m) act(api.updateMeal(m, day, kind));
  }
  /** Clicking a day adds the picked recipe there; without one, it opens that day. */
  function clickDay(day: number, kind: MealType | null = null) {
    if (pick) add(pick, day, kind);
    else if (view !== 'day') { anchor = day; view = 'day'; }
  }
  /** An empty slot or a day's "+": adds the recipe picked on the right, or opens the picker sheet. */
  function addTo(day: number, kind: MealType | null = null) {
    if (pick) add(pick, day, kind);
    else choosing = { day, kind };
  }

  function mealMenu(e: MouseEvent, m: Meal) {
    e.preventDefault();
    e.stopPropagation();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu = {
      x: e.clientX || r.left,
      y: e.clientY || r.bottom,
      items: [
        ...(m.recipe_id != null ? [{ label: 'Open Recipe', icon: 'book', run: () => onopen(m.recipe_id!) }] : []),
        'sep',
        ...kinds.filter((k) => k !== m.kind).map((k) => ({ label: `Make It ${k}`, run: () => act(api.updateMeal(m.id, null, k)) })),
        'sep',
        { label: 'Move a Day Earlier (Alt+←)', run: () => shift(m, -1) },
        { label: 'Move a Day Later (Alt+→)', run: () => shift(m, 1) },
        'sep',
        { label: 'Remove', icon: 'trash', danger: true, run: () => remove([m.id]) },
      ] as MenuItem[],
    };
  }
  /** Move a meal by some days and keep keyboard focus on it (Alt+arrows; also in its menu). */
  async function shift(m: Meal, days: number) {
    const to = m.day + days;
    const away = to < shown[0] || to > shown[1];
    await act(api.updateMeal(m.id, to, null), `Moved to ${api.dayLabel(to)}`);
    if (away) anchor = to; // follow it onto the next page
    requestAnimationFrame(() => document.querySelector<HTMLElement>(`[data-meal="${m.id}"]`)?.focus());
  }
  function mealKeys(e: KeyboardEvent, m: Meal) {
    if (!e.altKey) return;
    const d = ({ ArrowLeft: -1, ArrowRight: 1, ArrowUp: -7, ArrowDown: 7 } as Record<string, number>)[e.key];
    if (!d) return;
    e.preventDefault();
    e.stopPropagation();
    shift(m, d);
  }

  function moreMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const [f, t] = shown;
    menu = {
      x: r.right - 260,
      y: r.bottom + 6,
      items: [
        { label: 'Add a Menu…', icon: 'menu', run: () => api.menus().then((m) => { if (m.length) applying = { menus: m, start: Math.max(f, today) }; else onnote('Make a menu on the Menus page first'); }, (e) => onerror(String(e))) },
        { label: 'Make a Grocery List for These Days…', icon: 'cart', run: () => onplan(f, t) },
        'sep',
        { label: `Clear ${view === 'day' ? 'This Day' : view === 'week' ? 'This Week' : 'This Month'}`, icon: 'trash', danger: true, run: () => remove([], f, t) },
      ],
    };
  }

  function keys(e: KeyboardEvent) {
    if (e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement || menu || choosing) return;
    if (e.key === 'Escape' && pick) pick = null;
    else if (e.key === 'ArrowLeft') step(-1);
    else if (e.key === 'ArrowRight') step(1);
    else if (e.key.toLowerCase() === 't') anchor = today;
    else return;
    e.preventDefault();
  }
  const kindShort: Record<MealType, string> = { Breakfast: 'B', Lunch: 'L', Dinner: 'D', Snack: 'S' };
</script>

<svelte:window onkeydown={keys} ondragend={() => (dropDay = dropKind = null)} />

{#snippet chip(m: Meal, big = false)}
  <button
    class="meal label-scope"
    class:big
    style:--l={api.labelVar(m.label)}
    draggable="true"
    ondragstart={(e) => dragMeal(e, m)}
    onclick={(e) => { e.stopPropagation(); if (m.recipe_id != null) onopen(m.recipe_id); }}
    oncontextmenu={(e) => mealMenu(e, m)}
    onkeydown={(e) => mealKeys(e, m)}
    data-meal={m.id}
    aria-keyshortcuts="Alt+ArrowLeft Alt+ArrowRight Alt+ArrowUp Alt+ArrowDown"
    title="{m.kind}: {m.name}{m.eaten ? ' · eaten (in the diary)' : ''} (Alt+arrow keys move it)"
  >
    {#if big}<span class="thumb"><Photo photo={m.photo} seed={m.seed} /></span>{:else}<span class="bar"></span>{/if}
    <span class="mname">{m.name}</span>
    {#if m.eaten}<span class="eaten"><Icon name="check" size={13} /><span class="sr">Eaten, logged in the diary</span></span>{/if}
    {#if !big}<span class="kind" aria-label={m.kind}>{kindShort[m.kind]}</span>{/if}
  </button>
{/snippet}

<div class="meals" class:picking={!!pick}>
  <section class="cal">
    <header class="top">
      <div class="nav">
        <button class="arrow" aria-label="Previous" onclick={() => step(-1)}><Icon name="back" size={17} /></button>
        <button class="arrow flip" aria-label="Next" onclick={() => step(1)}><Icon name="back" size={17} /></button>
        {#key title}<h1 in:fade={{ duration: 160 }}>{title}</h1>{/key}
        {#if !(grid.days.includes(today) && (view !== 'month' || ymd(today)[1] === grid.month))}
          <button class="today" onclick={() => (anchor = today)}>Today</button>
        {/if}
      </div>
      <div class="tools">
        <Segmented label="View" options={['day', 'week', 'month'] as View[]} value={view} onchange={(v) => (view = v)}>
          {#snippet item(v)}{({ day: 'Day', week: 'Week', month: 'Month' })[v]}{/snippet}
        </Segmented>
        <button class="icon" aria-label="More meal plan actions" aria-haspopup="menu" onclick={moreMenu}><Icon name="more" size={18} /></button>
      </div>
    </header>

    {#if pick}
      <p class="hint" transition:fade={{ duration: 150 }}>
        Click a day to add <strong>{pick.name}</strong>
        <button onclick={() => (pick = null)}>Done</button>
      </p>
    {/if}

    {#if view === 'month'}
      <div class="month" role="grid" aria-label={title}>
        {#each ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'] as w}<div class="wd caps" role="columnheader">{w}</div>{/each}
        {#each grid.days as d (d)}
          {@const list = on(d)}
          <div
            class="cell"
            class:other={ymd(d)[1] !== grid.month}
            class:now={d === today}
            class:drop={dropDay === d}
            role="gridcell"
            tabindex="-1"
            ondragover={(e) => over(e, d)}
            ondragleave={() => { if (dropDay === d) dropDay = null; }}
            ondrop={(e) => drop(e, d)}
            onclick={() => clickDay(d)}
            onkeydown={(e) => { if (e.key === 'Enter') clickDay(d); }}
          >
            <span class="dn num">{ymd(d)[2]}</span>
            {#each list.slice(0, 3) as m (m.id)}{@render chip(m)}{/each}
            {#if list.length > 3}<span class="more">+{list.length - 3} more</span>{/if}
          </div>
        {/each}
      </div>
    {:else if view === 'week'}
      <div class="week">
        {#each grid.days as d (d)}
          <div
            class="col"
            class:now={d === today}
            class:drop={dropDay === d}
            role="group"
            aria-label={api.dayLabel(d, { weekday: 'long', month: 'long', day: 'numeric' })}
            ondragover={(e) => over(e, d)}
            ondragleave={(e) => { if (!(e.currentTarget as HTMLElement).contains(e.relatedTarget as Node)) dropDay = null; }}
            ondrop={(e) => drop(e, d)}
          >
            <button class="colhead" onclick={() => clickDay(d)}>
              <span class="caps">{api.dayLabel(d, { weekday: 'short' })}</span>
              <span class="big num">{ymd(d)[2]}</span>
            </button>
            <div class="stack">
              {#each on(d) as m (m.id)}
                <div class="slot"><span class="k caps">{m.kind}</span>{@render chip(m, true)}</div>
              {/each}
              <button class="addzone" onclick={() => addTo(d)} aria-label="Add to {api.dayLabel(d, { weekday: 'long' })}">
                <Icon name="plus" size={15} />
              </button>
            </div>
          </div>
        {/each}
      </div>
    {:else}
      <div class="day">
        {#each kinds as k}
          {@const list = on(anchor).filter((m) => m.kind === k)}
          <section
            class="kindsec"
            class:drop={dropDay === anchor && dropKind === k}
            aria-label={k}
            ondragover={(e) => over(e, anchor, k)}
            ondragleave={(e) => { if (!(e.currentTarget as HTMLElement).contains(e.relatedTarget as Node)) dropDay = null; }}
            ondrop={(e) => drop(e, anchor, k)}
          >
            <h2 class="caps">{k}</h2>
            <div class="cards">
              {#each list as m (m.id)}{@render chip(m, true)}{/each}
              <button class="dropzone" onclick={() => addTo(anchor, k)}>
                <Icon name="plus" size={16} />
                {pick ? `Add ${pick.name}` : 'Choose a recipe, or drag one here'}
              </button>
            </div>
          </section>
        {/each}
      </div>
    {/if}
  </section>

  <aside class="picker" aria-label="Recipes to plan">
    <label class="shelf">
      <select bind:value={shelf} aria-label="Recipes from">
        <option value="all">All Recipes</option>
        <option value="favorites">Favorites</option>
        {#each categories as c}<option value={'cat:' + c.path}>{c.path.split('/').join(' › ')}</option>{/each}
      </select>
      <Icon name="chevron" size={13} />
    </label>
    <label class="search">
      <Icon name="search" size={15} />
      <input bind:value={search} placeholder="Search" aria-label="Search recipes to plan" spellcheck="false" />
    </label>
    <ul>
      {#each recipes as c (c.id)}
        <li>
          <button
            class="rrow label-scope"
            class:on={pick?.id === c.id}
            style:--l={api.labelVar(c.label_index)}
            draggable="true"
            ondragstart={(e) => dragRecipe(e, c)}
            onclick={() => (pick = pick?.id === c.id ? null : c)}
            ondblclick={() => onopen(c.id)}
            aria-pressed={pick?.id === c.id}
          >
            <span class="rthumb"><Photo photo={c.photo} seed={c.seed} /></span>
            <span class="rname">{c.name}</span>
          </button>
        </li>
      {/each}
    </ul>
  </aside>
</div>

{#if menu}<Menu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />{/if}
{#if applying}
  <ApplyMenuDialog menus={applying.menus} start={applying.start} {today} onclose={() => (applying = null)} ondone={(msg) => { applying = null; onnote(msg); load(); }} {onerror} />
{/if}
{#if choosing}<PlanRecipeDialog day={choosing.day} kind={choosing.kind} onclose={() => (choosing = null)} onadded={load} />{/if}

<style>
  .eaten { display: inline-flex; flex: none; color: var(--ok); }
  /* purple text dark enough to read on both themes; a deeper purple behind white text */
  .meals {
    --meals-ink: color-mix(in srgb, var(--tile-meals), var(--ink) 30%);
    --meals-deep: color-mix(in srgb, var(--tile-meals) 80%, black);
    display: flex;
    min-height: 100%;
  }
  .cal { flex: 1; min-width: 0; display: flex; flex-direction: column; padding: 0 24px 24px 32px; }
  .top {
    position: sticky;
    top: 0;
    z-index: 4;
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 10px 16px;
    min-height: 64px;
    padding: 10px 0;
    background: var(--toolbar);
    backdrop-filter: blur(20px) saturate(170%);
  }
  .nav { flex: 1; display: flex; align-items: center; gap: 4px; min-width: 0; }
  h1 { margin: 0 8px 0 6px; font: 700 26px var(--font-display); letter-spacing: -0.02em; color: var(--meals-ink); white-space: nowrap; }
  .arrow { display: grid; place-items: center; width: 30px; height: 30px; border-radius: 9px; color: var(--ink-2); }
  .arrow:hover { background: var(--well); color: var(--ink); }
  .flip { transform: scaleX(-1); }
  .today { height: 28px; padding: 0 12px; border-radius: 14px; font-size: 13px; font-weight: 600; color: var(--meals-ink); background: color-mix(in srgb, var(--tile-meals) 12%, transparent); }
  .today:hover { background: color-mix(in srgb, var(--tile-meals) 20%, transparent); }
  .tools { display: flex; align-items: center; gap: 8px; }
  .icon { display: grid; place-items: center; width: 32px; height: 30px; border-radius: 9px; color: var(--accent-text); }
  .icon:hover { background: var(--accent-wash); }
  .icon :global(svg) { stroke-width: 2.6; }
  .hint { display: flex; align-items: center; gap: 8px; margin: 0 0 10px; padding: 8px 12px; border-radius: 10px; font-size: 13.5px; background: color-mix(in srgb, var(--tile-meals) 12%, var(--page)); }
  .hint strong { font-weight: 600; }
  .hint button { margin-left: auto; font-weight: 600; color: var(--meals-ink); }

  /* a meal on the calendar: the recipe's label colour, its name, and B/L/D/S */
  .meal {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    min-width: 0;
    height: 22px;
    padding: 0 6px 0 3px;
    border-radius: 6px;
    font-size: 12.5px;
    text-align: left;
    background: color-mix(in srgb, var(--l) 12%, var(--card));
    transition: background-color 120ms ease-out, transform 100ms ease-out, box-shadow 160ms ease-out;
  }
  .meal:hover { background: color-mix(in srgb, var(--l) 22%, var(--card)); }
  .meal:active { transform: scale(0.98); }
  .bar { width: 3px; height: 14px; border-radius: 2px; background: var(--l); flex: none; }
  .mname { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .kind { font: 600 11px var(--font-label); color: var(--l-text); }
  /* the recipe's label colour tints the whole card, as on the month chips */
  .meal.big { height: auto; padding: 6px 10px 6px 6px; gap: 10px; border-radius: 10px; font-size: 14px; background: color-mix(in srgb, var(--l) 10%, var(--card)); box-shadow: var(--shadow-card); }
  .meal.big:hover { background: color-mix(in srgb, var(--l) 18%, var(--card)); box-shadow: var(--shadow-lift); transform: translateY(-1px); }
  .meal.big .mname { white-space: normal; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; }
  .thumb { width: 36px; height: 36px; border-radius: 8px; overflow: hidden; flex: none; }
  /* in a narrow week column the photo sits on top and the name gets the full width */
  .week .meal.big { flex-direction: column; align-items: stretch; gap: 6px; padding: 5px 5px 8px; }
  .week .thumb { width: 100%; height: auto; aspect-ratio: 16 / 10; border-radius: 7px; }
  .week .mname { font-size: 13px; line-height: 1.3; padding: 0 3px; -webkit-line-clamp: 3; line-clamp: 3; hyphens: auto; overflow-wrap: anywhere; }

  .month { flex: 1; display: grid; grid-template-columns: repeat(7, minmax(0, 1fr)); grid-template-rows: 28px; grid-auto-rows: minmax(104px, 1fr); border-left: 1px solid var(--rule); border-top: 1px solid var(--rule); }
  .wd { display: grid; place-items: center; height: 28px; font-size: 13px; color: var(--ink-2); border-right: 1px solid var(--rule); border-bottom: 1px solid var(--rule); background: var(--well); }
  .cell { position: relative; display: flex; flex-direction: column; gap: 3px; min-width: 0; padding: 26px 4px 4px; border-right: 1px solid var(--rule); border-bottom: 1px solid var(--rule); transition: background-color 120ms ease-out; outline: none; }
  .cell:hover { background: color-mix(in srgb, var(--well) 60%, transparent); }
  .other { background: color-mix(in srgb, var(--well) 50%, transparent); }
  .other .dn { color: var(--ink-3); }
  .dn { position: absolute; top: 4px; left: 6px; display: grid; place-items: center; min-width: 22px; height: 20px; padding: 0 4px; border-radius: 10px; font-size: 15px; color: var(--ink-2); }
  .now .dn { color: #fff; background: var(--meals-deep); }
  .more { font-size: 12px; color: var(--ink-2); padding-left: 6px; }
  .drop { background: color-mix(in srgb, var(--tile-meals) 14%, var(--page)) !important; box-shadow: inset 0 0 0 2px var(--tile-meals); }
  .picking .cell:hover, .picking .col:hover { box-shadow: inset 0 0 0 2px color-mix(in srgb, var(--tile-meals) 60%, transparent); }

  .week { flex: 1; display: grid; grid-template-columns: repeat(7, minmax(0, 1fr)); gap: 8px; }
  .col { display: flex; flex-direction: column; gap: 8px; min-width: 0; padding: 8px; border-radius: 14px; background: color-mix(in srgb, var(--well) 55%, transparent); transition: background-color 120ms ease-out; }
  .col.now { background: color-mix(in srgb, var(--tile-meals) 9%, var(--page)); }
  .colhead { display: flex; flex-direction: column; align-items: center; padding: 4px 0 6px; border-radius: 10px; color: var(--ink-2); }
  .colhead:hover { background: var(--well); }
  .colhead .caps { font-size: 13px; }
  .colhead .big { font-size: 26px; line-height: 1.05; color: var(--ink); }
  .now .colhead .big { color: var(--meals-ink); }
  .stack { display: flex; flex-direction: column; gap: 8px; flex: 1; }
  .slot { display: flex; flex-direction: column; gap: 3px; }
  .k { font-size: 11.5px; color: var(--ink-3); padding-left: 4px; }
  .addzone { display: grid; place-items: center; height: 36px; border-radius: 10px; color: var(--ink-3); border: 1.5px dashed var(--rule-strong); opacity: 0; transition: opacity 140ms ease-out, background-color 120ms ease-out; }
  .col:hover .addzone, .addzone:focus-visible, .picking .addzone, .col.drop .addzone { opacity: 1; }
  .addzone:hover { background: var(--card); color: var(--meals-ink); }

  .day { display: flex; flex-direction: column; gap: 18px; max-width: 720px; }
  .kindsec { padding: 12px 14px; border-radius: 14px; background: color-mix(in srgb, var(--well) 55%, transparent); transition: background-color 120ms ease-out; }
  .kindsec h2 { margin: 0 0 10px; font-size: 15px; color: var(--meals-ink); }
  .cards { display: flex; flex-direction: column; gap: 8px; }
  .dropzone { display: flex; align-items: center; gap: 8px; height: 44px; padding: 0 12px; border-radius: 10px; font-size: 13.5px; color: var(--ink-2); border: 1.5px dashed var(--rule-strong); }
  .dropzone:hover { background: var(--card); color: var(--meals-ink); }

  .picker {
    position: sticky;
    top: 0;
    align-self: flex-start;
    width: 260px;
    flex: none;
    height: 100vh;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 14px 10px;
    border-left: 1px solid var(--rule);
    background: var(--sidebar);
  }
  .shelf { position: relative; display: flex; align-items: center; }
  .shelf select { width: 100%; height: 32px; padding: 0 28px 0 10px; border: 0; border-radius: 9px; appearance: none; background: var(--well); box-shadow: inset 0 0 0 1px var(--rule); color: var(--ink); font: 600 13.5px var(--font-text); outline: none; }
  .shelf select option { color: #1f1b18; background: #fff; font-weight: 400; }
  .shelf :global(svg) { position: absolute; right: 10px; transform: rotate(90deg); color: var(--ink-2); pointer-events: none; }
  .search { display: flex; align-items: center; gap: 6px; height: 32px; padding: 0 10px; border-radius: 9px; background: var(--well); box-shadow: inset 0 0 0 1px var(--rule); color: var(--ink-3); }
  .search:focus-within, .shelf select:focus { box-shadow: 0 0 0 3px var(--focus), inset 0 0 0 1px var(--accent); }
  .search input::placeholder { color: var(--ink-3); }
  .search input { flex: 1; min-width: 0; border: 0; outline: none; background: none; color: var(--ink); font: inherit; font-size: 13.5px; -webkit-user-select: text; user-select: text; }
  .picker ul { list-style: none; margin: 0; padding: 0; overflow-y: auto; flex: 1; }
  .rrow { display: flex; align-items: center; gap: 10px; width: 100%; padding: 5px 6px; border-radius: 9px; text-align: left; transition: background-color 120ms ease-out; }
  .rrow:hover { background: var(--sidebar-hover); }
  .rrow.on { background: var(--meals-deep); color: #fff; }
  .rrow:active { cursor: grabbing; }
  .rthumb { width: 32px; height: 32px; border-radius: 8px; overflow: hidden; flex: none; box-shadow: inset 0 0 0 2px var(--l); }
  .rname { flex: 1; min-width: 0; font-size: 13.5px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

  @media (max-width: 1300px) {
    .picker { width: 200px; }
    .cal { padding: 0 14px 20px 20px; }
    .week { gap: 4px; }
    .col { padding: 6px 4px; }
  }
</style>
