<script lang="ts">
  // Drawn line icons on a 20px grid; colour comes from the text colour.
  let { name, size = 20 }: { name: string; size?: number } = $props();

  const paths: Record<string, string> = {
    moon: 'M16.5 12.5A7 7 0 0 1 7.5 3.5a7 7 0 1 0 9 9z',
    book: 'M4 3.5h9.5a2 2 0 0 1 2 2V17H6a2 2 0 0 1-2-2zM4 15a2 2 0 0 1 2-2h9.5M7.5 7h5',
    globe: 'M10 3a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM3 10h14M10 3c2 2 2.8 4.5 2.8 7s-.8 5-2.8 7c-2-2-2.8-4.5-2.8-7S8 5 10 3z',
    cart: 'M2.5 3.5h2l2 9.5h8.5l1.5-7H6M8 16.5h.01M14 16.5h.01',
    jar: 'M6 3.5h8M6.5 3.5v2L5 7.5V16a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1V7.5l-1.5-2v-2M5 10.5h10',
    calendar: 'M4 5h12v11.5H4zM4 8.5h12M7 3v3.5M13 3v3.5',
    menu: 'M5 3h10v14H5zM7.5 7h5M7.5 10h5M7.5 13h3',
    home: 'M3.5 9.5 10 4l6.5 5.5M5.5 8v8.5h9V8',
    clock: 'M10 3.5a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13zM10 6.5V10l2.5 1.5',
    star: 'M10 3.2l2 4.3 4.6.5-3.4 3.1 1 4.6L10 13.4l-4.2 2.3 1-4.6L3.4 8l4.6-.5z',
    heart: 'M10 16.2S3.5 12.4 3.5 8a3.4 3.4 0 0 1 6.5-1.5A3.4 3.4 0 0 1 16.5 8c0 4.4-6.5 8.2-6.5 8.2z',
    tray: 'M3.5 11.5 5.5 4h9l2 7.5V16h-13zM3.5 11.5h4l1 2h3l1-2h4',
    trash: 'M4 5.5h12M8 5.5V3.5h4v2M5.5 5.5l.8 11h7.4l.8-11M8.5 8.5v5M11.5 8.5v5',
    grid: 'M4 4h5v5H4zM11 4h5v5h-5zM4 11h5v5H4zM11 11h5v5h-5z',
    list: 'M4 5h2M4 10h2M4 15h2M8.5 5H16M8.5 10H16M8.5 15H16',
    search: 'M8.5 3.5a5 5 0 1 0 0 10 5 5 0 0 0 0-10zM12.2 12.2 16.5 16.5',
    back: 'M12.5 4 6.5 10l6 6',
    chevron: 'M7.5 5l5 5-5 5',
    close: 'M5.5 5.5l9 9M14.5 5.5l-9 9',
    warn: 'M10 3.5 17 16H3zM10 8.5v3.5M10 14v.01',
    link: 'M8.5 11.5a3 3 0 0 0 4.2 0l2.5-2.5a3 3 0 0 0-4.2-4.2l-1 1M11.5 8.5a3 3 0 0 0-4.2 0L4.8 11a3 3 0 0 0 4.2 4.2l1-1',
    sort: 'M6 4v12M3.5 13.5 6 16l2.5-2.5M14 16V4M11.5 6.5 14 4l2.5 2.5',
    more: 'M5 10h.01M10 10h.01M15 10h.01',
    join: 'M10 16V7M6 10.5 10 6.5l4 4M4 3.5h12',
    refresh: 'M15.5 8.5A5.8 5.8 0 0 0 5 6.2L3.8 7.5M3.5 4v3.5H7M4.5 11.5A5.8 5.8 0 0 0 15 13.8l1.2-1.3M16.5 16v-3.5H13',
    gear: 'M10 7.2a2.8 2.8 0 1 0 0 5.6 2.8 2.8 0 0 0 0-5.6zM10 2.5v2M10 15.5v2M2.5 10h2M15.5 10h2M4.7 4.7l1.4 1.4M13.9 13.9l1.4 1.4M4.7 15.3l1.4-1.4M13.9 6.1l1.4-1.4',
    minus: 'M4.5 10h11',
    copy: 'M7 7h9v9H7zM4 13V4h9',
    pin: 'M10 17v-5M6 12h8l-1.5-2.5V5h1V3.5h-7V5h1v4.5z',
    check: 'M4.5 10.5l3.5 3.5 7.5-8',
    play: 'M7 4.5v11l8.5-5.5z',
    pause: 'M7.5 5v10M12.5 5v10',
    pencil: 'M12.5 4.5l3 3-8.5 8.5H4v-3zM11 6l3 3',
    plus: 'M10 4.5v11M4.5 10h11',
    photo: 'M3.5 5.5h13v10h-13zM3.5 13l4-4 3 3 2-2 4 4M13 8.5h.01',
    wand: 'M4 16 13 7M11.5 5.5l3 3M14.5 3v2.5M13.25 4.25h2.5M16.5 9.5v2M15.5 10.5h2',
    out: 'M11 4h5v5M16 4l-7 7M14 11.5V16H4V6h4.5',
    sidebar: 'M3.5 4.5h13v11h-13zM12 4.5v11',
    flame:'M10 17a5 5 0 0 0 5-5c0-3.5-3-5-3.5-8.5C9 5 8.5 7 8.5 8.5 7.5 8 7 7 7 6c-1.5 1.5-2 3.5-2 6a5 5 0 0 0 5 5z',
    target: 'M10 3.5a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13zM10 7a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM10 10h.01',
    trend: 'M3.5 16.5h13M4.5 13l3.5-4 3 2.5 4.5-6',
  };
</script>

<svg width={size} height={size} viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  <path d={paths[name] ?? ''} />
</svg>
