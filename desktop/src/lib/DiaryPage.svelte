<script lang="ts">
  import { tick, untrack } from 'svelte';
  import Icon from './Icon.svelte';
  import SourceTag from './SourceTag.svelte';
  import AddFoodDialog from './AddFoodDialog.svelte';
  import EditEntryDialog from './EditEntryDialog.svelte';
  import PlannedMealsDialog from './PlannedMealsDialog.svelte';
  import Menu, { type MenuItem } from './Menu.svelte';
  import Meter from './Meter.svelte';
  import AddExerciseDialog from './AddExerciseDialog.svelte';
  import LogWeightDialog from './LogWeightDialog.svelte';
  import EditExerciseDialog from './EditExerciseDialog.svelte';
  import * as api from './api';
  import type { DayView, EntryView, ExerciseView, MacroGoal, MealType, Section, Totals } from './api';

  // One scrolling day: four meals, a summary bar that stays in view. The page does no nutrition
  // maths; every figure is printed as the crate returned it, only grouped "2,670" as the crate's own texts are.
  let { units, onerror, onnote, ongoto }: {
    units: 'metric' | 'us';
    /** Set Your Targets opens the Targets page */
    ongoto: (s: Section) => void;
    onerror: (m: string) => void;
    onnote: (m: string, action?: { label: string; run: () => void } | null) => void;
  } = $props();

  // the page opens on today every time the section opens: a stale day is how food lands on the wrong date
  const start = api.localDay();
  let today = $state(start);
  let day = $state(start);
  let view: DayView | null = $state(null);
  let busy = $state(false);
  let loadError = $state('');
  let picker: HTMLInputElement | undefined = $state();
  let seq = 0;
  /** the meal whose Add Food dialog is open, and the button to give focus back to */
  let adding: { slot: MealType; label: string; opener: HTMLElement } | null = $state(null);

  /** the one undo the crate holds, and until when the page offers it (10 s, as the toast stays) */
  let pending: { until: number } | null = null;
  let writing = false;

  /** a crate answer for the day being viewed replaces the page's view; a slower older load never overwrites it */
  function show(v: DayView) {
    seq++;
    busy = false;
    view = v;
  }
  const writeFailed = (e: unknown) => onerror(`Couldn't save that change: ${api.reason(e)}. Your diary is unchanged, so try again.`);
  function offer(msg: string) {
    pending = { until: Date.now() + 10000 };
    onnote(msg, { label: 'Undo', run: undo });
  }
  async function undo() {
    if (!pending || Date.now() > pending.until) return;
    pending = null;
    try {
      const r = await api.undoDiary(day, today);
      show(r.view);
      onnote(r.day === day ? 'Put back' : `Put back on ${dateOf(r.day)}`);
    } catch (e) {
      onerror(`Couldn't undo that: ${api.reason(e)}. Your diary is unchanged.`);
      load();
    }
  }
  /** Delete on a row: no confirm, the undo is the safety net; focus stays inside the meal */
  async function remove(slot: MealType, uid: string) {
    if (writing) return;
    const list = view?.slots.find((s) => s.slot === slot)?.entries ?? [];
    const i = list.findIndex((x) => x.uid === uid);
    const near = list[i + 1] ?? list[i - 1];
    const at = day;
    writing = true;
    try {
      const r = await api.deleteEntry(at, today, uid);
      if (day === at) show(r.view);
      offer(`Deleted ${r.name}`);
      await tick();
      (near && document.querySelector<HTMLElement>(`[data-uid="${CSS.escape(near.uid)}"]`) || document.querySelector<HTMLElement>(`[data-add="${slot}"]`))?.focus();
    } catch (e) {
      writeFailed(e);
      load();
    } finally {
      writing = false;
    }
  }

  /** Delete on an exercise row: the same one undo as food */
  async function removeExercise(uid: string) {
    if (writing) return;
    const list = view?.exercise.entries ?? [];
    const i = list.findIndex((x) => x.uid === uid);
    const near = list[i + 1] ?? list[i - 1];
    const at = day;
    writing = true;
    try {
      const r = await api.deleteExercise(at, today, uid);
      if (day === at) show(r.view);
      offer(`Deleted ${r.name}`);
      await tick();
      (near && document.querySelector<HTMLElement>(`[data-uid="${CSS.escape(near.uid)}"]`) || document.querySelector<HTMLElement>('[data-add="exercise"]'))?.focus();
    } catch (e) {
      writeFailed(e);
      load();
    } finally {
      writing = false;
    }
  }

  const entries = (n: number) => (n === 1 ? '1 entry' : `${n} entries`);

  /** Copy the day before the viewed day into it; copies sit next to what is there */
  async function copyPrevious() {
    if (!view || writing) return;
    const at = day, from = view.prev, wasToday = view.is_today;
    writing = true;
    try {
      const r = await api.copyEntries(from, at, today, null);
      if (day === at) show(r.view);
      offer(`Copied ${entries(r.count)} from ${wasToday ? 'yesterday' : 'the previous day'}`);
    } catch (e) {
      writeFailed(e);
      load();
    } finally {
      writing = false;
    }
  }
  /** one meal of a past day into the same meal today; the viewed day stays as it is */
  async function copyMeal(s: { slot: MealType; label: string }) {
    if (writing) return;
    writing = true;
    try {
      const r = await api.copyEntries(day, today, today, s.slot);
      offer(`Copied ${entries(r.count)} from ${s.label} to today`);
    } catch (e) {
      writeFailed(e);
      load();
    } finally {
      writing = false;
    }
  }
  async function toggleComplete() {
    if (!view || writing) return;
    const at = day;
    writing = true;
    try {
      const v = await api.setDayComplete(at, today, !view.complete);
      if (day === at) show(v);
    } catch (e) {
      writeFailed(e);
      load();
    } finally {
      writing = false;
    }
  }

  let menu: { x: number; y: number; items: MenuItem[] } | null = $state(null);
  let planning = $state(false);
  function more(e: MouseEvent) {
    if (!view) return;
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const items: MenuItem[] = [{ label: view.is_today ? 'Copy Yesterday' : 'Copy the Previous Day', icon: 'copy', disabled: view.previous_day_count === 0, run: copyPrevious }];
    if (view.is_today && view.planned_count > 0) items.push({ label: "Log Today's Planned Meals…", icon: 'calendar', run: () => (planning = true) });
    menu = { x: r.right - 260, y: r.bottom + 8, items };
  }

  async function load() {
    const mine = ++seq; // a slower older answer never replaces a newer one
    busy = true;
    try {
      const v = await api.diaryDay(day, today);
      if (mine === seq) { view = v; loadError = ''; }
    } catch (e) {
      if (mine === seq) loadError = api.reason(e);
      onerror(String(e));
    } finally {
      if (mine === seq) busy = false;
    }
  }
  $effect(() => {
    day; today;
    untrack(load);
  });

  // the date is built from the day key's own parts, never parsed as a date string (that is UTC)
  function dateOf(key: string) {
    const [y, m, d] = key.split('-').map(Number);
    const opts: Intl.DateTimeFormatOptions = { weekday: 'short', month: 'short', day: 'numeric' };
    if (y !== Number(today.slice(0, 4))) opts.year = 'numeric';
    return new Date(y, m - 1, d).toLocaleDateString(undefined, opts);
  }
  const dateText = $derived(dateOf(day));
  const ex = $derived.by(() => view?.exercise ?? null);

  const NET_HINT = "Carbs minus fiber. A food that doesn't report fiber counts as 0 fiber here.";
  const cells: { key: keyof Totals; label: string; unit: string; nutrient: string; dot?: 'p' | 'c' | 'f'; tone?: 'protein' | 'carbs' | 'fat' }[] = [
    { key: 'calories', label: 'Calories', unit: 'kcal', nutrient: 'calories' },
    { key: 'protein', label: 'Protein', unit: 'g', nutrient: 'protein', dot: 'p', tone: 'protein' },
    { key: 'carbs', label: 'Carbs', unit: 'g', nutrient: 'carbs', dot: 'c' },
    { key: 'fat', label: 'Fat', unit: 'g', nutrient: 'fat', dot: 'f', tone: 'fat' },
    { key: 'fiber', label: 'Fiber', unit: 'g', nutrient: 'fiber' },
    { key: 'net_carbs', label: 'Net Carbs', unit: 'g', nutrient: 'carbs', tone: 'carbs' },
  ];
  /** the crate's goal for a cell that has one: protein, net carbs and fat (total carbs and fiber have none) */
  function goalOf(key: keyof Totals): MacroGoal | null {
    const b = view?.budget;
    return !b ? null : key === 'protein' ? b.protein : key === 'net_carbs' ? b.carbs : key === 'fat' ? b.fat : null;
  }
  // Sentences for the states the crate reports with a case but no text of its own
  const NO_TARGET_YET = "Set a daily calorie target to see what's left each day.";
  const NO_BUDGET = "Your targets can't be read right now, so the daily budget isn't shown.";

  // shortcut: native date picker; swap for a small month picker if WebView2 anchors the popup badly, when a user reports it
  function choose() {
    try {
      picker?.showPicker();
    } catch { /* showPicker refuses outside a click; there is nothing else to open */ }
  }
  function picked(e: Event & { currentTarget: HTMLInputElement }) {
    if (e.currentTarget.value) day = e.currentTarget.value;
  }
  /** Alt+Left and Alt+Right move a day, Ctrl+Z runs the pending undo; neither while a dialog is open or a field has the keys */
  function keys(e: KeyboardEvent) {
    const t = e.target;
    if (document.querySelector('dialog[open]') || t instanceof HTMLInputElement || t instanceof HTMLSelectElement || t instanceof HTMLTextAreaElement) return;
    if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'z') {
      if (pending && Date.now() <= pending.until) { e.preventDefault(); undo(); }
      return;
    }
    if (!e.altKey || !view || (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight')) return;
    e.preventDefault();
    day = e.key === 'ArrowLeft' ? view.prev : view.next;
  }
  /** back from a long absence (or past midnight): re-read today; the viewed day does not jump */
  function visible() {
    if (document.visibilityState === 'visible') today = api.localDay();
  }

  function openAdd(s: { slot: MealType; label: string }, opener: HTMLElement) {
    adding = { slot: s.slot, label: s.label, opener };
  }
  /** the entry whose Edit Entry dialog is open, and the row to give focus back to */
  let editing: { entry: EntryView; slot: MealType; opener: HTMLElement } | null = $state(null);
  function closeEdit() {
    const g = editing;
    editing = null;
    // a moved or deleted entry's row is gone: fall back to the meal's Add Food button
    queueMicrotask(() => (document.querySelector<HTMLElement>(`[data-uid="${CSS.escape(g?.entry.uid ?? '')}"]`) ?? (g?.opener.isConnected ? g.opener : document.querySelector<HTMLElement>(`[data-add="${g?.slot}"]`)))?.focus());
  }
  function closeAdd() {
    const back = adding?.opener;
    adding = null;
    queueMicrotask(() => back?.focus());
  }
  /** the button that opened Add Exercise, to give focus back to */
  let addingExercise: { opener: HTMLElement } | null = $state(null);
  /** the workout whose Edit Exercise dialog is open, and the row to give focus back to */
  let editingExercise: { entry: ExerciseView } | null = $state(null);
  function closeEditExercise() {
    const g = editingExercise;
    editingExercise = null;
    // a deleted workout's row is gone: fall back to the Add Exercise button
    queueMicrotask(() => (document.querySelector<HTMLElement>(`[data-uid="${CSS.escape(g?.entry.uid ?? '')}"]`) ?? document.querySelector<HTMLElement>('[data-add="exercise"]'))?.focus());
  }
  function closeAddExercise() {
    const back = addingExercise?.opener;
    addingExercise = null;
    queueMicrotask(() => back?.focus());
  }
  /** the Log Weight button, to give focus back to */
  let loggingWeight: { opener: HTMLElement } | null = $state(null);
  function closeLogWeight() {
    const back = loggingWeight?.opener;
    loggingWeight = null;
    queueMicrotask(() => back?.focus());
  }
</script>

<svelte:window onkeydown={keys} />
<svelte:document onvisibilitychange={visible} />

{#snippet val(x: number | null, said: string)}{#if x == null}<span aria-hidden="true">–</span><span class="sr">{said}</span>{:else}{x.toLocaleString('en-US', { maximumFractionDigits: 20 })}{/if}{/snippet}
{#snippet macros(t: { protein: number | null; carbs: number | null; fat: number | null }, said: string)}
  <span class="m"><i class="dot p" aria-hidden="true"></i><span class="caps">P</span> <span class="num">{@render val(t.protein, said)}</span></span>
  <span class="m"><i class="dot c" aria-hidden="true"></i><span class="caps">C</span> <span class="num">{@render val(t.carbs, said)}</span></span>
  <span class="m"><i class="dot f" aria-hidden="true"></i><span class="caps">F</span> <span class="num">{@render val(t.fat, said)}</span></span>
{/snippet}

<div class="diary">
  <div class="head">
    <header class="nav">
      <button type="button" class="arrow" aria-label="Previous day" title="Previous day (Alt+Left)" onclick={() => view && (day = view.prev)}><Icon name="back" size={18} /></button>
      <button type="button" class="arrow flip" aria-label="Next day" title="Next day (Alt+Right)" onclick={() => view && (day = view.next)}><Icon name="back" size={18} /></button>
      <h1>
        <button type="button" class="datebtn" onclick={choose}>{dateText}{#if view?.complete}<span class="done"><Icon name="check" size={16} /></span>{/if}<span class="chev"><Icon name="chevron" size={14} /></span><span class="sr">{view?.complete ? ', day complete' : ''}. Choose a date</span></button>
        <input bind:this={picker} class="picker" type="date" min="2000-01-01" value={day} onchange={picked} tabindex="-1" aria-hidden="true" />
      </h1>
      {#if day !== today}<button type="button" class="chip" onclick={() => (day = today)}>Show Today</button>{/if}
      {#if view}
        <div class="right">
          <button type="button" class="btn" onclick={(e) => (loggingWeight = { opener: e.currentTarget })}>Log Weight</button>
          <button type="button" class="btn complete" class:on={view.complete} aria-pressed={view.complete} title="Mark this day as fully logged. You can still edit it." onclick={toggleComplete}>
            {#if view.complete}<Icon name="check" size={16} />Day Complete{:else}Mark Day Complete{/if}
          </button>
          <button type="button" class="icon" aria-label="More diary actions" aria-haspopup="menu" onclick={more}><Icon name="more" size={18} /></button>
        </div>
      {/if}
    </header>

    {#if view}
      {@const b = view.budget}
      <div class="bar" role="group" aria-label="Day totals">
        <div class="cells">
          {#if b}
            <!-- the crate's label, figure and sentence; over is amber words, never a sign or red -->
            <div class="cell rem" class:over={b.over} title={b.remaining == null ? b.said : undefined}>
              <span class="lab caps" aria-hidden="true">{b.label}{#if b.template}<SourceTag tag={b.template} title={b.template_title ?? undefined} />{/if}</span>
              <span class="fig" aria-hidden="true">
                <span class="num">{b.figure_text ?? '–'}</span>{#if b.figure != null}<span class="unit"> kcal</span>{/if}
              </span>
              <span class="sr">{b.said}</span>
            </div>
          {/if}
          {#each cells as c (c.key)}
            {@const v = view.totals[c.key]}
            {@const g = goalOf(c.key)}
            <div class="cell" class:cal={c.key === 'calories'} class:over={g?.over} title={v == null ? `At least one food here doesn't report ${c.nutrient}.` : undefined}>
              <span class="lab caps" title={c.key === 'net_carbs' ? NET_HINT : undefined} aria-describedby={c.key === 'net_carbs' ? 'net-hint' : undefined}>
                {#if c.dot}<i class="dot {c.dot}" aria-hidden="true"></i>{/if}{c.label}
              </span>
              <span class="fig" class:goal={g}>
                <span class="num">{@render val(v, 'not reported for some foods')}</span>{#if g}<span class="unit"> {g.unit_text}</span>{:else if v != null}<span class="unit"> {c.unit}</span>{/if}
              </span>
              {#if g && c.tone}<Meter value={g.eaten} goal={g.goal} label={c.label} tone={c.tone} said={g.said} over={g.over} />{/if}
            </div>
          {/each}
        </div>
        {#if b}
          <p class="formula fine">
            <span>{b.formula}</span>
            {#if b.raised_note}<span class="warn">{b.raised_note}</span>{/if}
            {#if b.exercise_note}<span>{b.exercise_note}</span>{/if}
          </p>
        {:else if view.budget_state === 'never'}
          <p class="formula fine">
            <span>{NO_TARGET_YET}</span>
            <button type="button" class="btn ghost" onclick={() => ongoto('targets')}>Set Your Targets</button>
          </p>
        {:else if view.budget_state === 'before'}
          <p class="formula fine">{view.budget_note ?? 'No target was set for this day.'}</p>
        {:else if view.budget_state === 'unavailable'}
          <p class="formula fine">{NO_BUDGET}</p>
        {/if}
        <span id="net-hint" class="sr">{NET_HINT}</span>
      </div>
    {/if}
  </div>

  <div class="content" aria-busy={busy}>
    {#if loadError}
      <div class="notice" role="alert">
        <Icon name="warn" size={16} />
        <p>Couldn't open this day: {loadError}. Press Try Again, or restart Tiffin if it keeps happening.</p>
        <button type="button" class="btn ghost" onclick={load}>Try Again</button>
      </div>
    {/if}
    {#if view}
      {#if view.slots.every((s) => !s.entries.length) && !view.exercise.entries.length}
        <div class="empty">
          <p>Nothing logged for this day yet.</p>
          {#if view.previous_day_count > 0}
            <button type="button" class="btn ghost" onclick={copyPrevious}>{view.is_today ? 'Copy Yesterday' : 'Copy the Previous Day'}</button>
          {/if}
          {#if view.is_today && view.planned_count > 0}
            <button type="button" class="btn ghost" onclick={() => (planning = true)}>Log Today's Planned Meals</button>
          {/if}
        </div>
      {/if}
      <div class="meals">
        {#each view.slots as s (s.slot)}
          <section aria-labelledby="meal-{s.slot}">
            <div class="mhead">
              <h2 id="meal-{s.slot}" class="caps">{s.label}</h2>
              {#if s.entries.length}
                <p class="sub">
                  <span class="kcal"><span class="num">{@render val(s.totals.calories, 'not reported for some foods')}</span> kcal</span>
                  <span class="macros fine">{@render macros(s.totals, 'not reported for some foods')}</span>
                </p>
              {/if}
              {#if !view.is_today && s.entries.length}
                <button type="button" class="icon copy" aria-label="Copy {s.label} to Today" title="Copy {s.label} to Today" onclick={() => copyMeal(s)}><Icon name="copy" size={18} /></button>
              {/if}
              <button type="button" class="btn ghost add" data-add={s.slot} onclick={(e) => openAdd(s, e.currentTarget)}>
                <Icon name="plus" size={16} />Add Food<span class="sr"> to {s.label}</span>
              </button>
            </div>
            {#if s.entries.length}
              <ul class="entries">
                {#each s.entries as e (e.uid)}
                  <li>
                    <button type="button" class="entry" data-uid={e.uid} onclick={(ev) => (editing = { entry: e, slot: s.slot, opener: ev.currentTarget })} onkeydown={(k) => { if (k.key === 'Delete') { k.preventDefault(); remove(s.slot, e.uid); } }}>
                    <span class="l1">
                      <span class="name" title={e.name}>{e.name}</span>
                      <SourceTag tag={e.tag} />
                      <span class="kc"><span class="num">{@render val(e.calories, 'not reported')}</span> <span class="fine">kcal</span></span>
                    </span>
                    <span class="l2 fine">
                      <span>{e.amount_text}</span>
                      {@render macros(e, 'not reported')}
                    </span>
                    </button>
                  </li>
                {/each}
              </ul>
            {:else}
              <p class="none">Nothing in {s.label} yet.</p>
            {/if}
          </section>
        {/each}
        {#if ex}
        <section aria-labelledby="meal-exercise">
          <div class="mhead">
            <h2 id="meal-exercise" class="caps">Exercise</h2>
            {#if ex.total_text}
              <p class="sub">
                <span class="kcal"><span class="num">{#if ex.entries.every((x) => x.kcal == null)}<span aria-hidden="true">–</span><span class="sr">no weight on record</span>{:else}{ex.total_text}{/if}</span> <span class="fine">net</span></span>
                {#if ex.counted_text}<span class="fine">{ex.counted_text}</span>{/if}
              </p>
            {/if}
            <button type="button" class="btn ghost add" data-add="exercise" onclick={(e) => (addingExercise = { opener: e.currentTarget })}>
              <Icon name="plus" size={16} />Add Exercise
            </button>
          </div>
          {#if ex.entries.length}
            <ul class="entries">
              {#each ex.entries as x (x.uid)}
                <li>
                  <button type="button" class="entry" data-uid={x.uid} onclick={() => (editingExercise = { entry: x })} onkeydown={(k) => { if (k.key === 'Delete') { k.preventDefault(); removeExercise(x.uid); } }}>
                    <span class="l1">
                      <span class="name" title={x.name}>{x.name}</span>
                      <SourceTag tag={x.tag} />
                      {#if x.edited}<SourceTag tag="edited" />{/if}
                      <span class="kc"><span class="num">{#if x.kcal_text == null}<span aria-hidden="true">–</span><span class="sr">no weight on record</span>{:else}{x.kcal_text}{/if}</span> <span class="fine">kcal</span></span>
                    </span>
                    <span class="l2 fine">
                      <span>{x.minutes_text}</span>
                      <span>{x.basis_text}</span>
                      {#if x.weight_text}<span>{x.weight_text}</span>{/if}
                    </span>
                  </button>
                </li>
              {/each}
            </ul>
          {:else}
            <p class="none">No exercise logged yet.</p>
          {/if}
        </section>
        {/if}
      </div>
    {/if}
  </div>
</div>

{#if editing && view}
  <EditEntryDialog
    {day}
    {today}
    entry={editing.entry}
    slot={editing.slot}
    us={units === 'us'}
    onclose={closeEdit}
    onsaved={(v, message) => { show(v); offer(message); closeEdit(); }}
    ondeleted={(v, name) => { show(v); offer(`Deleted ${name}`); closeEdit(); }}
    onfailed={load}
    {onerror}
  />
{/if}

{#if menu}<Menu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />{/if}
{#if planning}
  <PlannedMealsDialog
    {today}
    onclose={() => (planning = false)}
    onlogged={(v, n) => {
      if (day === today) show(v);
      offer(`Logged ${n === 1 ? '1 planned meal' : `${n} planned meals`}`);
    }}
    {onerror}
  />
{/if}

{#if editingExercise && view}
  <EditExerciseDialog
    {day}
    {today}
    entry={editingExercise.entry}
    onclose={closeEditExercise}
    onsaved={(v, message) => { show(v); offer(message); closeEditExercise(); }}
    ondeleted={(v, name) => { show(v); offer(`Deleted ${name}`); closeEditExercise(); }}
    onfailed={load}
  />
{/if}

{#if addingExercise && view}
  <AddExerciseDialog
    {day}
    {today}
    onclose={closeAddExercise}
    onlogged={(v, name) => {
      show(v);
      // the crate's log_exercise drops the journal's undo, so the page drops it too and the old Undo toast is replaced
      if (pending) { const live = Date.now() <= pending.until; pending = null; if (live) onnote(`Added ${name}`); }
    }}
    onfailed={load}
    {onerror}
  />
{/if}

{#if loggingWeight && view}
  <LogWeightDialog
    {day}
    {today}
    {dateText}
    onclose={closeLogWeight}
    onlogged={(text) => { load(); offer(`Logged ${text} for ${day === today ? 'today' : dateText}`); }}
    onupdated={(message) => { load(); onnote(message); }}
    {onerror}
  />
{/if}

{#if adding && view}
  <AddFoodDialog
    {day}
    {today}
    slot={adding.slot}
    label={adding.label}
    us={units === 'us'}
    onclose={closeAdd}
    onlogged={(v, name) => {
      show(v);
      // the crate's log_food and quick_add drop the journal's undo, so the page drops it too and the old Undo toast is replaced
      if (pending) { const live = Date.now() <= pending.until; pending = null; if (live) onnote(`Added ${name}`); }
    }}
    onfailed={load}
    {onerror}
  />
{/if}

<style>
  /* the column is 840 px of content; every layout threshold below is this column's own width, never the window's */
  .diary { container-type: inline-size; --diary-ink: color-mix(in srgb, var(--tile-diary), var(--ink) 35%); max-width: 904px; margin: 0 auto; padding: 0 32px 64px; font-size: 14.5px; line-height: 1.45; }
  .fine { font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .head {
    position: sticky;
    top: 0;
    z-index: 4;
    margin: 0 -32px;
    padding: 0 32px 8px;
    background: var(--toolbar);
    backdrop-filter: blur(20px) saturate(170%);
  }
  .nav { display: flex; align-items: center; flex-wrap: wrap; gap: 8px 4px; min-height: 64px; }
  .arrow { display: grid; place-items: center; width: 32px; height: 32px; border-radius: 9px; color: var(--ink-2); }
  .arrow:hover { background: var(--well); color: var(--ink); }
  .flip :global(svg) { transform: scaleX(-1); }
  h1 { position: relative; margin: 0 8px 0 4px; font: 700 24px var(--font-display); letter-spacing: -0.018em; color: var(--diary-ink); }
  .datebtn { display: inline-flex; align-items: center; gap: 8px; padding: 0 8px; border-radius: 9px; font: inherit; letter-spacing: inherit; color: inherit; white-space: nowrap; }
  .datebtn:hover { background: var(--well); }
  .chev { display: inline-grid; transform: rotate(90deg); }
  .picker { position: absolute; inset: 0; width: 100%; height: 100%; opacity: 0; pointer-events: none; }
  .right { display: flex; align-items: center; gap: 8px; margin-left: auto; }
  .icon { display: grid; place-items: center; width: 32px; height: 32px; border-radius: 9px; color: var(--accent-text); }
  .icon:hover { background: var(--accent-wash); }
  .icon.copy { margin-left: auto; }
  .icon.copy + .add { margin-left: 0; }
  .btn.complete.on { background: var(--accent-wash); color: var(--accent-text); }
  .done { display: inline-grid; color: var(--ok); }
  .chip { height: 28px; padding: 0 16px; border-radius: 14px; font-size: 13px; font-weight: 600; color: var(--diary-ink); background: color-mix(in srgb, var(--tile-diary) 12%, transparent); }
  .chip:hover { background: color-mix(in srgb, var(--tile-diary) 20%, transparent); }

  .bar { padding: 16px 24px; border-radius: 14px; background: var(--card); box-shadow: var(--shadow-card), inset 0 0 0 1px var(--rule); }
  .cells { display: flex; gap: 16px; }
  .cell { flex: 1 1 88px; display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .cell.cal { flex: 1.4 1 128px; }
  .cell.rem { flex: 1.6 1 160px; }
  .rem .fig { font-size: 26px; line-height: 1.1; }
  /* over a budget or a goal: amber words (--warn), never red */
  .cell.over .fig, .cell.over .unit, .rem.over .lab { color: var(--warn); }
  .rem .lab { gap: 8px; }
  .fig.goal { white-space: normal; }
  .fig.goal .num { margin-right: 4px; } /* the gap ends the first line when the unit wraps, so the unit never starts indented */
  .fig.goal .unit { display: inline-block; white-space: nowrap; } /* "over by 332 g" wraps whole, never "over / by" */
  .formula { display: flex; flex-wrap: wrap; align-items: center; gap: 0 16px; margin: 8px 0 0; }
  .formula .warn { color: var(--warn); }
  .formula .btn { height: auto; padding: 0; min-height: 0; }
  .lab { display: flex; align-items: center; gap: 4px; font-size: 13px; line-height: 1.4; color: var(--ink-2); white-space: nowrap; }
  .fig { display: block; font-size: 21px; line-height: 1.2; white-space: nowrap; }
  .cal .fig { font-size: 26px; line-height: 1.1; }
  .unit { font-size: 13px; color: var(--ink-2); }
  .dot { width: 8px; height: 8px; border-radius: 50%; flex: none; }
  .dot.p { background: var(--protein); }
  .dot.c { background: var(--carbs); }
  .dot.f { background: var(--fat); }

  .notice { display: flex; align-items: center; gap: 16px; margin: 24px 0 0; padding: 8px 16px; border-radius: 14px; background: var(--warn-wash); color: var(--warn); }
  .notice :global(svg) { flex: none; }
  .notice p { flex: 1; margin: 0; }
  .empty { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 16px; margin: 24px 0 0; padding: 16px 24px; border-radius: 14px; background: var(--well); color: var(--ink-2); }
  .empty p { margin: 0; }
  .meals { display: flex; flex-direction: column; gap: 24px; margin-top: 24px; }
  .mhead { display: flex; align-items: center; gap: 16px; min-height: 48px; }
  .mhead h2 { margin: 0; font-size: 14.5px; color: var(--ink); }
  .sub { flex: 1; display: flex; align-items: baseline; justify-content: flex-end; flex-wrap: wrap; gap: 4px 16px; min-width: 0; margin: 0; }
  .kcal { font-weight: 600; }
  .macros { display: inline-flex; align-items: center; gap: 8px; }
  .add { margin-left: auto; }
  .m { display: inline-flex; align-items: center; gap: 4px; }

  .entries { list-style: none; margin: 0; padding: 0; border-top: 1px solid var(--rule); }
  .entry { display: flex; flex-direction: column; justify-content: center; gap: 4px; width: 100%; min-height: 56px; padding: 8px 16px; border-bottom: 1px solid var(--rule); text-align: left; font: inherit; color: inherit; }
  .entry:hover { background: var(--well); }
  .l1 { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .name { flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
  .kc { flex: none; font-weight: 600; }
  .kc .fine { font-weight: 400; }
  .l2 { display: flex; flex-wrap: wrap; align-items: center; gap: 4px 16px; }
  .none { display: flex; align-items: center; min-height: 48px; margin: 0; padding: 0 16px; font-size: 13px; line-height: 1.4; color: var(--ink-3); border-top: 1px solid var(--rule); }

  /* under 640 px of column the date and arrows scroll away and only the totals stay, as 3 x 2 */
  @container (max-width: 639px) {
    .head { display: contents; }
    .bar { position: sticky; top: 0; z-index: 4; }
    .cells { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); }
    .cell.rem { grid-column: 1 / -1; }
    .macros { display: none; }
  }
</style>
