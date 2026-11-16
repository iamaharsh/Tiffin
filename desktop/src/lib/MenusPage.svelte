<script lang="ts">
  import { untrack } from 'svelte';
  import { slide, fade } from 'svelte/transition';
  import Icon from './Icon.svelte';
  import Photo from './Photo.svelte';
  import Dialog from './Dialog.svelte';
  import Menu, { type MenuItem } from './Menu.svelte';
  import ApplyMenuDialog from './ApplyMenuDialog.svelte';
  import * as api from './api';
  import type { Card, MealType, MenuEntry, MenuOut } from './api';

  let { today, onopen, onerror, onnote }: {
    today: number;
    onopen: (recipeId: number) => void;
    onerror: (m: string) => void;
    onnote: (m: string, action?: { label: string; run: () => void } | null) => void;
  } = $props();

  const remembered = (k: string) => { try { return localStorage.getItem(k); } catch { return null; } };
  const remember = (k: string, v: string) => { try { localStorage.setItem(k, v); } catch { /* private mode */ } };

  let all: MenuOut[] = $state([]);
  let sel = $state(Number(remembered('menu') ?? 0));
  let name = $state('');
  let applying = $state(false);
  let adding: { day: number; kind: MealType; q: string; found: Card[] } | null = $state(null);
  let menu: { x: number; y: number; items: MenuItem[] } | null = $state(null);
  let dropDay: number | null = $state(null);

  const m = $derived(all.find((x) => x.id === sel) ?? all[0]);
  const hidden = $derived(m ? m.entries.filter((e) => e.day > m.days).length : 0);
  const kinds: MealType[] = ['Breakfast', 'Lunch', 'Dinner', 'Snack'];
  const order = (k: MealType) => kinds.indexOf(k);

  async function load() {
    try {
      all = await api.menus();
      if (m) {
        sel = m.id;
        name = m.name;
        remember('menu', String(sel));
      }
    } catch (e) {
      onerror(String(e));
    }
  }
  $effect(() => { untrack(load); });

  const plain = (e: MenuEntry): MenuEntry => ({ day: e.day, kind: e.kind, recipe_id: e.recipe_id, name: e.name });
  /** Save the open menu with a change to its entries or length. */
  async function save(entries = m!.entries.map(plain), days = m!.days) {
    try {
      await api.saveMenu(m!.id, name, days, entries);
    } catch (e) {
      onerror(String(e));
    }
    await load();
  }

  async function create() {
    try {
      sel = await api.newMenu();
    } catch (e) {
      onerror(String(e));
    }
    await load();
    document.querySelector<HTMLInputElement>('.mtitle')?.select();
  }

  async function del(id: number) {
    try {
      const gone = await api.deleteMenu(id);
      onnote('Deleted the menu', { label: 'Undo', run: () => api.restoreMenu(gone).then(load, (e) => onerror(String(e))) });
    } catch (e) {
      onerror(String(e));
    }
    sel = 0;
    await load();
  }

  function listMenu(e: MouseEvent, x: MenuOut) {
    e.preventDefault();
    menu = { x: e.clientX, y: e.clientY, items: [{ label: 'Delete Menu', icon: 'trash', danger: true, run: () => del(x.id) }] };
  }

  // ---- adding a recipe to a day ----
  function openAdd(day: number) {
    adding = { day, kind: 'Dinner', q: '', found: [] };
  }
  $effect(() => {
    if (!adding) return;
    const q = adding.q;
    api.list('all', q, 0).then((c) => { if (adding) adding.found = c; }, (e) => onerror(String(e)));
  });
  function pickRecipe(c: Card) {
    const a = adding!;
    adding = null;
    save([...m!.entries.map(plain), { day: a.day, kind: a.kind, recipe_id: c.id, name: c.name }]);
  }
  const removeEntry = (i: number) => save(m!.entries.map(plain).filter((_, j) => j !== i));

  // drag a recipe to another day of the menu
  function dragEntry(e: DragEvent, i: number) {
    e.dataTransfer!.setData('application/x-menu-entry', String(i));
    e.dataTransfer!.effectAllowed = 'move';
  }
  function over(e: DragEvent, day: number) {
    if (!e.dataTransfer?.types.includes('application/x-menu-entry')) return;
    e.preventDefault();
    dropDay = day;
  }
  function drop(e: DragEvent, day: number) {
    e.preventDefault();
    dropDay = null;
    const i = Number(e.dataTransfer?.getData('application/x-menu-entry'));
    const list = m!.entries.map(plain);
    if (Number.isNaN(i) || !list[i] || list[i].day === day) return;
    list[i].day = day;
    save(list);
  }
