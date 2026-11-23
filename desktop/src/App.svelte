<script lang="ts">
  import { tick } from 'svelte';
  import { fly } from 'svelte/transition';
  import { prefersReducedMotion } from 'svelte/motion';
  import Sidebar from './lib/Sidebar.svelte';
  import RecipeList from './lib/RecipeList.svelte';
  import RecipeDetail from './lib/RecipeDetail.svelte';
  import RecipeEditor from './lib/RecipeEditor.svelte';
  import CookMode from './lib/CookMode.svelte';
  import TimerTray from './lib/TimerTray.svelte';
  import Segmented from './lib/Segmented.svelte';
  import Icon from './lib/Icon.svelte';
  import Menu, { type MenuItem } from './lib/Menu.svelte';
  import Dialog from './lib/Dialog.svelte';
  import MealDialog from './lib/MealDialog.svelte';
  import GroceryDialog from './lib/GroceryDialog.svelte';
  import ImportPage from './lib/ImportPage.svelte';
  import ReviewPage from './lib/ReviewPage.svelte';
  import GroceriesPage from './lib/GroceriesPage.svelte';
  import PantryPage from './lib/PantryPage.svelte';
  import FoodsPage from './lib/FoodsPage.svelte';
  import DiaryPage from './lib/DiaryPage.svelte';
  import HomePage from './lib/HomePage.svelte';
  import ReportsPage from './lib/ReportsPage.svelte';
  import TargetsPage from './lib/TargetsPage.svelte';
  import BodyPage from './lib/BodyPage.svelte';
  import { targetsDirty } from './lib/targets.svelte';
  import MealsPage from './lib/MealsPage.svelte';
  import MenusPage from './lib/MenusPage.svelte';
  import SettingsPage from './lib/SettingsPage.svelte';
  import UpdateBanner from './lib/UpdateBanner.svelte';
  import { update, checkForUpdate, holdsUnsaved } from './lib/updates.svelte';
  import * as api from './lib/api';
  import type { Card, Detail, Draft, Overview, Review, Section } from './lib/api';

  // Per-viewer conveniences only; the library itself lives in Rust.
  const remembered = (k: string) => { try { return localStorage.getItem(k); } catch { return null; } };
  const remember = (k: string, v: string) => { try { localStorage.setItem(k, v); } catch { /* private mode */ } };

  let ov: Overview | null = $state(null);
  let section: Section = $state('home');
  /** which Settings tab opens (another page can ask for one) */
  let settingsTab: 'sync' | 'backup' | 'history' | 'trackers' | 'claude' | 'about' = $state('sync');
  let view = $state('all');
  let search = $state('');
  let sort = $state(Number(remembered('sort') ?? -1));
  let mode: 'grid' | 'list' = $state(remembered('mode') === 'list' ? 'list' : 'grid');
  let foodUnits: 'metric' | 'us' = $state(remembered('foodUnits') === 'us' ? 'us' : 'metric');
  let cards: Card[] = $state([]);
  let entering = $state(true);
  let open: Detail | null = $state(null);
  let heroId: number | null = $state(null);
  let editing: { id: number; draft: Draft } | null = $state(null);
  let cooking: Detail | null = $state(null);
  let toast = $state('');
  let toastOk = $state(false);
  let toastAction: { label: string; run: () => void } | null = $state(null);
  let menu: { x: number; y: number; items: MenuItem[] } | null = $state(null);
  let mealFor: { id: number; name: string } | null = $state(null);
  let groceryFor: { id: number; name: string } | null = $state(null);
  let confirmDelete: { ids: number[]; what: string } | null = $state(null);
  // something to say about the library file: recovered, replaced behind our back, or damaged (blocks until a choice)
  let notice: api.Notice | null = $state(null);
  let savedVersions: api.SavedVersion[] = $state([]);
  let savedBackups: api.SavedVersion[] = $state([]);
  let recovering = $state(false);
  let backupPicker: HTMLInputElement | undefined = $state();
  // imported recipes waiting for review; `review` is the one on screen
  let review: Review | null = $state(null);
  let queue: Review[] = $state([]);
  // days the Meals page asked Groceries to make a list for
  let planFor: [number, number] | null = $state(null);
  let scrolled = $state(false);
  let scroller: HTMLElement;
  let searchBox: HTMLInputElement | undefined = $state();
  let savedScroll = 0;
  let seq = 0;
  let toastTimer = 0;

  const viewNames: Record<string, string> = {
    all: 'All Recipes', recent: 'Most Recent', top: 'Top Rated',
    favorites: 'Favorites', uncategorized: 'Uncategorized', trash: 'Trash',
  };
  const title = $derived(view.startsWith('cat:') ? view.slice(4).split('/').join(' › ') : viewNames[view]);
  const sortNames = ['Name', 'Rating', 'Newest'];

  /** Show a note at the bottom: an error, or (ok) a confirmation, optionally with an Undo-style action. `stay` keeps it until dismissed. */
  function fail(msg: string, ok = false, action: { label: string; run: () => void } | null = null, stay = false) {
    toast = msg;
    toastOk = ok;
    toastAction = action;
    clearTimeout(toastTimer);
    if (!stay) toastTimer = window.setTimeout(() => (toast = ''), action ? 10000 : 6000); // stay: until Dismiss
  }
  const note = (msg: string, action: { label: string; run: () => void } | null = null) => fail(msg, true, action);

  /** the tab Tiffin opens on is applied once, at start, and only while nothing has moved the window off the first page yet */
  let launched = false;
  async function refresh() {
    try {
      ov = await api.overview();
      if (!launched) {
        launched = true;
        if (section === 'home' && api.tabNames[ov.launch]) section = ov.launch;
      }
      if (sort < 0) sort = ov.sort;
      await load();
      // assign only: reading `notice` here would make the effect that calls refresh() depend on it
      const n = await api.pendingNotice();
      if (n) {
        if (n.kind === 'damaged') [savedVersions, savedBackups] = await Promise.all([api.snapshots(), api.backups()]);
        notice = n;
      }
    } catch (e) {
      fail(String(e));
    }
  }

  async function load() {
    const mine = ++seq; // a slower older answer never replaces a newer one
    const got = await api.list(view, search, Math.max(sort, 0));
    if (mine === seq) cards = got;
  }

  const nameOf = (p: string) => p.split(/[\\/]/).pop() ?? p;
  const folderOf = (p: string) => p.slice(0, Math.max(p.lastIndexOf('\\'), p.lastIndexOf('/')));

  /** One of the ways out of a damaged library. On success the lock is gone; on failure the dialog stays. */
  async function recover(job: () => Promise<string>) {
    if (recovering) return; // the buttons use aria-disabled, not disabled, so keyboard focus stays on them; this guard does the blocking
    recovering = true;
    try {
      const msg = await job();
      notice = null;
      note(msg);
      await refresh();
    } catch (e) {
      fail(String(e));
    }
    recovering = false;
  }
  async function chooseBackupFile(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const f = input.files?.[0];
    input.value = '';
    if (f) {
      try {
        const text = await f.text();
        recover(() => api.restoreBackup(null, text));
      } catch (e) {
        fail(String(e));
      }
    }
  }

  /** Run a page change as a view transition (photo grows from its card), or plainly. */
  async function transition(update: () => Promise<void> | void) {
    if (!document.startViewTransition || prefersReducedMotion.current) {
      await update();
      return;
    }
    await document.startViewTransition(async () => {
      await update();
      await tick();
    }).finished;
  }

  /** Run a change, then show the library (and the open recipe) as they are now. */
  async function act(job: Promise<unknown>, msg = '', action: { label: string; run: () => void } | null = null) {
    try {
      await job;
      if (msg) note(msg, action);
    } catch (e) {
      fail(String(e));
    }
    await refresh();
    if (open) {
      const id = open.card.id;
      open = await api.recipe(id, 1, 'original').catch(() => null);
    }
  }

  function trash(ids: number[], name: string) {
    if (open && ids.includes(open.card.id)) back(false);
    act(api.setTrashed(ids, true), `Moved “${name}” to Trash`, { label: 'Undo', run: () => act(api.setTrashed(ids, false), 'Put back') });
  }

  async function deleteForever(ids: number[]) {
    confirmDelete = null;
    if (open && ids.includes(open.card.id)) await back(false);
    try {
      const n = await api.deleteForever(ids);
      note(`Deleted ${n} recipe${n === 1 ? '' : 's'} for good`);
    } catch (e) {
      fail(String(e));
    }
    await refresh();
  }

  /** The right-click menu for a recipe (the classic app's, in its order). */
  function recipeMenu(c: Card, x: number, y: number) {
    const items: MenuItem[] = c.in_trash
      ? [
          { label: 'Put Back', icon: 'back', run: () => act(api.setTrashed([c.id], false), `Put “${c.name}” back`) },
          'sep',
          { label: 'Delete Forever…', icon: 'trash', danger: true, run: () => (confirmDelete = { ids: [c.id], what: `“${c.name}”` }) },
        ]
      : [
          ...(open?.card.id === c.id ? [] : [{ label: 'Open', run: () => openById(c.id) }]),
          { label: 'Edit', icon: 'pencil', run: () => startEdit(c.id) },
          'sep',
          { label: 'Duplicate', icon: 'copy', run: () => act(api.duplicate(c.id), `Made a copy of “${c.name}”`) },
          'sep',
          { label: 'Add to Grocery List…', icon: 'cart', run: () => (groceryFor = { id: c.id, name: c.name }) },
          { label: 'Add to Meal Plan…', icon: 'calendar', run: () => (mealFor = { id: c.id, name: c.name }) },
          { label: c.favorite ? 'Remove from Favorites' : 'Add to Favorites', icon: 'heart', run: () => act(api.setFavorite(c.id, !c.favorite)) },
          { label: c.pinned ? 'Unpin' : 'Pin to Sidebar', icon: 'pin', run: () => act(api.setPinned(c.id, !c.pinned)) },
          'sep',
          { label: 'Move to Trash', icon: 'trash', danger: true, run: () => trash([c.id], c.name) },
        ];
    menu = { x, y, items };
  }

  async function openById(id: number) {
    if (busyEditing()) return;
    parkReview();
    section = 'recipes';
    try {
      const d = await api.recipe(id, 1, 'original');
      if (!open) savedScroll = scroller.scrollTop;
      open = d;
      scroller.scrollTop = 0;
    } catch (e) {
      fail(String(e));
    }
  }

  // ---- importing ----
  /** A recipe read from the web, pasted text or the inbox, waiting to be checked and saved. */
  function queueReview(rv: Review, show: boolean) {
    if (show && !review) {
      review = rv;
      scroller.scrollTop = 0;
    } else queue = [...queue, rv];
  }
  /** Leaving the review page keeps the recipe waiting on the Browser page instead of losing it. */
  function parkReview() {
    if (review) {
      queue = [review, ...queue];
      review = null;
    }
  }
  function nextReview() {
    review = queue[0] ?? null;
    queue = queue.slice(1);
    scroller.scrollTop = 0;
  }
  async function reviewSaved(id: number, msg: string) {
    review = null;
    await refresh();
    if (queue.length) {
      nextReview();
      note(`${msg}. ${queue.length + 1} more to review.`);
    } else {
      note(msg);
      await openById(id);
    }
  }
  async function pollInbox() {
    if (document.hidden) return;
    try {
      const got = await api.takeInbox();
      got.notes.forEach((n) => note(n));
      const launch = await api.launchNotes();
      if (launch.length) fail(launch.join(' '), false, null, true);
      if (got.notes.length) refresh();
      for (const rv of got.reviews) {
        const show = section === 'browser' && !editing && !cooking;
        queueReview(rv, show);
        if (!show) note(`“${rv.recipe.name || 'A recipe'}” arrived in the inbox`, { label: 'Review', run: () => { section = 'browser'; if (!review) nextReview(); } });
      }
    } catch {
      /* the next poll tries again */
    }
  }
  $effect(() => {
    const t = window.setInterval(pollInbox, 3000);
    return () => clearInterval(t);
  });

  // Sync with the shared folder when the app opens and every 5 minutes (only if it's set up).
  // A failure is reported once, not every time it retries.
  let syncFailed = false;
  async function autoSync() {
    try {
      const msg = await api.autoSync();
      if (msg != null) {
        syncFailed = false;
        await refresh();
      }
    } catch (e) {
      if (!syncFailed) fail(`Sync didn’t work: ${e}`);
      syncFailed = true;
    }
  }
  $effect(() => {
    autoSync();
    const t = window.setInterval(autoSync, 5 * 60_000);
    return () => clearInterval(t);
  });

  // A quiet update check 5 s after launch; a failure shows nothing and the next launch tries again.
  $effect(() => {
    const t = setTimeout(() => checkForUpdate(false), 5000);
    return () => clearTimeout(t);
  });
  // An open editor, a recipe under review or imported recipes waiting hold an update install back.
  holdsUnsaved(() => editing !== null || review !== null || queue.length > 0);
  // The Targets draft outlives its page, so an install waits for it even while the page is closed.
  holdsUnsaved(targetsDirty);
  // Rust enforces it where the install runs; this also runs on mount, so a reload clears a stale value.
  $effect(() => {
    api.setUnsaved(update.unsaved).catch((e) => fail(String(e)));
  });

  const busyEditing = () => (editing ? (fail('Save or cancel the recipe you’re editing first.'), true) : false);

  async function startEdit(id: number) {
    try {
      const draft = await api.draft(id);
      if (!open) savedScroll = scroller.scrollTop;
      editing = { id, draft };
      scroller.scrollTop = 0;
    } catch (e) {
      fail(String(e));
    }
  }

  async function saved(id: number) {
    const isNew = editing?.id === 0;
    editing = null;
    try {
      open = await api.recipe(id, 1, 'original');
    } catch (e) {
      open = null;
      fail(String(e));
    }
    scroller.scrollTop = 0;
    fail(isNew ? 'Recipe added' : 'Changes saved', true);
    await refresh();
  }

  async function chooseView(v: string) {
    if (busyEditing()) return;
    parkReview();
    section = 'recipes';
    if (open) await back(false);
    view = v;
    search = '';
    entering = true;
    await load();
    scroller.scrollTop = 0;
    setTimeout(() => (entering = false), 700);
  }

  async function openRecipe(c: Card) {
    let d: Detail;
    try {
      d = await api.recipe(c.id, 1, 'original');
    } catch (e) {
      return fail(String(e));
    }
    savedScroll = scroller.scrollTop;
    entering = false;
    heroId = c.id;
    await tick();
    await transition(() => {
      heroId = null;
      open = d;
      scroller.scrollTop = 0;
    });
  }

  async function back(animated = true) {
    if (!open) return;
    const id = open.card.id;
    const go = async () => {
      open = null;
      heroId = id;
      await tick();
      scroller.scrollTop = savedScroll;
    };
    if (animated) await transition(go);
    else await go();
    heroId = null;
  }

  function keys(e: KeyboardEvent) {
    if (editing || cooking || review || menu || section !== 'recipes') return; // those pages handle their own keys
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'n' && section === 'recipes') {
      e.preventDefault();
      startEdit(0);
    } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'f') {
      e.preventDefault();
      if (open) back(false);
      searchBox?.focus();
      searchBox?.select();
    } else if (e.key === 'Escape') {
      if (open) back();
      else if (search) { search = ''; load(); }
    } else if (e.altKey && e.key === 'ArrowLeft') {
      back();
    }
  }

  $effect(() => { refresh(); });
  $effect(() => { if (sort >= 0) remember('sort', String(sort)); remember('mode', mode); remember('foodUnits', foodUnits); });
  setTimeout(() => (entering = false), 900);
