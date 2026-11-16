<script lang="ts">
  import FoodArt from './FoodArt.svelte';
  import { photoUrl } from './api';
  // The recipe's photo, or its painted plate when there is none (or the file is missing).
  let { photo, seed }: { photo: string; seed: number } = $props();
  let failed = $state(false);
  $effect(() => { photo; failed = false; });
</script>

{#if photo && !failed}
  <img src={photoUrl(photo)} alt="" draggable="false" loading="lazy" decoding="async" onerror={() => (failed = true)} />
{:else}
  <FoodArt {seed} />
{/if}

<style>
  img { display: block; width: 100%; height: 100%; object-fit: cover; }
</style>
