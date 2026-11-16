<script lang="ts">
  import { fade } from 'svelte/transition';
  import Photo from './Photo.svelte';
  import Stars from './Stars.svelte';
  import Icon from './Icon.svelte';
  import Segmented from './Segmented.svelte';
  import * as api from './api';
  import { holdsUnsaved } from './updates.svelte';
  import type { Detail, Nutrients, Units } from './api';
  import { startTimer } from './timers.svelte';

  let {
    initial,
    onchanged,
    onerror,
    onedit,
    oncook,
    onmenu,
    onreview,
    onrestore,
    ondelete,
  }: {
    initial: Detail;
    onchanged: () => void;
    onerror: (m: string) => void;
    onedit: () => void;
    oncook: (d: Detail) => void;
    onmenu: (x: number, y: number) => void;
    onreview: () => void;
    onrestore: () => void;
    ondelete: () => void;
  } = $props();

  // Nutrition typed in by hand (per serving); null while not editing. An empty box is unknown, not 0.
  let nEdit: Nutrients | null = $state(null);
  holdsUnsaved(() => nEdit !== null);
  const nFields: [keyof Nutrients, string][] = [
    ['calories', 'Calories'], ['protein_g', 'Protein (g)'], ['carbs_g', 'Carbs (g)'], ['fat_g', 'Fat (g)'],
    ['fiber_g', 'Fiber (g)'], ['sugar_g', 'Sugar (g)'], ['sodium_mg', 'Sodium (mg)'],
  ];
  function startNutrition() {
    const r = (v: number) => Math.round(v * 10) / 10;
    nEdit = Object.fromEntries(nFields.map(([k]) => [k, d.per_serving[k] == null ? null : r(d.per_serving[k])])) as Nutrients;
  }
  async function saveNutrition(values: Nutrients | null) {
    nEdit = null;
    // only the boxes holding a number are sent, so a blank box is stored as unknown
    const sent = values && Object.fromEntries(Object.entries(values).filter(([, v]) => typeof v === 'number' && !Number.isNaN(v)));
    await write(api.setNutrition(c.id, sent));
  }
  function more(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    onmenu(r.left, r.bottom + 6);
  }

  // svelte-ignore state_referenced_locally -- the parent remounts this page per recipe
  let d = $state(initial);
  let scale = $state(1);
  let units: Units = $state('original');
  let heartPop = $state(false);

  const c = $derived(d.card);
  const scales = [0.5, 1, 2, 3];
  const scaleText: Record<number, string> = { 0.5: '½×', 1: '1×', 2: '2×', 3: '3×' };
  const unitText: Record<Units, string> = { original: 'As written', metric: 'Metric', us: 'US' };

  async function reload() {
    try {
      d = await api.recipe(c.id, scale, units);
    } catch (e) {
      onerror(String(e));
    }
  }

  async function write(job: Promise<void>) {
    try {
      await job;
      onchanged();
    } catch (e) {
      onerror(String(e));
    }
    await reload();
  }

  function toggleFavorite() {
    d.card.favorite = !c.favorite; // answer the click now; reload corrects it if saving failed
    heartPop = d.card.favorite;
    write(api.setFavorite(c.id, d.card.favorite));
  }
  function rate(n: number) {
    d.card.rating = n;
    write(api.setRating(c.id, n));
  }

  const facts = $derived(
    [
      ['Prep', d.prep_time],
      ['Cook', d.cook_time],
      ['Total', c.total_time],
      ['Serves', d.servings],
      ['Difficulty', d.difficulty],
    ].filter(([, v]) => v.trim()),
  );
  const host = $derived.by(() => {
    try {
      return d.source_url ? new URL(d.source_url).hostname.replace(/^www\./, '') : '';
    } catch {
      return '';
    }
  });

  // per serving; bar widths by the calories each macro brings
  const n = $derived(d.per_serving);
  const kcal = $derived((n.protein_g ?? 0) * 4 + (n.carbs_g ?? 0) * 4 + (n.fat_g ?? 0) * 9 || 1);
  const macros = $derived([
    { name: 'Protein', g: n.protein_g, color: 'var(--protein)', share: ((n.protein_g ?? 0) * 4) / kcal },
    { name: 'Carbs', g: n.carbs_g, color: 'var(--carbs)', share: ((n.carbs_g ?? 0) * 4) / kcal },
    { name: 'Fat', g: n.fat_g, color: 'var(--fat)', share: ((n.fat_g ?? 0) * 9) / kcal },
  ]);
  // a bar with a missing macro would show a false split
  const macrosKnown = $derived(macros.every((m) => m.g != null));
  const open = (url: string) => api.openLink(url).catch((e) => onerror(String(e)));
  const fmt = (v: number) => (v >= 10 ? Math.round(v).toString() : v.toFixed(1).replace(/\.0$/, ''));
