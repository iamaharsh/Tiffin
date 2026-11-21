<script lang="ts">
  import Dialog from './Dialog.svelte';
  import Icon from './Icon.svelte';
  import { tick } from 'svelte';
  import * as api from './api';
  import type { BodyRange, BodyView, MeasureKind, MeasureOption } from './api';

  // Manage Measures: the built-in measures (fixed), the person's own (add, archive, restore). There is no delete: an archived
  // measure keeps every value, and restoring it brings its column back.
  let { view, range, today, onclose, onview, onarchived }: {
    view: BodyView;
    range: BodyRange;
    today: string;
    onclose: () => void;
    /** the crate's refreshed page after an add or a restore */
    onview: (v: BodyView) => void;
    /** the crate's refreshed page after an archive, with what was archived so the page can offer Undo */
    onarchived: (v: BodyView, m: MeasureOption) => void;
  } = $props();

  const kinds: { id: MeasureKind; name: string }[] = [
    { id: 'mass', name: 'Weight' },
    { id: 'length', name: 'Length' },
    { id: 'percent', name: 'Percent' },
    { id: 'number', name: 'Number' },
  ];
  const kindName = (k: MeasureKind) => kinds.find((x) => x.id === k)?.name ?? k;
  // shortcut: the crate lists its eight built-in measures first, then the person's own, so the split is by position; upgrade when MeasureOption says which is which
  const builtIn = $derived(view.measures.slice(0, 8));
  const own = $derived(view.measures.slice(8));

  let name = $state('');
  let kind: MeasureKind = $state('length');
  let problem = $state('');
  let failure = $state('');
  let busy = false;
  let nameBox: HTMLInputElement | undefined = $state();
  let root: HTMLDivElement | undefined = $state();

  // the button that was pressed leaves the list, so focus would fall to nowhere: it moves to the next button in that list, or to the Name box
  async function stay(within: string) {
    await tick();
    (root?.querySelector<HTMLElement>(`${within} .btn.ghost`) ?? nameBox)?.focus();
  }

  async function add() {
    if (busy) return;
    busy = true;
    problem = '';
    try {
      onview(await api.saveMeasure(name, kind, range, today));
      name = '';
    } catch (e) {
      problem = `${api.reason(e)}.`;
    } finally {
      busy = false;
    }
  }

  async function archive(m: MeasureOption) {
    if (busy) return;
    busy = true;
    failure = '';
    try {
      onarchived(await api.archiveMeasure(m.id, true, range, today), m);
      stay('.own');
    } catch (e) {
      failure = `Couldn't save that change: ${api.reason(e)}. Your measures are unchanged, so try again.`;
    } finally {
      busy = false;
    }
  }

  async function restore(m: MeasureOption) {
    if (busy) return;
    busy = true;
    failure = '';
    try {
      onview(await api.archiveMeasure(m.id, false, range, today));
      stay('details');
    } catch (e) {
      failure = `Couldn't save that change: ${api.reason(e)}. Your measures are unchanged, so try again.`;
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title="Manage Measures" wide {onclose}>
  <div bind:this={root} class="manage">
  <section>
    <h3 class="caps">Built in</h3>
    <ul>
      {#each builtIn as m (m.id)}
        <li><span class="nm" title={m.name}>{m.name}</span><span class="kind">{kindName(m.kind)}</span></li>
      {/each}
    </ul>
  </section>

  <section class="own">
    <h3 class="caps">Your measures</h3>
    {#if own.length}
      <ul>
        {#each own as m (m.id)}
          <li>
            <span class="nm" title={m.name}>{m.name}</span><span class="kind">{kindName(m.kind)}</span>
            <button type="button" class="btn ghost" aria-label="Archive Measure {m.name}" onclick={() => archive(m)}>Archive Measure</button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="fine">You haven't added any measures of your own.</p>
    {/if}
  </section>

  <section>
    <h3 class="caps">Add a measure</h3>
    <form onsubmit={(e) => { e.preventDefault(); add(); }}>
      <label class="f">
        <span>Name</span>
        <input bind:this={nameBox} bind:value={name} maxlength="40" autocomplete="off" aria-invalid={problem ? 'true' : undefined} aria-describedby={problem ? 'measure-problem' : undefined} oninput={() => (problem = '')} />
      </label>
      <label class="f">
        <span>Kind</span>
        <select bind:value={kind}>
          {#each kinds as k (k.id)}<option value={k.id}>{k.name}</option>{/each}
        </select>
      </label>
      <button type="submit" class="btn primary">Add Measure</button>
    </form>
    {#if problem}<p id="measure-problem" class="note" role="alert">{problem}</p>{/if}
  </section>

  {#if view.archived.length}
    <details>
      <summary>Archived ({view.archived.length})</summary>
      <ul>
        {#each view.archived as m (m.id)}
          <li>
            <span class="nm" title={m.name}>{m.name}</span><span class="kind">{kindName(m.kind)}</span>
            <button type="button" class="btn ghost" aria-label="Restore Measure {m.name}" onclick={() => restore(m)}>Restore Measure</button>
          </li>
        {/each}
      </ul>
    </details>
  {/if}

  {#if failure}
    <p class="notice" role="alert"><Icon name="warn" size={16} />{failure}</p>
  {/if}
  </div>
  {#snippet actions()}
    <button type="button" class="btn" onclick={onclose}>Done Managing Measures</button>
  {/snippet}
</Dialog>

<style>
  .manage { display: flex; flex-direction: column; gap: 12px; }
  section { display: flex; flex-direction: column; gap: 8px; }
  h3 { margin: 0; font-size: 13px; color: var(--ink-2); }
  p { margin: 0; }
  ul { display: flex; flex-direction: column; margin: 0; padding: 0; list-style: none; }
  li { display: flex; align-items: center; gap: 16px; min-height: 40px; border-bottom: 1px solid var(--rule); }
  .nm { flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--ink); }
  .kind { flex: none; font-size: 13px; color: var(--ink-3); }
  .fine { font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  form { display: flex; flex-wrap: wrap; align-items: flex-end; gap: 16px; }
  .f { display: flex; flex-direction: column; gap: 4px; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .f:first-child { flex: 1 1 200px; min-width: 0; }
  .note { font-size: 13px; line-height: 1.4; color: var(--warn); }
  summary { cursor: pointer; font-size: 13px; color: var(--ink-2); }
  summary:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
  .notice { display: flex; align-items: flex-start; gap: 8px; padding: 8px 16px; border-radius: 9px; background: var(--warn-wash); color: var(--warn); }
  .notice :global(svg) { flex: none; margin-top: 4px; }
  input[aria-invalid='true'] { box-shadow: inset 0 0 0 1px var(--warn); }
</style>
