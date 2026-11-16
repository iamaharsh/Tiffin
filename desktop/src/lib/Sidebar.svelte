<script lang="ts">
  import Icon from './Icon.svelte';
  import Logo from './Logo.svelte';
  import { labelVar, type Overview, type Section } from './api';

  let {
    ov, section, view, waiting = 0, onsection, onview, onpinned,
  }: {
    ov: Overview | null;
    section: Section;
    view: string;
    /** imported recipes waiting for review (badge on Browser) */
    waiting?: number;
    onsection: (s: Section) => void;
    onview: (v: string) => void;
    onpinned: (id: number) => void;
  } = $props();

  const sections: [Section, string, string][] = [
    ['recipes', 'Recipes', 'book'],
    ['groceries', 'Groceries', 'cart'],
    ['pantry', 'Pantry', 'jar'],
    ['foods', 'Foods', 'flame'],
    ['meals', 'Meals', 'calendar'],
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

  let open: Record<string, boolean> = $state({});
  const tops = $derived(ov?.categories.filter((c) => c.depth === 0) ?? []);
  const children = (path: string) => ov?.categories.filter((c) => c.depth === 1 && c.path.startsWith(path + '/')) ?? [];
</script>

<nav class="sidebar" aria-label="Main">
  <div class="brand"><Logo width={120} /></div>
  <ul class="sections">
    {#each sections as [id, name, icon]}
      <li>
        <button class="row" class:sel={section === id} aria-current={section === id ? 'page' : undefined} onclick={() => onsection(id)}>
          <span class="tile" style:background="var(--tile-{id})"><Icon name={icon} size={15} /></span>
          <span class="name">{name}</span>
          {#if id === 'browser' && waiting}<span class="badge num" aria-label="{waiting} waiting for review">{waiting}</span>{/if}
        </button>
      </li>
    {/each}
  </ul>

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
  .brand { padding: 12px; } /* clear space = the handle width */
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
  .sections .row { height: 36px; font-weight: 600; }
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
