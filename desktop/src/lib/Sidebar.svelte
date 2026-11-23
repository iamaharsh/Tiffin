<script lang="ts">
  import { tick } from 'svelte';
  import Icon from './Icon.svelte';
  import Menu, { type MenuItem } from './Menu.svelte';
  import { labelVar, type Overview, type Section } from './api';

  let {
    ov, section, view, waiting = 0, onsection, onview, onpinned, onorder, onlaunch,
  }: {
    ov: Overview | null;
    section: Section;
    view: string;
    /** imported recipes waiting for review (badge on Browser) */
    waiting?: number;
    onsection: (s: Section) => void;
    onview: (v: string) => void;
    onpinned: (id: number) => void;
    /** store a new tab order (the whole list); settles once the saved order is back in `ov` */
    onorder: (order: Section[]) => Promise<void>;
    /** make a tab the one Tiffin opens on */
    onlaunch: (tab: Section) => void;
  } = $props();

  const sections: [Section, string, string][] = [
    ['home', 'Home', 'home'],
    ['reports', 'Reports', 'report'],
    ['diary', 'Diary', 'list'],
    ['targets', 'Targets', 'target'],
    ['body', 'Body', 'trend'],
    ['recipes', 'Recipes', 'book'],
    ['groceries', 'Groceries', 'cart'],
    ['pantry', 'Pantry', 'jar'],
    ['foods', 'Foods', 'flame'],
    ['meals', 'Meal Plan', 'calendar'],
    ['menus', 'Menus', 'menu'],
    ['browser', 'Browser', 'globe'],
  ];
  const library: [string, string, string, number][] = [
    ['all', 'All Recipes', 'home', 0],
    ['recent', 'Most Recent', 'clock', 1],
    ['top', 'Top Rated', 'star', 2],
    ['favorites', 'Favorites', 'heart', 3],
    ['uncategorized', 'Uncategorized', 'tray', 4],
    ['trash', 'Trash', 'trash', 5],
  ];

  // The tabs in the person's order (the crate cleans it); during a drag, the order being previewed.
  const meta = new Map(sections.map(([id, name, icon]) => [id, { name, icon }]));
  const defaultOrder = sections.map(([id]) => id);
  const order = $derived((ov?.sidebar ?? defaultOrder).filter((id) => meta.has(id)));
  const launch = $derived(ov?.launch ?? 'home');
  let preview: Section[] | null = $state(null);
  const shown = $derived(preview ?? order);
  let said = $state(''); // what a reorder did, for screen readers
  let tabMenu: { x: number; y: number; items: MenuItem[]; id: Section } | null = $state(null);
  let list: HTMLUListElement | undefined = $state();

  const same = (a: Section[], b: Section[]) => a.length === b.length && a.every((x, i) => x === b[i]);
  /** save `next`; the preview (if any) stays until the saved order is back, so a dropped tab never snaps back first */
  async function commit(next: Section[], id: Section) {
    if (same(next, order)) return;
    preview = next;
    try {
      await onorder(next);
    } finally {
      preview = null;
    }
    said = '';
    await tick(); // the same sentence twice in a row is still announced
    said = `Moved ${meta.get(id)?.name} to position ${next.indexOf(id) + 1} of ${next.length}.`;
  }
  /** a move from the menu keeps the keyboard on that tab's dots button, wherever the row went */
  async function move(id: Section, next: Section[]) {
    await commit(next, id);
    await tick();
    list?.querySelector<HTMLElement>(`li[data-tab="${id}"] .dots`)?.focus();
  }
  function swapped(id: Section, by: -1 | 1): Section[] {
    const next = [...order];
    const i = next.indexOf(id);
    const j = i + by;
    if (i >= 0 && j >= 0 && j < next.length) [next[i], next[j]] = [next[j], next[i]];
    return next;
  }
  function openTabMenu(id: Section, x: number, y: number) {
    if (dragging) return;
    cancelPress();
    const i = order.indexOf(id);
    tabMenu = {
      x, y, id,
      items: [
        id === launch ? { label: 'Tiffin Opens Here', icon: 'check', disabled: true, run: () => {} } : { label: 'Make Launch Page', icon: 'home', run: () => onlaunch(id) },
        'sep',
        { label: 'Move Up', disabled: i <= 0, run: () => move(id, swapped(id, -1)) },
        { label: 'Move Down', disabled: i >= order.length - 1, run: () => move(id, swapped(id, 1)) },
        'sep',
        { label: 'Reset Order', disabled: same(order, defaultOrder), run: () => move(id, [...defaultOrder]) },
      ],
    };
  }
  /** the dots button opens the menu under itself; a right-click (or the keyboard's menu key) opens it at the pointer or the row */
  function fromButton(id: Section, e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    openTabMenu(id, r.left, r.bottom + 4);
  }
  function fromContext(id: Section, e: MouseEvent) {
    e.preventDefault();
    if (e.clientX === 0 && e.clientY === 0) {
      const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
      openTabMenu(id, r.left + 24, r.bottom + 4);
    } else openTabMenu(id, e.clientX, e.clientY);
  }

  // Long-press to drag: a press held 350 ms without moving lifts the tab, and it follows the pointer until let go. A press
  // that moves first, or is let go sooner, is an ordinary click. Esc puts everything back. Once lifted, the window follows
  // the pointer (no pointer capture: reordering moves the row's element, which would drop a capture and end the drag).
  // a hand on a mouse or trackpad drifts a few pixels during the hold; a press that wanders under SLOP is still a hold (and still a click)
  const HOLD_MS = 350, SLOP = 10;
  let press: { id: Section; x: number; y: number; timer: number } | null = null;
  let dragging: Section | null = $state(null);
  /** the tab being held, before it lifts: it darkens over HOLD_MS, so the hold shows that something is coming */
  let holding: Section | null = $state(null);
  let swallowClick = false;
  function down(id: Section, e: PointerEvent) {
    if (e.button !== 0) return;
    swallowClick = false;
    holding = id;
    if (press) clearTimeout(press.timer); // a second press never leaves the first one's timer running
    press = { id, x: e.clientX, y: e.clientY, timer: window.setTimeout(() => {
      if (!press) return;
      holding = null;
      dragging = press.id;
      preview = [...order];
    }, HOLD_MS) };
  }
  function moveTo(e: PointerEvent) {
    if (!press) return;
    if (!dragging) {
      if (Math.hypot(e.clientX - press.x, e.clientY - press.y) > SLOP) cancelPress();
      return;
    }
    const held = dragging;
    const rows = [...(list?.querySelectorAll<HTMLElement>(':scope > li') ?? [])];
    // the slot is the number of other rows whose middle is above the pointer
    let slot = 0;
    rows.forEach((li) => {
      if (li.dataset.tab === held) return;
      const r = li.getBoundingClientRect();
      if (e.clientY > r.top + r.height / 2) slot++;
    });
    const next = (preview ?? order).filter((t) => t !== held);
    next.splice(slot, 0, held);
    if (!same(next, preview ?? order)) preview = next;
  }
  function up() {
    if (dragging && preview) {
      const next = preview, id = dragging;
      if (press) clearTimeout(press.timer);
      press = null;
      holding = null;
      dragging = null;
      // the click that may follow this release isn't a choice; released over another row, no click comes, so don't wait for one
      swallowClick = true;
      setTimeout(() => (swallowClick = false), 0);
      if (same(next, order)) preview = null;
      else commit(next, id);
      return;
    }
    cancelPress();
  }
  function cancelPress() {
    if (press) clearTimeout(press.timer);
    press = null;
    holding = null;
    dragging = null;
    preview = null;
  }
  /** a press let go, or slid off its row, before the hold time is an ordinary press, never a drag that starts later */
  function released() {
    if (press && !dragging) cancelPress();
  }
  $effect(() => () => { if (press) clearTimeout(press.timer); });
  function choose(id: Section) {
    if (swallowClick) { swallowClick = false; return; }
    onsection(id);
  }

  let open: Record<string, boolean> = $state({});
  const tops = $derived(ov?.categories.filter((c) => c.depth === 0) ?? []);
  const children = (path: string) => ov?.categories.filter((c) => c.depth === 1 && c.path.startsWith(path + '/')) ?? [];