</script>

<!-- an unknown value is a dash for the eye and "not reported" for a screen reader, never 0 -->
{#snippet val(x: number | null | undefined, show: (v: number) => string | number)}{#if x == null}<span aria-hidden="true">–</span><span class="sr">not reported</span>{:else}{show(x)}{/if}{/snippet}

<article class="detail label-scope" style:--l={api.labelVar(c.label_index)}>
  <header class="hero">
    <div class="photo" style:view-transition-name="hero"><Photo photo={c.photo} seed={c.seed} /></div>
    <div class="head">
      <span class="chip caps">{c.label}</span>
      <h1 class="selectable">{c.name}</h1>
      <div class="actions">
        <Stars rating={c.rating} size={19} onrate={rate} />
        <button
          class="heart"
          class:on={c.favorite}
          class:pop={heartPop}
          aria-pressed={c.favorite}
          onclick={toggleFavorite}
          onanimationend={() => (heartPop = false)}
        >
          <Icon name="heart" size={18} />
          <span>{c.favorite ? 'Favorite' : 'Add to Favorites'}</span>
        </button>
        <button class="pill" onclick={onedit}><Icon name="pencil" size={16} /> Edit</button>
        <button class="pill round" onclick={more} aria-label="More actions" aria-haspopup="menu"><Icon name="more" size={17} /></button>
      </div>
      {#if facts.length}
        <dl class="facts">
          {#each facts as [k, v]}
            <div><dt>{k}</dt><dd class="num">{v}</dd></div>
          {/each}
        </dl>
      {/if}
      {#if d.source || host || d.video}
        <p class="source">
          {#if d.source || host}
            <Icon name="link" size={14} />
            {#if host}
              <button class="src" onclick={() => open(d.source_url)} title={d.source_url}>{d.source || host}{#if d.source && !d.source.includes(host)} · {host}{/if} <Icon name="out" size={12} /></button>
            {:else}<span class="selectable">{d.source}</span>{/if}
          {/if}
          {#if d.video}<button class="src" onclick={() => open(d.video!)}><Icon name="play" size={13} /> Watch Video</button>{/if}
        </p>
      {/if}
      {#if d.steps.length}
        <button class="cook" onclick={() => oncook(d)}><Icon name="flame" size={18} /> Start Cooking</button>
      {/if}
    </div>
  </header>

  {#if c.in_trash}
    <div class="trashed" role="note">
      <Icon name="trash" size={17} />
      <span>This recipe is in the Trash.</span>
      <button class="tbtn" onclick={onrestore}>Put Back</button>
      <button class="tbtn danger" onclick={ondelete}>Delete Forever…</button>
    </div>
  {/if}

  {#if d.description.trim()}<p class="desc selectable">{d.description}</p>{/if}

  {#if d.flags.length}
    <div class="warn" role="note">
      <Icon name="warn" size={17} />
      <div>
        <strong>Check {d.flags.length === 1 ? 'this line' : `these ${d.flags.length} lines`} from the import</strong>
        <ul>{#each d.flags as f}<li>{f}</li>{/each}</ul>
        <button class="fix" onclick={onreview}>Review &amp; Fix</button>
      </div>
    </div>
  {/if}

  <div class="cols">
    <section class="ingredients">
      <div class="sec-head">
        <h2 class="caps">Ingredients</h2>
      </div>
      <div class="controls">
        <Segmented label="Scale" options={scales} value={scale} onchange={(v) => { scale = v; reload(); }}>
          {#snippet item(v)}<span class="num">{scaleText[v]}</span>{/snippet}
        </Segmented>
        <Segmented label="Units" options={['original', 'metric', 'us'] as Units[]} value={units} onchange={(v) => { units = v; reload(); }}>
          {#snippet item(v)}{unitText[v]}{/snippet}
        </Segmented>
      </div>
      <table class="selectable">
        <tbody>
          {#each d.rows as r, i (i)}
            {#if r.heading}
              <tr class="subhead"><td colspan="2">{r.text}</td></tr>
            {:else}
              <tr>
                <td class="amt num">{#key r.amount}<span in:fade={{ duration: 180 }}>{r.amount ?? ''}</span>{/key}</td>
                <td>{#key r.text}<span in:fade={{ duration: 180 }}>{r.text}</span>{/key}</td>
              </tr>
            {/if}
          {/each}
        </tbody>
      </table>
    </section>

    <section class="directions">
      <div class="sec-head"><h2 class="caps">Directions</h2></div>
      <ol class="selectable">
        {#each d.steps as s, i}
          <li>
            <span class="step num" aria-hidden="true">{i + 1}</span>
            <p>{#each s as p}{#if p.seconds}<button class="time" onclick={() => startTimer(`${p.text} · step ${i + 1}`, p.seconds!)} title="Start a {p.text} timer">{p.text}</button>{:else}{p.text}{/if}{/each}</p>
          </li>
        {/each}
      </ol>
    </section>
  </div>

  <section class="nutrition">
    <div class="sec-head">
      <h2 class="caps">Nutrition</h2><span class="per">per serving</span>
      {#if !nEdit}<button class="nlink" onclick={startNutrition}>Edit</button>{/if}
    </div>
    {#if nEdit}
      {@const ne = nEdit}
      <form class="nedit" onsubmit={(e) => { e.preventDefault(); saveNutrition(ne); }}>
        {#each nFields as [k, label]}
          <label><span>{label}</span><input type="number" min="0" step="0.1" inputmode="decimal" placeholder="–" bind:value={ne[k]} /></label>
        {/each}
        <p class="nnote wide">Leave a box empty if you don't know it. Empty means unknown, not zero.</p>
        <div class="nactions">
          {#if d.nutrition_edited}<button type="button" class="nbtn" onclick={() => saveNutrition(null)}>Use Calculated</button>{/if}
          <span class="grow"></span>
          <button type="button" class="nbtn" onclick={() => (nEdit = null)}>Cancel</button>
          <button class="nbtn primary">Save</button>
        </div>
      </form>
    {:else}
    <div class="ncard">
      <div class="cal"><span class="big num">{@render val(n.calories, Math.round)}</span><span>calories</span></div>
      <div class="macros">
        {#if macrosKnown}
          <div class="bar" role="img" aria-label="Calories from protein, carbs and fat">
            {#each macros as m}<span style:flex-grow={m.share} style:background={m.color}></span>{/each}
          </div>
        {/if}
        <div class="legend">
          {#each macros as m}
            <div><span class="dot" style:background={m.color}></span><span class="num">{@render val(m.g, fmt)}{#if m.g != null} g{/if}</span> {m.name}</div>
          {/each}
        </div>
      </div>
      <dl class="minor">
        <div><dt>Fiber</dt><dd class="num">{@render val(n.fiber_g, fmt)}{#if n.fiber_g != null} g{/if}</dd></div>
        <div><dt>Sugar</dt><dd class="num">{@render val(n.sugar_g, fmt)}{#if n.sugar_g != null} g{/if}</dd></div>
        <div><dt>Sodium</dt><dd class="num">{@render val(n.sodium_mg, Math.round)}{#if n.sodium_mg != null} mg{/if}</dd></div>
      </dl>
    </div>
    <p class="nnote">
      {#if d.nutrition_edited}Entered by hand.
      {:else}Estimated from {d.nutrition_counted} ingredient{d.nutrition_counted === 1 ? '' : 's'}{#if d.nutrition_skipped}; {d.nutrition_skipped} couldn't be matched{/if}.{/if}
    </p>
    {/if}
  </section>

  {#if d.notes.trim()}
    <section class="notes">
      <div class="sec-head"><h2 class="caps">Notes</h2></div>
      <p class="selectable">{d.notes}</p>
    </section>
  {/if}
</article>

<style>
  .detail { max-width: 1040px; margin: 0 auto; padding: 8px 0 64px; }
  .hero { display: flex; gap: 32px; align-items: flex-end; }
  .photo {
    width: 260px;
    aspect-ratio: 1;
    flex: none;
    border-radius: 20px;
    overflow: hidden;
    box-shadow: var(--shadow-lift);
  }
  .head { min-width: 0; padding-bottom: 4px; }
  .chip {
    display: inline-block;
    padding: 2px 9px;
    border-radius: 6px;
    font-size: 14px;
    color: #fff;
    background: var(--l);
  }
  h1 {
    margin: 10px 0 12px;
    font-family: var(--font-display);
    font-size: 36px;
    font-weight: 700;
    line-height: 1.08;
    letter-spacing: -0.022em;
    text-wrap: balance;
    overflow-wrap: anywhere;
  }
  .actions { display: flex; align-items: center; gap: 14px; margin-left: -3px; }
  .heart {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 32px;
    padding: 0 13px 0 10px;
    border-radius: 16px;
    color: var(--ink-2);
    background: var(--well);
    box-shadow: inset 0 0 0 1px var(--rule);
    font-size: 13.5px;
    transition: background-color 150ms ease-out, color 150ms ease-out, transform 100ms ease-out;
  }
  .heart:hover, .pill:hover { color: var(--ink); background: var(--rule); }
  .heart:active { transform: scale(0.95); }
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    padding: 0 13px 0 10px;
    border-radius: 16px;
    color: var(--ink-2);
    background: var(--well);
    box-shadow: inset 0 0 0 1px var(--rule);
    font-size: 13.5px;
    transition: background-color 150ms ease-out, color 150ms ease-out, transform 100ms ease-out;
  }
  .pill:active { transform: scale(0.95); }
  .pill.round { width: 32px; padding: 0; justify-content: center; }
  .pill.round :global(svg) { stroke-width: 2.8; }
  .trashed {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 24px;
    padding: 10px 12px 10px 14px;
    border-radius: 10px;
    background: var(--well);
    color: var(--ink-2);
  }
  .trashed span { flex: 1; color: var(--ink); }
  .tbtn { height: 30px; padding: 0 12px; border-radius: 8px; font-size: 13.5px; font-weight: 600; color: var(--accent-text); }
  .tbtn:hover { background: var(--accent-wash); }
  .tbtn.danger { color: var(--heart); }
  .tbtn.danger:hover { background: color-mix(in srgb, var(--heart) 12%, transparent); }
  .fix {
    margin-top: 10px;
    height: 30px;
    padding: 0 14px;
    border-radius: 8px;
    font-size: 13.5px;
    font-weight: 600;
    color: #fff;
    background: var(--warn);
  }
  .fix:hover { filter: brightness(1.08); }
  .nlink { margin-left: auto; font-size: 13.5px; color: var(--accent-text); border-radius: 4px; }
  .nlink:hover { text-decoration: underline; }
  .nedit {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(130px, 1fr));
    gap: 12px;
    margin-top: 14px;
    padding: 18px 22px;
    border-radius: 14px;
    background: var(--card);
    box-shadow: var(--shadow-card), inset 0 0 0 1px var(--rule);
  }
  .nedit .wide { grid-column: 1 / -1; margin: 0; }
  .nedit label { display: flex; flex-direction: column; gap: 4px; font-size: 12.5px; color: var(--ink-2); }
  .nedit input {
    height: 34px;
    padding: 0 10px;
    border: 0;
    border-radius: 9px;
    background: var(--well);
    box-shadow: inset 0 0 0 1px var(--rule);
    color: var(--ink);
    font: 600 17px var(--font-label);
    font-variant-numeric: tabular-nums;
    outline: none;
    -webkit-user-select: text;
    user-select: text;
  }
  .nedit input:focus { background: var(--card); box-shadow: 0 0 0 3px var(--focus), inset 0 0 0 1px var(--accent); }
  .nactions { grid-column: 1 / -1; display: flex; gap: 8px; }
  .grow { flex: 1; }
  .nbtn { height: 32px; padding: 0 14px; border-radius: 9px; font-size: 13.5px; font-weight: 500; background: var(--well); box-shadow: inset 0 0 0 1px var(--rule); }
  .nbtn:hover { background: var(--rule); }
  .nbtn.primary { color: #fff; background: var(--accent); box-shadow: none; }
  .nbtn.primary:hover { background: var(--accent-press); }
  .heart.on { color: var(--heart); background: color-mix(in srgb, var(--heart) 12%, transparent); box-shadow: none; }
  .heart.on :global(svg) { fill: currentColor; }
  .heart.pop :global(svg) { animation: heart 420ms var(--ease-spring); }
  @keyframes heart { 0% { transform: scale(0.6); } 45% { transform: scale(1.3); } }
  .facts { display: flex; flex-wrap: wrap; gap: 8px; margin: 18px 0 0; }
  .facts div {
    display: flex;
    align-items: baseline;
    gap: 6px;
    padding: 4px 11px;
    border-radius: 8px;
    background: var(--well);
  }
  dt { font-size: 12.5px; color: var(--ink-2); }
  dd { margin: 0; font-size: 16.5px; }
  .source { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; margin: 14px 0 0; font-size: 13px; color: var(--ink-2); }
  .src { display: inline-flex; align-items: center; gap: 4px; color: var(--accent-text); border-radius: 4px; }
  .src:hover { text-decoration: underline; }
  .src + .src { margin-left: 10px; }
  .cook {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 42px;
    margin-top: 20px;
    padding: 0 20px 0 16px;
    border-radius: 12px;
    font-size: 15px;
    font-weight: 600;
    color: #fff;
    background: var(--accent);
    box-shadow: var(--shadow-card), inset 0 -2px 0 rgba(0, 0, 0, 0.15);
    transition: background-color 140ms ease-out, transform 100ms ease-out, box-shadow 200ms ease-out;
  }
  .cook:hover { background: var(--accent-press); box-shadow: var(--shadow-lift), inset 0 -2px 0 rgba(0, 0, 0, 0.15); }
  .cook:active { transform: scale(0.97); }
  .time {
    padding: 0 5px;
    margin: 0 -1px;
    border-radius: 5px;
    font: inherit;
    color: var(--accent-text);
    background: var(--accent-wash);
    transition: background-color 120ms ease-out;
  }
  .time:hover { background: color-mix(in srgb, var(--accent) 22%, var(--page)); }
  .time:active { transform: scale(0.97); }

  .desc { max-width: 70ch; margin: 28px 0 0; font-size: 15.5px; color: var(--ink-2); }
  .warn {
    display: flex;
    gap: 10px;
    margin-top: 24px;
    padding: 12px 14px;
    border-radius: 10px;
    background: var(--warn-wash);
    color: var(--warn);
  }
  .warn :global(svg) { flex: none; margin-top: 2px; }
  .warn strong { color: var(--ink); font-weight: 600; }
  .warn ul { margin: 4px 0 0; padding-left: 18px; color: var(--ink); }

  .cols { display: grid; grid-template-columns: minmax(0, 5fr) minmax(0, 6fr); gap: 48px; margin-top: 36px; }
  .sec-head { display: flex; align-items: baseline; gap: 10px; border-bottom: 2px solid var(--l); padding-bottom: 6px; }
  h2 { margin: 0; font-size: 19px; color: var(--l-text); }
  .controls { display: flex; flex-wrap: wrap; gap: 8px; margin: 14px 0 10px; }
  table { border-collapse: collapse; width: 100%; font-size: 15px; }
  td { padding: 6px 0; vertical-align: baseline; border-bottom: 1px solid var(--rule); }
  .amt { width: 1%; white-space: nowrap; padding-right: 14px; text-align: right; font-size: 17px; color: var(--l-text); }
  .subhead td { padding-top: 16px; font-weight: 600; border-bottom: 0; }
  ol { list-style: none; margin: 14px 0 0; padding: 0; }
  ol li { display: flex; gap: 16px; padding: 4px 0 14px; }
  .step {
    flex: none;
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 50%;
    font-size: 17px;
    color: var(--l-text);
    background: color-mix(in srgb, var(--l) 13%, transparent);
  }
  ol p { margin: 3px 0 0; font-size: 15.5px; line-height: 1.55; max-width: 62ch; }

  .nutrition { margin-top: 40px; }
  .per { font-size: 13px; color: var(--ink-2); }
  .ncard {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    gap: 32px;
    align-items: center;
    margin-top: 14px;
    padding: 18px 22px;
    border-radius: 14px;
    background: var(--card);
    box-shadow: var(--shadow-card), inset 0 0 0 1px var(--rule);
  }
  .cal { display: flex; flex-direction: column; color: var(--ink-2); font-size: 13px; }
  .big { font-size: 44px; line-height: 1; color: var(--ink); }
  .bar { display: flex; gap: 3px; height: 10px; }
  .bar span { flex-basis: 0; border-radius: 5px; min-width: 4px; transition: flex-grow 400ms var(--ease-out); }
  .legend { display: flex; flex-wrap: wrap; gap: 6px 18px; margin-top: 10px; font-size: 13.5px; color: var(--ink-2); }
  .legend .num { font-size: 16px; color: var(--ink); }
  .dot { display: inline-block; width: 8px; height: 8px; border-radius: 50%; margin-right: 6px; }
  .minor { display: flex; gap: 20px; margin: 0; }
  .minor div { display: flex; flex-direction: column; }
  .nnote { margin: 8px 2px 0; font-size: 12.5px; color: var(--ink-3); }
  .notes { margin-top: 36px; }
  .notes p { white-space: pre-wrap; max-width: 70ch; }

  @media (max-width: 900px) {
    .hero { flex-direction: column; align-items: flex-start; }
    .photo { width: 200px; }
    .cols { grid-template-columns: 1fr; }
    .ncard { grid-template-columns: 1fr; gap: 16px; }
  }
</style>