</script>

<svelte:window onkeydown={keys} onmouseup={(e) => { if (e.button === 3 && !editing && !cooking) back(); }} />

<div class="app">
  <Sidebar
    {ov}
    {section}
    {view}
    waiting={queue.length + (review ? 1 : 0)}
    onsection={(s) => { if (!busyEditing()) { parkReview(); planFor = null; section = s; scroller.scrollTop = 0; } }}
    onview={chooseView}
    onpinned={openById}
    onorder={async (order) => { try { await api.setSidebarOrder(order); ov = await api.overview(); } catch (e) { fail(String(e)); } }}
    onlaunch={async (tab) => { try { await api.setLaunchPage(tab); ov = await api.overview(); note(`Tiffin will open on ${api.tabNames[tab]}.`); } catch (e) { fail(String(e)); } }}
  />

  <main bind:this={scroller} onscroll={() => (scrolled = scroller.scrollTop > 4)}>
    <UpdateBanner />
    {#if editing}
      <div class="content">
        {#key editing}
          <RecipeEditor
            id={editing.id}
            initial={editing.draft}
            categories={ov?.categories ?? []}
            onsaved={saved}
            oncancel={() => { editing = null; if (!open) tick().then(() => (scroller.scrollTop = savedScroll)); }}
            onerror={fail}
            onnote={(m) => fail(m, true)}
          />
        {/key}
      </div>
    {:else if review}
      <div class="content">
        {#key review}
          <ReviewPage
            initial={review}
            categories={ov?.categories ?? []}
            waiting={queue.length}
            onsaved={reviewSaved}
            ondiscard={() => { nextReview(); if (!review) note('Discarded'); }}
            onerror={fail}
            onnote={note}
          />
        {/key}
      </div>
    {:else if section === 'browser'}
      <div class="content">
        <ImportPage
          {queue}
          onreview={(rv) => queueReview(rv, true)}
          onopen={(i) => { review = queue[i]; queue = queue.filter((_, j) => j !== i); scroller.scrollTop = 0; }}
          onsetupclaude={() => { settingsTab = 'claude'; section = 'settings'; scroller.scrollTop = 0; }}
          onerror={fail}
          onnote={note}
        />
      </div>
    {:else if section === 'groceries'}
      <GroceriesPage plan={planFor} onerror={fail} onnote={note} />
    {:else if section === 'meals' && ov}
      <MealsPage
        today={ov.today}
        categories={ov.categories}
        onopen={openById}
        onplan={(f, t) => { planFor = [f, t]; section = 'groceries'; }}
        onerror={fail}
        onnote={note}
      />
    {:else if section === 'settings'}
      <SettingsPage bind:tab={settingsTab} onchanged={refresh} onerror={fail} onnote={note} />
    {:else if section === 'menus' && ov}
      <MenusPage today={ov.today} onopen={openById} onerror={fail} onnote={note} />
    {:else if section === 'pantry'}
      <PantryPage onerror={fail} onnote={note} />
    {:else if section === 'foods'}
      <FoodsPage bind:units={foodUnits} onchanged={refresh} onerror={fail} onnote={note} />
    {:else if section === 'home'}
      <HomePage onerror={fail} ongoto={(s) => { if (!busyEditing()) { parkReview(); planFor = null; section = s; scroller.scrollTop = 0; } }} />
    {:else if section === 'reports'}
      <ReportsPage onerror={fail} ongoto={(s) => { if (!busyEditing()) { parkReview(); planFor = null; section = s; scroller.scrollTop = 0; } }} />
    {:else if section === 'diary'}
      <DiaryPage units={foodUnits} onerror={fail} onnote={note} ongoto={(s) => { if (!busyEditing()) { parkReview(); planFor = null; section = s; scroller.scrollTop = 0; } }} />
    {:else if section === 'targets'}
      <TargetsPage onerror={fail} onnote={note} />
    {:else if section === 'body'}
      <BodyPage onerror={fail} onnote={note} />
    {:else if section !== 'recipes'}
      <!-- a page whose data hasn't loaded yet shows nothing, rather than falling through to the recipe list -->
    {:else}
      <header class="toolbar" class:scrolled>
        {#if open}
          <button class="back" onclick={() => back()}><Icon name="back" size={18} /> {title}</button>
        {:else}
          <div class="heading">
            <h1>{title}</h1>
            <span class="count num">{cards.length} {cards.length === 1 ? 'recipe' : 'recipes'}</span>
          </div>
          <div class="tools">
            <Segmented label="Layout" options={['grid', 'list'] as ('grid' | 'list')[]} value={mode} onchange={(m) => (mode = m)}>
              {#snippet item(m)}<Icon name={m} size={16} /><span class="sr">{m === 'grid' ? 'Grid' : 'List'}</span>{/snippet}
            </Segmented>
            {#if view !== 'recent' && view !== 'top'}
              <Segmented label="Sort by" title="Sort by" options={[0, 1, 2]} value={Math.max(sort, 0)} status={(i) => `Sorted by ${sortNames[i]}`} onchange={(v) => { sort = v; load(); }}>
                {#snippet item(i)}{sortNames[i]}{/snippet}
              </Segmented>
            {/if}
            {#if view === 'trash' && cards.length}
              <button class="empty-trash" onclick={() => (confirmDelete = { ids: cards.map((c) => c.id), what: cards.length === 1 ? `“${cards[0].name}”` : `all ${cards.length} recipes in the Trash` })}>Empty Trash</button>
            {/if}
            <button class="new" onclick={() => startEdit(0)} title="New recipe (Ctrl+N)"><Icon name="plus" size={17} /><span class="sr">New recipe</span></button>
            <label class="search">
              <Icon name="search" size={16} />
              <input bind:this={searchBox} bind:value={search} oninput={(e) => { search = e.currentTarget.value; load(); }} placeholder="Search" aria-label="Search recipes" spellcheck="false" />
              {#if search}<button class="clear" aria-label="Clear search" onclick={() => { search = ''; load(); searchBox?.focus(); }}><Icon name="close" size={13} /></button>{/if}
            </label>
          </div>
        {/if}
      </header>

      <div class="content">
        {#if open}
          {#key open}
            <RecipeDetail
              initial={open}
              onchanged={refresh}
              onerror={fail}
              onedit={() => startEdit(open!.card.id)}
              oncook={(d) => (cooking = d)}
              onmenu={(x, y) => recipeMenu(open!.card, x, y)}
              onreview={() => api.reviewExisting(open!.card.id).then((rv) => { review = rv; scroller.scrollTop = 0; }, (e) => fail(String(e)))}
              onrestore={() => act(api.setTrashed([open!.card.id], false), 'Put back')}
              ondelete={() => (confirmDelete = { ids: [open!.card.id], what: `“${open!.card.name}”` })}
            />
          {/key}
        {:else if cards.length}
          <RecipeList {cards} {mode} {entering} {heroId} onopen={openRecipe} onmenu={recipeMenu} />
        {:else if ov}
          <div class="empty">
            <h2>{search ? `Nothing matches “${search}”` : view === 'trash' ? 'Trash is empty' : 'No recipes here yet'}</h2>
            <p>{search ? 'Search looks in names, ingredients, directions, notes and sources.' : 'Recipes you file under this shelf show up here.'}</p>
          </div>
        {/if}
      </div>
    {/if}
  </main>
</div>

{#if cooking}
  <CookMode d={cooking} onclose={() => (cooking = null)} onerror={fail} />
{/if}

<TimerTray />

{#if menu}<Menu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />{/if}
{#if mealFor && ov}
  <MealDialog id={mealFor.id} name={mealFor.name} today={ov.today} onclose={() => (mealFor = null)} ondone={(m) => { mealFor = null; note(m); }} onerror={fail} />
{/if}
{#if groceryFor}
  <GroceryDialog id={groceryFor.id} name={groceryFor.name} onclose={() => (groceryFor = null)} ondone={(m) => { groceryFor = null; note(m); }} onerror={fail} />
{/if}
{#if confirmDelete}
  {@const c = confirmDelete}
  <Dialog title="Delete Forever?" onclose={() => (confirmDelete = null)}>
    <p style="margin:0;color:var(--ink)">Delete {c.what} for good?</p>
    <p class="fine">They’ll be removed from every synced device. If you change your mind, Version History in Settings can still roll back.</p>
    {#snippet actions()}
      <button class="btn" onclick={() => (confirmDelete = null)}>Cancel</button>
      <button class="btn danger" onclick={() => deleteForever(c.ids)}>Delete Forever</button>
    {/snippet}
  </Dialog>
{/if}

{#if notice?.kind === 'replaced'}
  <Dialog title="Your library file was changed outside Tiffin" onclose={() => (notice = null)}>
    <p class="msg">Another program changed or damaged the library file, so it couldn’t be read. It was kept as {nameOf(notice.kept)} in {folderOf(notice.kept)}, and the library you have open was saved in its place. Nothing shown here was lost.</p>
    {#snippet actions()}
      <button class="btn primary" onclick={() => (notice = null)}>OK</button>
    {/snippet}
  </Dialog>
{:else if notice?.kind === 'recovered'}
  <Dialog title="Recovered from the backup copy" onclose={() => (notice = null)}>
    {#if notice.kept}
      <p class="msg">The main library file was damaged. It was kept as {nameOf(notice.kept)} in {folderOf(notice.kept)}, and your library was opened from its backup copy. The most recent changes may be missing.</p>
    {:else}
      <p class="msg">The library file was missing, so Tiffin restored its backup copy. The most recent changes may be missing.</p>
    {/if}
    {#snippet actions()}
      <button class="btn primary" onclick={() => (notice = null)}>OK</button>
    {/snippet}
  </Dialog>
{:else if notice?.kind === 'damaged'}
  <!-- blocking: Esc and the backdrop leave it open, because nothing may be saved until a choice is made -->
  <Dialog title="Your library file couldn’t be read" onclose={() => {}}>
    {#if notice.kept}
      <p class="msg">Your damaged library file was kept as {nameOf(notice.kept)} in {folderOf(notice.kept)}. Nothing is saved until you choose how to continue.</p>
    {:else}
      <p class="msg">Tiffin couldn’t make a safety copy of {notice.file} yet, so the file is left exactly as it is. Each choice below tries the copy again first. Nothing is saved until you choose how to continue.</p>
    {/if}
    <div class="choices">
      {#if savedVersions.length}
        {@const v = savedVersions[0]}
        <button class="btn" aria-disabled={recovering} onclick={() => recover(() => api.rollback(v.path))}>Restore the Newest Saved Version ({v.when})</button>
      {/if}
      {#if savedBackups.length}
        {@const b = savedBackups[0]}
        <button class="btn" aria-disabled={recovering} onclick={() => recover(() => api.restoreBackup(b.path))}>Restore the Newest Backup ({b.when})</button>
      {/if}
      <button class="btn" aria-disabled={recovering} onclick={() => !recovering && backupPicker?.click()}>Choose a Backup File…</button>
      <input bind:this={backupPicker} type="file" accept=".recipebox" hidden onchange={chooseBackupFile} />
      <button class="btn" aria-disabled={recovering} onclick={() => recover(() => api.startEmpty())}>Start an Empty Library</button>
    </div>
    <p class="fine" role="status">{recovering ? 'Restoring your library…' : ''}</p>
    {#snippet actions()}{/snippet}
  </Dialog>
{/if}

{#if toast}
  <div class="toast" class:ok={toastOk} role={toastOk ? 'status' : 'alert'} transition:fly={{ y: 16, duration: 220 }}>
    <Icon name={toastOk ? 'check' : 'warn'} size={17} />
    <span>{toast}</span>
    {#if toastAction}
      {@const a = toastAction}
      <button class="act" onclick={() => { toast = ''; a.run(); }}>{a.label}</button>
    {/if}
    <button aria-label="Dismiss" onclick={() => (toast = '')}><Icon name="close" size={14} /></button>
  </div>
{/if}

<style>
  .msg { margin: 0; color: var(--ink); overflow-wrap: anywhere; } /* file paths have no spaces to break at */
  .choices .btn[aria-disabled='true'] { opacity: 0.5; transform: none; pointer-events: none; }
  .choices { display: flex; flex-direction: column; gap: 8px; }
  .choices .btn { justify-content: flex-start; width: 100%; height: auto; min-height: 34px; padding-block: 7px; text-align: left; }
  .app { display: flex; height: 100%; }
  main { flex: 1; min-width: 0; overflow-y: auto; position: relative; scrollbar-gutter: stable; background: var(--page); }

  /* the toolbar is a material: content scrolls under it, blurred */
  .toolbar {
    position: sticky;
    top: 0;
    z-index: 5;
    display: flex;
    align-items: center;
    gap: 16px;
    min-height: 64px;
    padding: 8px 32px;
    background: var(--toolbar);
    backdrop-filter: blur(20px) saturate(170%);
    border-bottom: 1px solid transparent;
    transition: border-color 200ms ease-out;
  }
  .toolbar.scrolled { border-bottom-color: var(--rule); }
  .heading { display: flex; align-items: baseline; gap: 12px; min-width: 0; flex: 1; }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 24px;
    font-weight: 700;
    letter-spacing: -0.018em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .count { font-size: 16px; color: var(--ink-3); white-space: nowrap; }
  .tools { display: flex; align-items: center; flex-wrap: wrap; row-gap: 8px; column-gap: 10px; justify-content: flex-end; min-width: 0; }

  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 10px;
    border-radius: 9px;
    background: var(--well);
    box-shadow: inset 0 0 0 1px var(--rule);
    color: var(--ink-2);
    transition: box-shadow 150ms ease-out, background-color 150ms ease-out;
  }
  .search:focus-within { background: var(--card); box-shadow: 0 0 0 3px var(--focus), inset 0 0 0 1px var(--accent); }
  .search { flex: 1 1 160px; min-width: 140px; width: auto; }
  input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: none;
    background: none;
    color: var(--ink);
    font: inherit;
    font-size: 13.5px;
    caret-color: var(--accent);
  }
  input::placeholder { color: var(--ink-3); }
  .clear { display: grid; place-items: center; width: 18px; height: 18px; border-radius: 50%; background: var(--ink-3); color: var(--page); }
  .clear :global(svg) { stroke-width: 2.4; }

  .back {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 32px;
    padding: 0 12px 0 6px;
    margin-left: -8px;
    border-radius: 8px;
    color: var(--accent-text);
    font-weight: 600;
    transition: background-color 120ms ease-out;
  }
  .back:hover { background: var(--accent-wash); }
  .back:active { transform: scale(0.97); }

  .content { padding: 12px 32px 48px; }
  .empty { padding: 80px 0; text-align: center; color: var(--ink-2); }
  .empty h2 { margin: 0 0 6px; font-family: var(--font-display); font-size: 20px; color: var(--ink); }
  .empty p { margin: 0; }

  @keyframes rise { from { opacity: 0; transform: translateY(10px); } }

  .toast {
    position: fixed;
    left: 50%;
    bottom: 24px;
    translate: -50% 0;
    z-index: 20;
    display: flex;
    align-items: center;
    gap: 10px;
    max-width: min(560px, calc(100vw - 32px));
    padding: 10px 10px 10px 14px;
    border-radius: 12px;
    background: #2b2622;
    color: #f4efe9;
    font-size: 13.5px;
    box-shadow: var(--shadow-float);
  }
  .toast :global(svg) { flex: none; color: #f0b44c; }
  .toast.ok :global(svg) { color: var(--toast-ok); }
  .toast .act { width: auto; padding: 0 10px; font-weight: 600; color: var(--toast-act); }
  .empty-trash {
    height: 30px;
    padding: 0 12px;
    border-radius: 9px;
    font-size: 13px;
    font-weight: 600;
    color: var(--heart);
    background: color-mix(in srgb, var(--heart) 10%, transparent);
  }
  .empty-trash:hover { background: color-mix(in srgb, var(--heart) 18%, transparent); }
  .new {
    display: grid;
    place-items: center;
    width: 32px;
    height: 30px;
    border-radius: 9px;
    color: var(--accent-text);
    transition: background-color 120ms ease-out, transform 100ms ease-out;
  }
  .new:hover { background: var(--accent-wash); }
  .new:active { transform: scale(0.94); }
  .toast button { display: grid; place-items: center; width: 24px; height: 24px; border-radius: 6px; color: #c9c1b8; }
  .toast button:hover { background: rgba(255, 255, 255, 0.1); }
</style>
