<script lang="ts">
  import Photo from './Photo.svelte';
  import Stars from './Stars.svelte';
  import Icon from './Icon.svelte';
  import * as api from './api';
  import type { Category, Draft } from './api';

  let {
    id,
    initial,
    categories,
    onsaved,
    oncancel,
    onerror,
    onnote,
  }: {
    id: number;
    initial: Draft;
    categories: Category[];
    onsaved: (id: number) => void;
    oncancel: () => void;
    onerror: (m: string) => void;
    onnote: (m: string) => void;
  } = $props();

  // svelte-ignore state_referenced_locally -- the parent remounts this page per recipe
  let d: Draft = $state($state.snapshot(initial));
  // svelte-ignore state_referenced_locally
  const start = JSON.stringify(initial);
  const dirty = $derived(JSON.stringify(d) !== start);
  let saving = $state(false);
  let addingPhoto = $state(false);
  let confirmDiscard = $state(false);
  let discardTimer = 0;
  let fileInput: HTMLInputElement;
  let nameBox: HTMLInputElement;

  async function save() {
    if (saving) return;
    saving = true;
    try {
      onsaved(await api.saveRecipe(id, d));
    } catch (e) {
      onerror(String(e));
    }
    saving = false;
  }

  // Leaving with changes takes a second press, so a stray Escape can't throw an edit away.
  function cancel() {
    if (!dirty || confirmDiscard) return oncancel();
    confirmDiscard = true;
    clearTimeout(discardTimer);
    discardTimer = window.setTimeout(() => (confirmDiscard = false), 4000);
  }

  async function tidy() {
    try {
      const [t, changes] = await api.tidy(d);
      d = t;
      onnote(changes.length ? changes.join('; ') : 'Already tidy: nothing to change.');
    } catch (e) {
      onerror(String(e));
    }
  }

  async function pickPhoto(e: Event) {
    const file = (e.currentTarget as HTMLInputElement).files?.[0];
    (e.currentTarget as HTMLInputElement).value = '';
    if (file) await usePhoto(file);
  }

  // A picture dragged anywhere onto the page becomes the photo.
  let dropping = $state(false);
  const carriesFile = (e: DragEvent) => !!e.dataTransfer?.types.includes('Files');
  function dragOver(e: DragEvent) {
    if (!carriesFile(e)) return;
    e.preventDefault();
    e.dataTransfer!.dropEffect = 'copy';
    dropping = true;
  }
  function drop(e: DragEvent) {
    dropping = false;
    if (!carriesFile(e)) return;
    e.preventDefault();
    const file = [...(e.dataTransfer?.files ?? [])].find((f) => f.type.startsWith('image/'));
    if (file) usePhoto(file);
    else onerror('That isn’t a picture. Drop a JPEG, PNG, WebP, GIF or BMP.');
  }

  async function usePhoto(file: File) {
    addingPhoto = true;
    try {
      d.photo = await api.addPhoto(file, d.name || 'photo');
    } catch (err) {
      onerror(String(err));
    }
    addingPhoto = false;
  }

  function toggleCategory(path: string) {
    d.categories = d.categories.includes(path) ? d.categories.filter((c) => c !== path) : [...d.categories, path];
  }

  function keys(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') {
      e.preventDefault();
      save();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      cancel();
    }
  }

  const facts: [keyof Draft, string, string][] = [
    ['servings', 'Servings', '4'],
    ['prep_time', 'Prep time', '15 min'],
    ['cook_time', 'Cook time', '30 min'],
    ['total_time', 'Total time', 'Worked out from prep + cook'],
    ['difficulty', 'Difficulty', 'Easy'],
  ];
  // one row per top-level category, its subcategories beside it
  const groups = $derived(
    categories.filter((c) => c.depth === 0).map((top) => ({ top, subs: categories.filter((c) => c.path.startsWith(top.path + '/')) })),
  );
  const lineCount = (s: string) => s.split('\n').filter((l) => l.trim()).length;

  $effect(() => { if (id === 0) nameBox.focus(); });