</script>

<!-- a lifted drag ends on any release; one let go outside the window shows up as a move with no button held -->
<svelte:window
  onpointermove={(e) => { if (!dragging) return; if (e.buttons === 0) up(); else moveTo(e); }}
  onpointerup={() => { if (dragging) up(); else released(); }}
  onpointercancel={() => { if (dragging) cancelPress(); }}
  onblur={() => { if (dragging) cancelPress(); }}
  onkeydown={(e) => { if (e.key === 'Escape' && dragging) { e.preventDefault(); cancelPress(); } }} />

<nav class="sidebar" aria-label="Main">
  <ul class="sections" class:dragging={dragging != null} bind:this={list}>
    {#each shown as id (id)}
      {@const m = meta.get(id)}
      {#if m}
        <li data-tab={id} class:lifted={dragging === id} class:holding={holding === id} class:menu-open={tabMenu?.id === id}>
          <button
            class="row"
            class:sel={section === id}
            aria-current={section === id ? 'page' : undefined}
            onclick={() => choose(id)}
            oncontextmenu={(e) => fromContext(id, e)}
            onpointerdown={(e) => down(id, e)}
            onpointermove={(e) => { if (!dragging) moveTo(e); }}
            onpointercancel={cancelPress}
            onpointerleave={released}
          >
            <span class="tile" style:background="var(--tile-{id})"><Icon name={m.icon} size={15} /></span>
            <span class="name">{m.name}</span>
            {#if id === 'browser' && waiting}<span class="badge num" aria-label="{waiting} waiting for review">{waiting}</span>{/if}
          </button>
          <button type="button" class="dots" aria-label="More actions for {m.name}" aria-haspopup="menu" aria-expanded={tabMenu?.id === id} onclick={(e) => fromButton(id, e)}><Icon name="more-v" size={16} /></button>
        </li>
      {/if}
    {/each}
  </ul>
  <span class="sr" role="status">{said}</span>

  {#if section === 'recipes' && ov}
    <div class="scroll">
      <h2 class="caps">Library</h2>
      <ul>
        {#each library as [id, name, icon, ci]}
          <li>
            <button class="row" class:sel={view === id} aria-current={view === id ? 'page' : undefined} onclick={() => onview(id)}>
              <span class="glyph"><Icon name={icon} size={17} /></span>
              <span class="name">{name}</span>
              {#if ov.counts[ci] && id !== 'recent'}<span class="count num">{ov.counts[ci]}</span>{/if}
            </button>
          </li>
        {/each}
      </ul>

      {#if ov.pinned.length}
        <h2 class="caps">Pinned</h2>
        <ul>
          {#each ov.pinned as p (p.id)}
            <li>
              <button class="row" onclick={() => onpinned(p.id)}>
                <span class="glyph"><Icon name="pin" size={15} /></span>
                <span class="name">{p.name}</span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}

      {#if tops.length}
        <h2 class="caps">Categories</h2>
        <ul>
          {#each tops as c (c.path)}
            {@const kids = children(c.path)}
            <li>
              <div class="cat">
                {#if kids.length}
                  <button
                    class="disclose"
                    class:open={open[c.path]}
                    aria-label="{open[c.path] ? 'Hide' : 'Show'} {c.name} subcategories"
                    aria-expanded={!!open[c.path]}
                    onclick={() => (open[c.path] = !open[c.path])}><Icon name="chevron" size={12} /></button
                  >
                {/if}
                <button class="row" class:sel={view === 'cat:' + c.path} aria-current={view === 'cat:' + c.path ? 'page' : undefined} onclick={() => onview('cat:' + c.path)}>
                  <span class="glyph"><span class="swatch" style:background={labelVar(c.label)}></span></span>
                  <span class="name">{c.name}</span>
                  {#if c.count}<span class="count num">{c.count}</span>{/if}
                </button>
              </div>
              {#if kids.length}
                <div class="kids" class:open={open[c.path]} inert={!open[c.path]}>
                  <ul class="kids-inner">
                    {#each kids as k (k.path)}
                      <li>
                        <button class="row sub" class:sel={view === 'cat:' + k.path} onclick={() => onview('cat:' + k.path)}>
                          <span class="name">{k.name}</span>
                          {#if k.count}<span class="count num">{k.count}</span>{/if}
                        </button>
                      </li>
                    {/each}
                  </ul>
                </div>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}

  <div class="foot">
    <button class="row" class:sel={section === 'settings'} aria-current={section === 'settings' ? 'page' : undefined} onclick={() => onsection('settings')}>
      <span class="glyph"><Icon name="gear" size={17} /></span>
      <span class="name">Settings</span>
    </button>
  </div>

  {#if ov?.locked}
    <p class="note" role="status"><Icon name="warn" size={15} /> {ov.locked}</p>
  {/if}
</nav>
<!-- outside the nav: the sidebar is its own layer (view-transition-name), so a menu inside it would sit under the page -->
{#if tabMenu}
  <Menu x={tabMenu.x} y={tabMenu.y} items={tabMenu.items} onclose={() => (tabMenu = null)} />
{/if}

<style>
  .sidebar {
    view-transition-name: sidebar;
    display: flex;
    flex-direction: column;
    width: 244px;
    flex: none;
    height: 100%;
    background: var(--sidebar);
    border-right: 1px solid var(--rule);
    padding: 10px 0 0;
  }
  @media not (prefers-reduced-transparency: reduce) {
    :global(html.mica) .sidebar { background: color-mix(in srgb, var(--sidebar) 62%, transparent); }
  }
  ul { list-style: none; margin: 0; padding: 0 10px; }
  .sections { padding-bottom: 10px; border-bottom: 1px solid var(--rule); }
  .scroll { flex: 1; overflow-y: auto; padding-bottom: 16px; scrollbar-width: thin; }
  h2 {
    margin: 16px 20px 4px;
    font-size: 12.5px;
    color: var(--ink-3);
  }
  .row {
    position: relative;
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    height: 32px;
    padding: 0 10px;
    border-radius: 8px;
    text-align: left;
    color: var(--ink);
    transition: background-color 120ms ease-out;
  }
  .sections .row { height: 36px; font-weight: 600; padding-right: 36px; -webkit-touch-callout: none; user-select: none; touch-action: none; }
  .sections li { position: relative; }
  /* the tab menu button stays quiet: shown on hover, on keyboard focus anywhere in the row, and while its menu is open */
  .dots { position: absolute; right: 6px; top: 6px; display: grid; place-items: center; width: 24px; height: 24px; border-radius: 6px; color: var(--ink-2); opacity: 0; transition: opacity 120ms ease-out; }
  .sections li:hover .dots, .sections li:focus-within .dots, .sections li.menu-open .dots { opacity: 1; }
  .dots:hover { background: var(--rule-strong); color: var(--ink); }
  .dots :global(svg) { stroke-width: 2.4; }
  .sections.dragging .row { cursor: grabbing; }
  /* held: the row darkens over the hold time (its edges stay put, so a press near the side is never lost); lifted: bigger,
     outlined and floating, so it never looks like the selected tab */
  .sections li.holding .row { background: var(--rule-strong); transition: background-color 350ms linear; /* HOLD_MS */ }
  .sections li.lifted { z-index: 1; }
  .sections li.lifted .row { background: var(--card); box-shadow: var(--shadow-float), 0 0 0 2px var(--accent); transform: scale(1.04); transition: transform 140ms var(--ease-out); }
  .sections.dragging .dots { opacity: 0; }
  /* no hover on a touch screen: the button is always there to find */
  @media (hover: none) { .dots { opacity: 1; } }
  .row:hover { background: var(--sidebar-hover); }
  .row:active { background: var(--rule-strong); }
  .row.sel { background: var(--accent); color: #fff; }
  .row.sel .glyph, .row.sel .count { color: rgba(255, 255, 255, 0.85); }
  .sections .row.sel { background: var(--card); color: var(--ink); box-shadow: var(--shadow-card); }
  .tile {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 7px;
    color: #fff;
    flex: none;
    box-shadow: inset 0 -1px 0 rgba(0, 0, 0, 0.15), inset 0 1px 0 rgba(255, 255, 255, 0.25);
  }
  .tile :global(svg) { stroke-width: 1.9; }
  .glyph { display: grid; place-items: center; width: 20px; color: var(--ink-2); flex: none; }
  .swatch { width: 11px; height: 11px; border-radius: 3px; box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.1); }
  .row.sel .swatch { box-shadow: 0 0 0 1.5px #fff; }
  .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .count { font-size: 14px; color: var(--ink-3); }
  .badge {
    min-width: 20px;
    height: 20px;
    padding: 0 6px;
    border-radius: 10px;
    font-size: 13px;
    line-height: 20px;
    text-align: center;
    color: #fff;
    background: var(--tile-browser);
    animation: badge-in 360ms var(--ease-spring);
  }
  @keyframes badge-in { from { transform: scale(0.4); opacity: 0; } }
  .cat { position: relative; }
  .disclose {
    position: absolute;
    left: -8px;
    top: 7px;
    z-index: 1;
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border-radius: 4px;
    color: var(--ink-3);
    transition: transform 200ms var(--ease-out);
  }
  .disclose.open { transform: rotate(90deg); }
  .disclose:hover { color: var(--ink); }
  .cat:has(.sel) .disclose { color: var(--ink-2); }
  /* subcategories open by animating the row height (grid 0fr → 1fr) */
  .kids {
    display: grid;
    grid-template-rows: 0fr;
    padding: 0;
    transition: grid-template-rows 240ms var(--ease-out), opacity 200ms ease-out;
    opacity: 0;
  }
  .kids.open { grid-template-rows: 1fr; opacity: 1; }
  .kids-inner { overflow: hidden; padding: 0; }
  .sub { padding-left: 40px; height: 28px; font-size: 13.5px; }
  .foot { margin-top: auto; padding: 8px 10px; border-top: 1px solid var(--rule); }
  .foot .row.sel { background: var(--card); color: var(--ink); box-shadow: var(--shadow-card); }
  .note {
    display: flex;
    gap: 8px;
    margin: 0;
    padding: 10px 16px;
    border-top: 1px solid var(--rule);
    font-size: 12.5px;
    color: var(--warn);
    background: var(--warn-wash);
  }
  .note :global(svg) { flex: none; margin-top: 1px; }
</style>
