<script lang="ts">
  import { tick, untrack } from 'svelte';
  import Dialog from './Dialog.svelte';
  import Icon from './Icon.svelte';
  import Photo from './Photo.svelte';
  import * as api from './api';
  import type { Card, MealType } from './api';

  // The meal plan's own recipe picker, shaped like the diary's Add Food sheet. It stays open, so a whole day goes in during one visit.
  let { day, kind, onclose, onadded }: {
    day: number;
    kind: MealType | null;
    onclose: () => void;
    /** a recipe was planned: the calendar behind reloads at once */
    onadded: () => void;
  } = $props();

  const when = $derived(api.dayLabel(day, { weekday: 'long', month: 'short', day: 'numeric' }));
  const target = $derived(kind ?? api.dayLabel(day, { weekday: 'long' }));
  const heading = $derived(kind ? `Add a Recipe to ${kind} on ${when}` : `Add a Recipe to ${when}`);

  let q = $state('');
  let recipes: Card[] = $state([]);
  let loaded = $state(false);
  let chosen: Card | null = $state(null);
  let status = $state('');
  let failure = $state('');
  let busy = false;
  let seq = 0;
  let box: HTMLInputElement | undefined = $state();
  let list: HTMLElement | undefined = $state();

  async function search(text: string) {
    const mine = ++seq; // a slower older answer never replaces a newer one
    try {
      const got = await api.list('all', text, 0);
      if (mine !== seq) return;
      recipes = got;
      loaded = true;
      chosen = got.find((c) => c.id === chosen?.id) ?? null;
    } catch (e) {
      // the toast sits behind a modal, so the notice is inline
      if (mine === seq) failure = `Couldn't load your recipes: ${api.reason(e)}.`;
    }
  }
  $effect(() => {
    box?.focus();
    untrack(() => search(''));
  });

  function typed(e: Event & { currentTarget: HTMLInputElement }) {
    q = e.currentTarget.value;
    status = '';
    search(q);
  }
  async function add(c: Card) {
    if (busy) return;
    busy = true;
    failure = '';
    try {
      await api.addMeal(c.id, day, kind);
      onadded();
      q = '';
      chosen = null;
      status = `Added ${c.name} to ${target}`;
      search('');
      await tick();
      box?.focus();
    } catch (e) {
      failure = `Couldn't plan that recipe: ${api.reason(e)}. Your meal plan is unchanged, so try again.`;
    } finally {
      busy = false;
    }
  }
  function cancel() {
    if (!q) return onclose();
    q = '';
    status = '';
    search('');
    box?.focus();
  }
  function fieldKeys(e: KeyboardEvent) {
    if (e.key === 'Enter') {
      e.preventDefault();
      if (chosen) add(chosen);
      else if (q.trim() && recipes[0]) chosen = recipes[0];
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      list?.querySelector<HTMLElement>('button.row')?.focus();
    }
  }
  function rowKeys(e: KeyboardEvent) {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    e.preventDefault();
    const all = [...(list?.querySelectorAll<HTMLElement>('button.row') ?? [])];
    const i = all.indexOf(e.currentTarget as HTMLElement);
    if (e.key === 'ArrowDown') all[i + 1]?.focus();
    else if (i > 0) all[i - 1].focus();
    else box?.focus();
  }
</script>

