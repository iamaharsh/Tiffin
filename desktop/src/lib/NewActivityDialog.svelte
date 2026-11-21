<script lang="ts">
  import Dialog from './Dialog.svelte';
  import Segmented from './Segmented.svelte';
  import Icon from './Icon.svelte';
  import * as api from './api';
  import type { CustomActivity } from './api';

  // New Activity / Edit Activity: your own activity, kept in your profile and never in the cited table. The crate checks the name and the number.
  let { activity = null, onclose, onsaved, ondeleted }: {
    /** the activity being edited; null makes a new one */
    activity?: CustomActivity | null;
    /** Back to Search and Esc: nothing is kept, nothing asked */
    onclose: () => void;
    onsaved: (uid: string) => void;
    ondeleted: () => void;
  } = $props();

  type Kind = 'MET' | 'kcal per hour';
  const kinds: Kind[] = ['MET', 'kcal per hour'];

  // svelte-ignore state_referenced_locally
  let name = $state(activity?.name ?? '');
  // svelte-ignore state_referenced_locally
  let kind: Kind = $state(activity && 'KcalPerHour' in activity.basis ? 'kcal per hour' : 'MET');
  // svelte-ignore state_referenced_locally
  let value: number | null = $state(activity ? ('Met' in activity.basis ? activity.basis.Met : activity.basis.KcalPerHour) : null);
  let failure = $state('');
  let confirming = $state(false);
  let nameBox: HTMLInputElement | undefined = $state();
  let busy = false;

  $effect(() => { nameBox?.focus(); });

  async function save() {
    if (busy) return;
    busy = true;
    failure = '';
    try {
      // an empty box goes as 0, which the crate refuses with its own sentence
      const v = value ?? 0;
      const uid = await api.saveActivity({ uid: activity?.uid ?? '', name, basis: kind === 'MET' ? { Met: v } : { KcalPerHour: v } });
      onsaved(uid);
    } catch (e) {
      failure = api.reason(e) + '.';
    } finally {
      busy = false;
    }
  }
  async function remove() {
    if (busy || !activity) return;
    busy = true;
    failure = '';
    try {
      await api.deleteActivity(activity.uid);
      confirming = false;
      ondeleted();
    } catch (e) {
      confirming = false;
      failure = api.reason(e) + '.';
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title={activity ? 'Edit Activity' : 'New Activity'} {onclose}>
  <form id="new-activity-form" onsubmit={(e) => { e.preventDefault(); save(); }}>
    <label class="f">
      <span>Name</span>
      <input bind:this={nameBox} bind:value={name} maxlength="60" autocomplete="off" />
    </label>
    <div class="f">
      <span>Activity value</span>
      <Segmented label="Activity value" options={kinds} value={kind} onchange={(v) => (kind = v)}>
        {#snippet item(o)}{o}{/snippet}
      </Segmented>
    </div>
    <label class="f">
      <span>{kind === 'MET' ? 'MET value' : 'Calories per hour'}</span>
      <input bind:value type="number" min="0" step="any" inputmode="decimal" autocomplete="off" />
    </label>
    <p class="note">MET is how many times your resting burn. Calories per hour are counted as net, above resting.</p>
  </form>
  {#if failure}
    <p class="notice" role="alert"><Icon name="warn" size={16} />{failure}</p>
  {/if}
  {#snippet actions()}
    {#if activity}<button type="button" class="btn ghost del" onclick={() => (confirming = true)}>Delete Activity</button>{/if}
    <button type="button" class="btn" onclick={onclose}>Back to Search</button>
    <button type="submit" form="new-activity-form" class="btn primary">Save Activity</button>
  {/snippet}
</Dialog>

{#if confirming && activity}
  <Dialog title="Delete {activity.name}?" onclose={() => (confirming = false)}>
    <p class="ask">Exercise you already logged keeps its values.</p>
    {#snippet actions()}
      <button type="button" class="btn" onclick={() => (confirming = false)}>Keep Activity</button>
      <button type="button" class="btn danger" onclick={remove}>Delete Activity</button>
    {/snippet}
  </Dialog>
{/if}

<style>
  form { display: flex; flex-direction: column; gap: 16px; }
  .f { display: flex; flex-direction: column; align-items: flex-start; gap: 4px; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  label.f input { align-self: stretch; }
  form .f input { font-size: 14.5px; }
  form p.note { margin: 0; font-size: 13px; line-height: 1.4; color: var(--ink-2); }
  .ask { margin: 0; font-size: 14.5px; line-height: 1.45; color: var(--ink-2); }
  .btn.ghost.del { margin-right: auto; color: var(--heart); }
  .btn.ghost.del:hover { background: color-mix(in srgb, var(--heart) 10%, transparent); text-decoration: none; }
  .notice { display: flex; align-items: flex-start; gap: 8px; margin: 0; padding: 8px 16px; border-radius: 9px; background: var(--warn-wash); color: var(--warn); }
  .notice :global(svg) { flex: none; margin-top: 4px; }
</style>