</script>

<svelte:window
  onkeydown={keys}
  ondragover={dragOver}
  ondragleave={(e) => { if (!e.relatedTarget) dropping = false; }}
  ondrop={drop}
/>

{#if dropping}
  <div class="drop" aria-hidden="true"><span><Icon name="photo" size={30} />Drop to use as the photo</span></div>
{/if}

<header class="bar">
  <h1>{id === 0 ? 'New Recipe' : 'Edit Recipe'}</h1>
  <button class="btn ghost" onclick={tidy} title="Fix amounts, units, step numbering and capitals in one pass">
    <Icon name="wand" size={16} /> Tidy Formatting
  </button>
  <button class="btn" class:danger={confirmDiscard} onclick={cancel}>{confirmDiscard ? 'Discard Changes?' : 'Cancel'}</button>
  <button class="btn primary" onclick={save} disabled={saving}>{saving ? 'Saving…' : 'Save'}</button>
</header>

<form class="edit" onsubmit={(e) => { e.preventDefault(); save(); }}>
  <div class="top">
    <div class="pic">
      <button type="button" class="photo" onclick={() => fileInput.click()} aria-label={d.photo ? 'Change photo' : 'Add a photo'}>
        <Photo photo={d.photo} seed={d.photo_seed || 1} />
        <span class="over"><Icon name="photo" size={22} />{addingPhoto ? 'Adding…' : d.photo ? 'Change' : 'Add Photo'}</span>
      </button>
      {#if d.photo}<button type="button" class="link" onclick={() => (d.photo = '')}>Remove Photo</button>{/if}
      <input bind:this={fileInput} type="file" accept="image/jpeg,image/png,image/webp,image/gif,image/bmp" hidden onchange={pickPhoto} />
    </div>
    <div class="title">
      <label class="sr" for="name">Name</label>
      <input id="name" bind:this={nameBox} class="name" bind:value={d.name} placeholder="Recipe name" autocomplete="off" />
      <div class="rating"><span>Rating</span><Stars rating={d.rating} size={20} onrate={(n) => (d.rating = n === d.rating ? 0 : n)} /></div>
      <div class="facts">
        {#each facts as [key, label, hint]}
          <label class="field" class:wide={key === 'total_time'}>
            <span>{label}</span>
            <input bind:value={d[key]} placeholder={hint} autocomplete="off" />
          </label>
        {/each}
      </div>
    </div>
  </div>

  <fieldset class="group">
    <legend class="caps">Categories</legend>
    {#if categories.length}
      <div class="groups">
        {#each groups as g (g.top.path)}
          <div class="chips">
            {#each [g.top, ...g.subs] as c (c.path)}
              {@const on = d.categories.includes(c.path)}
              <button type="button" class="chip" class:sub={c.depth > 0} class:on style:--l={api.labelVar(g.top.label)} aria-pressed={on} onclick={() => toggleCategory(c.path)}>
                <span class="tick"><Icon name="check" size={12} /></span>
                {c.path.slice(g.top.path.length + 1).split('/').join(' › ') || c.name}
              </button>
            {/each}
          </div>
        {/each}
      </div>
    {:else}
      <p class="hint">No categories yet.</p>
    {/if}
  </fieldset>

  <label class="block">
    <span class="caps">Description</span>
    <textarea bind:value={d.description} rows="2" placeholder="A line or two about this dish"></textarea>
  </label>

  <div class="two">
    <label class="block">
      <span class="caps">Ingredients <em class="num">{lineCount(d.ingredients)}</em></span>
      <textarea class="lines" bind:value={d.ingredients} rows="10" placeholder={'One per line, e.g.\n2 cups flour\n1 tsp salt\n\nA line ending in a colon is a heading:'} spellcheck="false"></textarea>
    </label>
    <label class="block">
      <span class="caps">Directions <em class="num">{lineCount(d.directions)}</em></span>
      <textarea class="lines" bind:value={d.directions} rows="10" placeholder="One step per line"></textarea>
    </label>
  </div>

  <label class="block">
    <span class="caps">Notes</span>
    <textarea bind:value={d.notes} rows="3" placeholder="Swaps, what worked, what to try next time"></textarea>
  </label>

  <fieldset class="group">
    <legend class="caps">Source</legend>
    <div class="source">
      <label class="field"><span>Name</span><input bind:value={d.source} placeholder="Grandma's notebook" autocomplete="off" /></label>
      <label class="field"><span>Web page</span><input type="url" bind:value={d.source_url} placeholder="https://" autocomplete="off" spellcheck="false" /></label>
      <label class="field"><span>Video</span><input type="url" bind:value={d.video_url} placeholder="YouTube, Vimeo or video file link" autocomplete="off" spellcheck="false" /></label>
    </div>
  </fieldset>
</form>

<style>
  .bar {
    position: sticky;
    top: 0;
    z-index: 5;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 64px;
    margin: 0 -32px;
    padding: 0 32px;
    background: var(--toolbar);
    backdrop-filter: blur(20px) saturate(170%);
    border-bottom: 1px solid var(--rule);
  }
  h1 { flex: 1; margin: 0; font-family: var(--font-display); font-size: 22px; font-weight: 700; letter-spacing: -0.015em; }
  .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    padding: 0 14px;
    border-radius: 9px;
    font-size: 13.5px;
    font-weight: 500;
    background: var(--well);
    box-shadow: inset 0 0 0 1px var(--rule);
    transition: background-color 120ms ease-out, color 120ms ease-out, transform 100ms ease-out;
  }
  .btn:hover { background: var(--rule); }
  .btn:active { transform: scale(0.97); }
  .btn.ghost { background: none; box-shadow: none; color: var(--accent-text); }
  .btn.ghost:hover { background: var(--accent-wash); }
  .btn.primary { min-width: 76px; justify-content: center; background: var(--accent); color: #fff; box-shadow: inset 0 -1px 0 rgba(0, 0, 0, 0.15); }
  .btn.primary:hover { background: var(--accent-press); }
  .btn.primary:disabled { opacity: 0.6; }
  .btn.danger { background: var(--heart); color: var(--on-heart); box-shadow: none; }

  .edit { max-width: 980px; margin: 0 auto; padding: 28px 0 80px; display: flex; flex-direction: column; gap: 28px; }
  .top { display: flex; gap: 28px; }
  .pic { display: flex; flex-direction: column; align-items: center; gap: 8px; flex: none; }
  .photo { position: relative; width: 176px; aspect-ratio: 1; border-radius: 18px; overflow: hidden; box-shadow: var(--shadow-lift); }
  .over {
    position: absolute;
    inset: auto 0 0 0;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 10px 0 9px;
    font-size: 13px;
    font-weight: 600;
    color: #fff;
    background: linear-gradient(transparent, rgba(0, 0, 0, 0.55));
    opacity: 0;
    transform: translateY(6px);
    transition: opacity 160ms ease-out, transform 200ms var(--ease-out);
  }
  .photo:hover .over, .photo:focus-visible .over { opacity: 1; transform: none; }
  .link { font-size: 13px; color: var(--accent-text); }
  .link:hover { text-decoration: underline; }

  .title { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 12px; }
  .name {
    width: 100%;
    padding: 4px 8px;
    margin-left: -8px;
    border: 0;
    border-radius: 8px;
    background: none;
    color: var(--ink);
    font: 700 32px/1.15 var(--font-display);
    letter-spacing: -0.022em;
    outline: none;
    transition: background-color 150ms ease-out, box-shadow 150ms ease-out;
  }
  .name:hover { background: var(--well); }
  .name:focus { background: var(--card); box-shadow: 0 0 0 3px var(--focus), inset 0 0 0 1px var(--accent); }
  .name::placeholder { color: var(--ink-3); }
  .rating { display: flex; align-items: center; gap: 10px; font-size: 13px; color: var(--ink-2); }
  .facts { display: grid; grid-template-columns: repeat(auto-fill, minmax(140px, 1fr)); gap: 10px; }
  .facts .wide { grid-column: span 2; }

  .field { display: flex; flex-direction: column; gap: 4px; font-size: 12.5px; color: var(--ink-2); }
  input:not(.name), textarea {
    width: 100%;
    padding: 7px 10px;
    border: 0;
    border-radius: 9px;
    background: var(--well);
    box-shadow: inset 0 0 0 1px var(--rule);
    color: var(--ink);
    font: inherit;
    font-size: 14.5px;
    outline: none;
    caret-color: var(--accent);
    -webkit-user-select: text;
    user-select: text;
    transition: background-color 150ms ease-out, box-shadow 150ms ease-out;
  }
  input:not(.name):focus, textarea:focus { background: var(--card); box-shadow: 0 0 0 3px var(--focus), inset 0 0 0 1px var(--accent); }
  input::placeholder, textarea::placeholder { color: var(--ink-3); }
  textarea { resize: none; field-sizing: content; min-height: 3lh; line-height: 1.55; }
  textarea.lines { min-height: 12lh; }

  .block { display: flex; flex-direction: column; gap: 8px; }
  .block > span, legend { font-size: 15px; color: var(--ink-2); }
  em { font-style: normal; margin-left: 4px; color: var(--ink-3); }
  .two { display: grid; grid-template-columns: minmax(0, 5fr) minmax(0, 6fr); gap: 24px; }

  .group { margin: 0; padding: 0; border: 0; min-width: 0; }
  legend { padding: 0; margin-bottom: 10px; }
  .groups { display: flex; flex-direction: column; gap: 10px; }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .chip:not(.sub) { font-weight: 600; margin-right: 6px; }
  .chip.sub { height: 28px; font-size: 13px; }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 12px 0 8px;
    border-radius: 15px;
    font-size: 13.5px;
    background: var(--well);
    box-shadow: inset 0 0 0 1px var(--rule);
    transition: background-color 160ms ease-out, color 160ms ease-out, box-shadow 160ms ease-out, transform 100ms ease-out;
  }
  .chip:hover { box-shadow: inset 0 0 0 1px var(--rule-strong); }
  .chip:active { transform: scale(0.96); }
  .tick {
    display: grid;
    place-items: center;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    color: transparent;
    box-shadow: inset 0 0 0 1.5px var(--l);
    transition: background-color 160ms ease-out, color 160ms ease-out;
  }
  .tick :global(svg) { stroke-width: 2.6; transform: scale(0.4); transition: transform 260ms var(--ease-spring); }
  .chip.on { background: color-mix(in srgb, var(--l) 14%, var(--card)); box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--l) 45%, transparent); }
  .chip.on .tick { background: var(--l); color: #fff; }
  .chip.on .tick :global(svg) { transform: none; }
  .hint { margin: 0; color: var(--ink-3); }
  .source { display: grid; grid-template-columns: 1fr 1.4fr 1.4fr; gap: 10px; }
  .drop {
    position: fixed;
    inset: 12px;
    z-index: 30;
    display: grid;
    place-items: center;
    border-radius: 20px;
    border: 2.5px dashed var(--accent);
    background: color-mix(in srgb, var(--accent-wash) 80%, transparent);
    backdrop-filter: blur(3px);
    pointer-events: none;
    animation: drop-in 160ms var(--ease-out);
  }
  .drop span { display: flex; flex-direction: column; align-items: center; gap: 10px; font: 600 18px var(--font-display); color: var(--accent-text); }
  @keyframes drop-in { from { opacity: 0; transform: scale(0.99); } }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }

  @media (max-width: 900px) {
    .top { flex-direction: column; }
    .pic { align-items: flex-start; }
    .two, .source { grid-template-columns: 1fr; }
  }
</style>