<!-- Esc undoes typed text and only then closes; a click outside and Done close at once -->
<Dialog wide title={heading} {onclose} oncancel={cancel}>
  <div class="pick">
    <div class="bar">
      <Icon name="search" size={18} />
      <input
        bind:this={box}
        class="q"
        value={q}
        oninput={typed}
        onkeydown={fieldKeys}
        placeholder="Search recipes"
        aria-label="Search recipes"
        autocomplete="off"
        spellcheck="false"
      />
    </div>
    <p class="status fine" role="status" aria-live="polite">{#if status}<Icon name="check" size={16} />{status}{/if}</p>
    {#if failure}
      <p class="notice" role="alert"><Icon name="warn" size={16} />{failure}</p>
    {/if}
    <div bind:this={list} class="list">
      <ul class="rows">
        {#each recipes as c (c.id)}
          <li>
            <button
              type="button"
              class="row label-scope"
              style:--l={api.labelVar(c.label_index)}
              aria-pressed={chosen?.id === c.id}
              onclick={() => (chosen = c)}
              ondblclick={() => add(c)}
              onkeydown={rowKeys}
            >
              <span class="thumb"><Photo photo={c.photo} seed={c.seed} /></span>
              <span class="name">{c.name}</span>
              {#if c.total_time}<span class="fine">{c.total_time}</span>{/if}
            </button>
          </li>
        {/each}
      </ul>
      {#if loaded && !recipes.length}
        <div class="empty">
          {#if q.trim()}
            <h3>No recipes match “{q.trim()}”</h3>
            <p>Check the spelling or try a shorter word.</p>
          {:else}
            <p>Your recipe box is empty. Save a recipe from the Browser first.</p>
          {/if}
        </div>
      {/if}
    </div>
  </div>
  {#snippet actions()}
    <button type="button" class="btn" onclick={onclose}>Done</button>
    <button
      type="button"
      class="btn primary"
      aria-disabled={chosen ? undefined : 'true'}
      onclick={() => { if (chosen) add(chosen); }}
    >Add to {target}</button>
  {/snippet}
</Dialog>

<style>
  .pick { display: flex; flex-direction: column; height: min(440px, calc(100vh - 220px)); min-height: 0; }
  .fine { font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .bar { display: flex; align-items: center; gap: 8px; height: 48px; margin-top: 4px; padding: 0 8px 0 16px; border-radius: 12px; background: var(--card); color: var(--ink-3); box-shadow: var(--shadow-card), inset 0 0 0 1px var(--rule); }
  .bar:focus-within { box-shadow: 0 0 0 3px var(--focus), inset 0 0 0 1px var(--accent); }
  /* input.q out-ranks the dialog's own field styles */
  .bar input.q { flex: 1; width: auto; height: 100%; min-width: 0; padding: 0; border: 0; border-radius: 0; outline: none; background: none; box-shadow: none; color: var(--ink); font: inherit; font-size: 15px; -webkit-user-select: text; user-select: text; }
  .bar input.q::placeholder { color: var(--ink-3); }
  .status { display: flex; align-items: flex-start; gap: 6px; min-height: 20px; margin: 8px 0; padding: 0 4px; }
  .status :global(svg) { flex: none; margin-top: 2px; }
  .notice { display: flex; align-items: flex-start; gap: 8px; margin: 0 0 8px; padding: 8px 16px; border-radius: 9px; background: var(--warn-wash); color: var(--warn); }
  .notice :global(svg) { flex: none; margin-top: 4px; }
  .btn[aria-disabled='true'] { opacity: 0.5; }
  .list { flex: 1; min-height: 0; overflow-y: auto; }
  .rows { list-style: none; margin: 0; padding: 0; }
  .row { display: flex; align-items: center; gap: 12px; width: 100%; min-height: 56px; padding: 8px 16px; border-bottom: 1px solid var(--rule); text-align: left; }
  .row:hover { background: var(--well); }
  .row[aria-pressed='true'] { background: var(--accent-wash); }
  .thumb { width: 40px; height: 40px; border-radius: 8px; overflow: hidden; flex: none; box-shadow: inset 0 0 0 2px var(--l); }
  .name { flex: 1; min-width: 0; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .empty { padding: 32px 16px; text-align: center; color: var(--ink-2); }
  .empty h3 { margin: 0 0 8px; font: 600 18px/1.2 var(--font-display); color: var(--ink); }
  .empty p { margin: 0 auto; max-width: 52ch; }
</style>
