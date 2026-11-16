<script lang="ts">
  import Photo from './Photo.svelte';
  import Stars from './Stars.svelte';
  import Icon from './Icon.svelte';
  import { labelVar, type Card } from './api';

  // `entering` is true for a moment after the shelf changes: only then do cards ripple in, so
  // typing in search never animates.
  let {
    cards, mode, entering, heroId, onopen,
    onmenu,
  }: { cards: Card[]; mode: 'grid' | 'list'; entering: boolean; heroId: number | null; onopen: (c: Card) => void; onmenu: (c: Card, x: number, y: number) => void } = $props();

  // Big libraries: draw the first cards, and more as the end of the shelf scrolls into view.
  // shortcut: cards already drawn stay in the page; switch to a real virtual list if libraries reach ~10k recipes.
  const PAGE = 120;
  let limit = $state(PAGE);
  $effect(() => { cards; limit = PAGE; });
  const visible = $derived(cards.length > limit ? cards.slice(0, limit) : cards);
  function more(node: HTMLElement) {
    const io = new IntersectionObserver((es) => { if (es.some((e) => e.isIntersecting)) limit += PAGE; }, { rootMargin: '1200px 0px' });
    io.observe(node);
    return { destroy: () => io.disconnect() };
  }

  // right-click, or the keyboard's menu key / Shift+F10 (those report the button's corner)
  function context(e: MouseEvent, c: Card) {
    e.preventDefault();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    onmenu(c, e.clientX || r.left + 24, e.clientY || r.top + 24);
  }
</script>

{#if mode === 'grid'}
  <ul class="grid" class:entering>
    {#each visible as c, i (c.id)}
      <li style:--i={Math.min(i, 14)}>
        <button class="card label-scope" style:--l={labelVar(c.label_index)} onclick={() => onopen(c)} oncontextmenu={(e) => context(e, c)}>
          <div class="photo" style:view-transition-name={heroId === c.id ? 'hero' : undefined}>
            <Photo photo={c.photo} seed={c.seed} />
            {#if c.favorite}<span class="badge fav" aria-label="Favorite"><Icon name="heart" size={13} /></span>{/if}
            {#if c.flags}<span class="badge warn" title="{c.flags} line{c.flags > 1 ? 's' : ''} to check"><Icon name="warn" size={13} /></span>{/if}
          </div>
          <span class="chip caps">{c.label}</span>
          <span class="title">{c.name}</span>
          <span class="meta">
            <Stars rating={c.rating} size={12} />
            {#if c.total_time}<span class="time num">{c.total_time}</span>{/if}
          </span>
        </button>
      </li>
    {/each}
    {#if visible.length < cards.length}{#key limit}<li class="more" use:more aria-hidden="true"></li>{/key}{/if}
  </ul>
{:else}
  <ul class="list" class:entering>
    {#each visible as c, i (c.id)}
      <li style:--i={Math.min(i, 14)}>
        <button class="line label-scope" style:--l={labelVar(c.label_index)} onclick={() => onopen(c)} oncontextmenu={(e) => context(e, c)}>
          <div class="thumb" style:view-transition-name={heroId === c.id ? 'hero' : undefined}><Photo photo={c.photo} seed={c.seed} /></div>
          <span class="title">{c.name}</span>
          <span class="flag">{#if c.flags}<Icon name="warn" size={14} />{/if}</span>
          <span class="heart">{#if c.favorite}<Icon name="heart" size={14} />{/if}</span>
          <span class="chip caps">{c.label}</span>
          <Stars rating={c.rating} size={12} />
          <span class="time num">{c.total_time}</span>
        </button>
      </li>
    {/each}
    {#if visible.length < cards.length}{#key limit}<li class="more" use:more aria-hidden="true"></li>{/key}{/if}
  </ul>
{/if}

<style>
  ul { list-style: none; margin: 0; padding: 0; }
  .more { height: 1px; }

  /* cards ripple in: first 15 staggered 22ms apart, the rest together */
  .entering > li { animation: rise 320ms var(--ease-out) calc(var(--i) * 22ms) both; }
  @keyframes rise { from { opacity: 0; transform: translateY(10px) scale(0.985); } }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(172px, 1fr));
    gap: 26px 20px;
  }
  .card {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    width: 100%;
    text-align: left;
  }
  .photo {
    position: relative;
    width: 100%;
    aspect-ratio: 1;
    border-radius: 14px;
    overflow: hidden;
    background: var(--well);
    box-shadow: var(--shadow-card);
    transition: transform 260ms var(--ease-out), box-shadow 260ms var(--ease-out);
  }
  .photo::after {
    content: '';
    position: absolute;
    inset: 0;
    border-radius: inherit;
    box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.06);
    pointer-events: none;
  }
  .card:hover .photo { transform: translateY(-3px); box-shadow: var(--shadow-lift); }
  .card:active .photo { transform: translateY(-1px) scale(0.985); transition-duration: 90ms; }
  .badge {
    position: absolute;
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.92);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.2);
  }
  .badge :global(svg) { fill: currentColor; stroke-width: 1.2; }
  .fav { right: 8px; bottom: 8px; color: var(--heart); }
  .warn { left: 8px; top: 8px; color: #b07400; }
  .warn :global(svg) { fill: none; stroke-width: 2; }
  .chip {
    margin-top: 11px;
    padding: 1px 7px;
    border-radius: 5px;
    font-size: 12.5px;
    color: var(--l-text);
    background: color-mix(in srgb, var(--l) 13%, transparent);
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .card .title {
    margin-top: 5px;
    font-family: var(--font-display);
    font-size: 15px;
    font-weight: 600;
    line-height: 1.25;
    letter-spacing: -0.005em;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    overflow-wrap: anywhere;
  }
  .meta { display: flex; align-items: center; gap: 8px; margin-top: 5px; }
  .time { font-size: 14px; color: var(--ink-2); }

  .list > li + li:not(.more) { border-top: 1px solid var(--rule); }
  .line {
    display: grid;
    grid-template-columns: 48px minmax(0, 1fr) 16px 16px minmax(0, 150px) 76px 64px;
    align-items: center;
    gap: 14px;
    width: 100%;
    padding: 8px 10px;
    border-radius: 10px;
    text-align: left;
    transition: background-color 120ms ease-out;
  }
  .line:hover { background: var(--well); }
  .line:active { background: var(--rule); }
  .thumb { width: 48px; height: 48px; border-radius: 9px; overflow: hidden; box-shadow: var(--shadow-card); }
  .line .title { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .line .chip { margin: 0; justify-self: start; }
  .heart { color: var(--heart); display: grid; }
  .heart :global(svg) { fill: currentColor; }
  .flag { color: var(--warn); display: grid; }
  .line .time { text-align: right; }
  .line > :global(.stars) { justify-self: end; }
  @media (max-width: 900px) {
    .line { grid-template-columns: 48px minmax(0, 1fr) 16px 16px 76px; }
    .line .chip, .line .time { display: none; }
  }
</style>
