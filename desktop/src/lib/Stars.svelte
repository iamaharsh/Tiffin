<script lang="ts">
  import Icon from './Icon.svelte';
  // Read-only when `onrate` is missing. Editing: hover previews, click sets, clicking the
  // current rating again clears it.
  let { rating, size = 14, onrate }: { rating: number; size?: number; onrate?: (n: number) => void } = $props();
  let hover = $state(0);
  let popped = $state(0);
  const shown = $derived(hover || rating);

  function rate(n: number) {
    onrate?.(n === rating ? 0 : n);
    popped = n;
  }
</script>

{#if onrate}
  <div class="stars edit" role="radiogroup" aria-label="Rating" tabindex="-1" onpointerleave={() => (hover = 0)}>
    {#each [1, 2, 3, 4, 5] as n}
      <button
        role="radio"
        aria-checked={rating === n}
        aria-label="{n} star{n > 1 ? 's' : ''}"
        class:on={n <= shown}
        class:pop={popped >= n}
        style:--i={n}
        onpointerenter={() => (hover = n)}
        onanimationend={() => { if (n === popped) popped = 0; }}
        onclick={() => rate(n)}
      >
        <Icon name="star" {size} />
      </button>
    {/each}
  </div>
{:else}
  <div class="stars" role="img" aria-label="{rating} of 5 stars">
    {#each [1, 2, 3, 4, 5] as n}
      <span class:on={n <= rating}><Icon name="star" {size} /></span>
    {/each}
  </div>
{/if}

<style>
  .stars { display: inline-flex; gap: 1px; color: var(--star-off); }
  .stars :global(svg) { fill: currentColor; stroke-width: 1; }
  .on { color: var(--star); }
  .edit button {
    display: grid;
    place-items: center;
    padding: 3px;
    border-radius: 6px;
    color: var(--star-off);
    transition: color 120ms ease-out, transform 120ms ease-out;
  }
  .edit button.on { color: var(--star); }
  .edit button:active { transform: scale(0.88); }
  /* a quick ripple left to right after a click */
  .edit button.pop { animation: pop 320ms var(--ease-spring) calc(var(--i) * 30ms) both; }
  @keyframes pop { 40% { transform: scale(1.28); } }
</style>