</script>

<svelte:window ondragend={() => (dropDay = null)} />

<div class="menus">
  <aside class="lists" aria-label="Menus">
    <div class="lhead">
      <h2>Menus</h2>
      <button class="icon" aria-label="New menu" title="New menu" onclick={create}><Icon name="plus" size={17} /></button>
    </div>
    <ul>
      {#each all as x (x.id)}
        <li>
          <button class="lrow" class:sel={x.id === m?.id} aria-current={x.id === m?.id ? 'page' : undefined} onclick={() => { sel = x.id; name = x.name; remember('menu', String(sel)); }} oncontextmenu={(e) => listMenu(e, x)}>
            <span class="ldot"><Icon name="menu" size={14} /></span>
            <span class="lname">{x.name}</span>
            <span class="lcount num">{x.days}d</span>
          </button>
        </li>
      {/each}
    </ul>
  </aside>

  <section class="main">
    {#if !m}
      <div class="empty">
        <span class="etile"><Icon name="menu" size={30} /></span>
        <h2>No menus yet</h2>
        <p>A menu is a set of meals over a few days, like a favourite week of dinners, that you can put on the meal plan any time.</p>
        <button class="primary" onclick={create}><Icon name="plus" size={16} /> New Menu</button>
      </div>
    {:else}
      <header class="top">
        <input
          class="mtitle"
          bind:value={name}
          aria-label="Menu name"
          onblur={() => { if (name.trim() !== m.name) save(); }}
          onkeydown={(e) => { if (e.key === 'Enter') e.currentTarget.blur(); }}
          autocomplete="off"
        />
        <div class="days" aria-label="Days">
          <button aria-label="Fewer days" disabled={m.days <= 1} onclick={() => save(undefined, m.days - 1)}>−</button>
          <span class="num">{m.days} day{m.days === 1 ? '' : 's'}</span>
          <button aria-label="More days" disabled={m.days >= 28} onclick={() => save(undefined, m.days + 1)}>+</button>
        </div>
        <button class="primary" onclick={() => (applying = true)} disabled={!m.entries.some((e) => e.day <= m.days)}><Icon name="calendar" size={16} /> Add to Meal Plan…</button>
        <button class="icon" aria-label="More menu actions" aria-haspopup="menu" onclick={(e) => { const r = e.currentTarget.getBoundingClientRect(); menu = { x: r.right - 200, y: r.bottom + 6, items: [{ label: 'Delete Menu', icon: 'trash', danger: true, run: () => del(m.id) }] }; }}><Icon name="more" size={18} /></button>
      </header>

      {#if hidden}
        <p class="warn" transition:slide={{ duration: 160 }}><Icon name="warn" size={15} /> {hidden} recipe{hidden === 1 ? '' : 's'} on days after Day {m.days} {hidden === 1 ? 'is' : 'are'} hidden and won’t be added. Add days to see {hidden === 1 ? 'it' : 'them'}.</p>
      {/if}

      <div class="grid">
        {#each Array.from({ length: m.days }, (_, i) => i + 1) as d (d)}
          {@const list = m.entries.map((e, i) => ({ e, i })).filter(({ e }) => e.day === d).sort((a, b) => order(a.e.kind) - order(b.e.kind))}
          <section
            class="day"
            class:drop={dropDay === d}
            aria-label="Day {d}"
            ondragover={(e) => over(e, d)}
            ondragleave={(e) => { if (!(e.currentTarget as HTMLElement).contains(e.relatedTarget as Node)) dropDay = null; }}
            ondrop={(e) => drop(e, d)}
            in:fade={{ duration: 160 }}
          >
            <h3 class="caps">Day {d}</h3>
            {#each list as { e, i } (i)}
              <div class="entry label-scope" style:--l={api.labelVar(e.label)} draggable="true" ondragstart={(ev) => dragEntry(ev, i)} role="listitem">
                <span class="k caps">{e.kind}</span>
                <div class="erow">
                  <button class="recipe" onclick={() => e.recipe_id != null && onopen(e.recipe_id)} title="Open {e.name}">
                    <span class="thumb"><Photo photo={e.photo} seed={e.seed} /></span>
                    <span class="ename">{e.name}</span>
                  </button>
                  <button class="x" aria-label="Remove {e.name} from Day {d}" onclick={() => removeEntry(i)}><Icon name="close" size={13} /></button>
                </div>
              </div>
            {/each}
            <button class="add" onclick={() => openAdd(d)}><Icon name="plus" size={14} /> Add Recipe</button>
          </section>
        {/each}
      </div>
    {/if}
  </section>
</div>

{#if menu}<Menu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />{/if}

{#if applying && m}
  <ApplyMenuDialog menus={all} menuId={m.id} {today} onclose={() => (applying = false)} ondone={(msg) => { applying = false; onnote(msg); }} {onerror} />
{/if}

{#if adding}
  {@const a = adding}
  <Dialog title="Add to Day {a.day}" onclose={() => (adding = null)} wide>
    <div class="kinds" role="radiogroup" aria-label="Meal">
      {#each kinds as k}<label class="check"><input type="radio" name="addkind" value={k} bind:group={a.kind} /> {k}</label>{/each}
    </div>
    <!-- svelte-ignore a11y_autofocus -->
    <input bind:value={a.q} placeholder="Search recipes" aria-label="Search recipes" autofocus spellcheck="false" onkeydown={(e) => { if (e.key === 'Enter' && a.found[0]) pickRecipe(a.found[0]); }} />
    <ul class="found">
      {#each a.found.slice(0, 60) as c (c.id)}
        <li>
          <button class="frow label-scope" style:--l={api.labelVar(c.label_index)} onclick={() => pickRecipe(c)}>
            <span class="fthumb"><Photo photo={c.photo} seed={c.seed} /></span>
            <span class="fname">{c.name}</span>
            <span class="fchip caps">{c.label}</span>
          </button>
        </li>
      {:else}
        <li class="none">No recipes match</li>
      {/each}
    </ul>
    {#snippet actions()}<button class="btn" onclick={() => (adding = null)}>Cancel</button>{/snippet}
  </Dialog>
{/if}

<style>
  .menus {
    --menus-ink: color-mix(in srgb, var(--tile-menus), var(--ink) 35%);
    --menus-deep: color-mix(in srgb, var(--tile-menus) 82%, black);
    display: flex;
    min-height: 100%;
  }
  .lists { position: sticky; top: 0; align-self: flex-start; width: 220px; flex: none; height: 100vh; padding: 18px 10px; border-right: 1px solid var(--rule); overflow-y: auto; }
  .lhead { display: flex; align-items: center; justify-content: space-between; padding: 0 6px 0 10px; margin-bottom: 8px; }
  .lhead h2 { margin: 0; font: 700 15px var(--font-display); color: var(--ink-2); }
  ul { list-style: none; margin: 0; padding: 0; }
  .lrow { display: flex; align-items: center; gap: 9px; width: 100%; height: 34px; padding: 0 10px; border-radius: 9px; text-align: left; transition: background-color 120ms ease-out; }
  .lrow:hover { background: var(--sidebar-hover); }
  .lrow.sel { background: var(--menus-deep); color: #fff; }
  .ldot { display: grid; place-items: center; width: 22px; height: 22px; border-radius: 6px; color: #fff; background: var(--tile-menus); flex: none; }
  .lrow.sel .ldot { background: rgba(255, 255, 255, 0.22); }
  .lname { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-weight: 500; }
  .lcount { font-size: 14px; color: var(--ink-3); }
  .lrow.sel .lcount { color: rgba(255, 255, 255, 0.85); }
  .icon { display: grid; place-items: center; width: 32px; height: 30px; border-radius: 9px; color: var(--accent-text); flex: none; }
  .icon:hover { background: var(--accent-wash); }
  .icon :global(svg) { stroke-width: 2.4; }

  .main { flex: 1; min-width: 0; padding: 0 32px 64px; }
  .top { position: sticky; top: 0; z-index: 4; display: flex; align-items: center; flex-wrap: wrap; gap: 10px; min-height: 64px; padding: 10px 0; background: var(--toolbar); backdrop-filter: blur(20px) saturate(170%); }
  .mtitle {
    flex: 0 1 auto;
    field-sizing: content; /* as wide as the name, up to the space there is */
    min-width: 160px;
    max-width: 100%;
    margin-right: auto;
    padding: 4px 8px;
    margin-left: -8px;
    border: 0;
    border-radius: 8px;
    background: none;
    color: var(--menus-ink);
    font: 700 26px var(--font-display);
    letter-spacing: -0.02em;
    outline: none;
    -webkit-user-select: text;
    user-select: text;
    transition: background-color 150ms ease-out, box-shadow 150ms ease-out;
  }
  .mtitle:hover { background: var(--well); }
  .mtitle:focus { background: var(--card); box-shadow: 0 0 0 3px var(--focus), inset 0 0 0 1px var(--accent); }
  .days { display: flex; align-items: center; gap: 2px; padding: 2px; border-radius: 10px; background: var(--well); box-shadow: inset 0 0 0 1px var(--rule); }
  .days button { width: 28px; height: 26px; border-radius: 8px; font-size: 16px; }
  .days button:hover:not(:disabled) { background: var(--card); box-shadow: var(--shadow-card); }
  .days button:disabled { color: var(--ink-3); }
  .days span { min-width: 62px; text-align: center; font-size: 15px; }
  .primary { display: inline-flex; align-items: center; gap: 7px; height: 32px; padding: 0 14px; border-radius: 9px; font-size: 13.5px; font-weight: 600; color: #fff; background: var(--accent); box-shadow: inset 0 -1px 0 rgba(0, 0, 0, 0.15); transition: background-color 120ms ease-out, transform 100ms ease-out; }
  .primary:hover:not(:disabled) { background: var(--accent-press); }
  .primary:active:not(:disabled) { transform: scale(0.97); }
  .primary:disabled { opacity: 0.5; }
  .warn { display: flex; align-items: center; gap: 8px; margin: 0 0 12px; padding: 9px 12px; border-radius: 10px; font-size: 13.5px; color: var(--warn); background: var(--warn-wash); }

  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(210px, 1fr)); gap: 26px 22px; padding-top: 10px; }
  /* a day is a heading over its recipes (no box around them) */
  .day { display: flex; flex-direction: column; gap: 8px; padding: 6px; margin: -6px; border-radius: 14px; transition: background-color 120ms ease-out, box-shadow 120ms ease-out; }
  .day.drop { background: color-mix(in srgb, var(--tile-menus) 12%, var(--page)); box-shadow: inset 0 0 0 2px var(--tile-menus); }
  h3 { margin: 0 0 4px; padding-bottom: 6px; font-size: 20px; color: var(--menus-ink); border-bottom: 2px solid color-mix(in srgb, var(--tile-menus) 45%, transparent); }
  .entry { display: flex; flex-direction: column; gap: 3px; }
  .k { font-size: 11.5px; color: var(--ink-3); padding-left: 2px; }
  .erow { display: flex; align-items: center; gap: 4px; border-radius: 10px; background: color-mix(in srgb, var(--l) 10%, var(--card)); box-shadow: var(--shadow-card); transition: background-color 120ms ease-out, box-shadow 160ms ease-out; }
  .erow:hover { background: color-mix(in srgb, var(--l) 18%, var(--card)); box-shadow: var(--shadow-lift); }
  .entry:active { cursor: grabbing; }
  .recipe { flex: 1; min-width: 0; display: flex; align-items: center; gap: 9px; padding: 5px; text-align: left; }
  .thumb { width: 34px; height: 34px; border-radius: 8px; overflow: hidden; flex: none; }
  .ename { flex: 1; min-width: 0; font-size: 13.5px; line-height: 1.3; overflow-wrap: anywhere; }
  .x { display: grid; place-items: center; width: 26px; height: 26px; margin-right: 4px; border-radius: 7px; color: var(--ink-3); opacity: 0; transition: opacity 120ms ease-out; }
  .erow:hover .x, .x:focus-visible { opacity: 1; }
  .x:hover { background: var(--rule); color: var(--ink); }
  .add { display: flex; align-items: center; justify-content: center; gap: 6px; height: 34px; border-radius: 10px; font-size: 13px; color: var(--ink-2); border: 1.5px dashed var(--rule-strong); transition: background-color 120ms ease-out, color 120ms ease-out; }
  .add:hover { background: var(--card); color: var(--menus-ink); }

  .empty { margin: 14vh auto 0; max-width: 440px; text-align: center; color: var(--ink-2); }
  .etile { display: inline-grid; place-items: center; width: 64px; height: 64px; border-radius: 18px; color: #fff; background: var(--tile-menus); box-shadow: var(--shadow-lift); }
  .empty h2 { margin: 16px 0 6px; font: 700 20px var(--font-display); color: var(--ink); }
  .empty p { margin: 0 0 18px; }

  .kinds { display: flex; gap: 18px; flex-wrap: wrap; }
  .found { max-height: 340px; overflow-y: auto; margin: 0 -6px; padding: 0 2px; }
  .frow { display: flex; align-items: center; gap: 10px; width: 100%; padding: 5px 6px; border-radius: 9px; text-align: left; color: var(--ink); }
  .frow:hover, .frow:focus-visible { background: var(--well); }
  .fthumb { width: 32px; height: 32px; border-radius: 8px; overflow: hidden; flex: none; }
  .fname { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .fchip { font-size: 12px; color: var(--l-text); }
  .none { padding: 12px 6px; color: var(--ink-3); }

  @media (max-width: 900px) {
    .lists { width: 170px; }
    .main { padding: 0 18px 48px; }
  }
</style>
