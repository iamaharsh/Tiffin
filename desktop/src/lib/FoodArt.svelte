<script lang="ts">
  // The painted plate shown when a recipe has no photo. Same seeded steps as the classic app's
  // food_image (app/src/ui/widgets.rs), so a recipe keeps its picture across both apps.
  let { seed }: { seed: number } = $props();

  function rng(start: number) {
    let s = start >>> 0 || 1;
    return () => {
      s ^= s << 13; s >>>= 0;
      s ^= s >>> 17;
      s ^= s << 5; s >>>= 0;
      return (s % 10000) / 10000;
    };
  }
  function hsv(h: number, sat: number, v: number) {
    h = (((h % 1) + 1) % 1) * 6;
    const i = Math.floor(h), f = h - i;
    const p = v * (1 - sat), q = v * (1 - sat * f), t = v * (1 - sat * (1 - f));
    const [r, g, b] = [[v, t, p], [q, v, p], [p, v, t], [p, q, v], [t, p, v], [v, p, q]][i] ?? [v, p, q];
    return `rgb(${(r * 255) | 0},${(g * 255) | 0},${(b * 255) | 0})`;
  }

  const art = $derived.by(() => {
    const r = rng(seed);
    const kind = Math.floor(r() * 4);
    const table = [['#b07c54', '#7a5032'], ['#5c626c', '#363a42'], ['#ece6dc', '#cec6ba'], ['#e8e8ec', '#c4c6ce']][Math.min(kind, 3)];
    const w = 100;
    const cx = 50 + (r() - 0.5) * w * 0.12, cy = 50 + (r() - 0.5) * w * 0.1;
    const pr = w * (0.36 + r() * 0.08);
    const hue = r() * 0.14 - 0.02;
    const food = hsv(hue, 0.55 + r() * 0.3, 0.75 + r() * 0.2);
    const foodDark = hsv(hue, 0.7, 0.55);
    const fr = pr * (0.62 + r() * 0.1);
    const bits = Array.from({ length: 6 }, () => {
      const a = r() * Math.PI * 2, d = r() * fr * 0.7;
      return { x: cx + d * Math.cos(a), y: cy + d * Math.sin(a), r: fr * (0.08 + r() * 0.08) };
    });
    const bitColor = hsv(hue + 0.03, 0.4, 0.98);
    const gh = r() > 0.5 ? 0.33 : 0.95;
    const garnish = Array.from({ length: 3 + Math.floor(r() * 4) }, () => {
      const a = r() * Math.PI * 2, d = fr * (0.2 + r() * 0.5);
      return { x: cx + d * Math.cos(a), y: cy + d * Math.sin(a), c: hsv(gh, 0.75, 0.6 + r() * 0.2) };
    });
    const bowl = r() > 0.4 ? hsv(r(), 0.5, 0.8) : null;
    return { kind, table, cx, cy, pr, food, foodDark, fr, bits, bitColor, garnish, bowl };
  });
  const gid = $derived(`t${seed}`);
</script>

<svg viewBox="0 0 100 100" preserveAspectRatio="xMidYMid slice" aria-hidden="true">
  <defs>
    <linearGradient id={gid} x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color={art.table[0]} />
      <stop offset="1" stop-color={art.table[1]} />
    </linearGradient>
  </defs>
  <rect width="100" height="100" fill="url(#{gid})" />
  {#if art.kind === 0}
    {#each [0, 1, 2, 3, 4] as i}
      <line x1="0" y1={12 + i * 20} x2="100" y2={14 + i * 20} stroke="rgba(0,0,0,.11)" stroke-width=".7" />
    {/each}
  {/if}
  <circle cx={art.cx + 2} cy={art.cy + 3.5} r={art.pr * 1.02} fill="rgba(0,0,0,.22)" />
  <circle cx={art.cx} cy={art.cy} r={art.pr} fill="#fafafa" />
  <circle cx={art.cx} cy={art.cy} r={art.pr * 0.8} fill="none" stroke="#e4e4e4" stroke-width=".7" />
  <circle cx={art.cx} cy={art.cy} r={art.fr} fill={art.foodDark} />
  <circle cx={art.cx - art.fr * 0.06} cy={art.cy - art.fr * 0.07} r={art.fr * 0.9} fill={art.food} />
  {#each art.bits as b}<circle cx={b.x} cy={b.y} r={b.r} fill={art.bitColor} opacity=".9" />{/each}
  {#each art.garnish as g}<circle cx={g.x} cy={g.y} r={art.fr * 0.1} fill={g.c} />{/each}
  {#if art.bowl}
    <circle cx="92" cy="10" r="12" fill="#f4f4f4" />
    <circle cx="92" cy="10" r="8" fill={art.bowl} />
  {/if}
</svg>

<style>
  svg { display: block; width: 100%; height: 100%; }
</style>
