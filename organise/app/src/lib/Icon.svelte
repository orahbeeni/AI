<script lang="ts">
  // Simple stroke icons on a 24×24 grid.
  let { name, size = 16, class: cls = "" }: { name: string; size?: number; class?: string } = $props();

  const P: Record<string, string> = {
    check: "M5 12.5l4.5 4.5L19 7.5",
    list: "M9 6h11M9 12h11M9 18h11M4.5 6h.01M4.5 12h.01M4.5 18h.01",
    grid: "M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z",
    search: "M10.5 17a6.5 6.5 0 1 0 0-13 6.5 6.5 0 0 0 0 13zM15.5 15.5L20 20",
    plus: "M12 5v14M5 12h14",
    more: "M5 12h.01M12 12h.01M19 12h.01",
    grip: "M9 6h.01M15 6h.01M9 12h.01M15 12h.01M9 18h.01M15 18h.01",
    "chevron-right": "M9 6l6 6-6 6",
    "chevron-down": "M6 9l6 6 6-6",
    "chevron-left": "M15 6l-6 6 6 6",
    file: "M14 3H7a1 1 0 0 0-1 1v16a1 1 0 0 0 1 1h10a1 1 0 0 0 1-1V7zM14 3v4h4",
    folder: "M3 7a1 1 0 0 1 1-1h5l2 2h9a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z",
    link: "M10 14a4 4 0 0 0 5.66 0l3-3a4 4 0 0 0-5.66-5.66l-1 1M14 10a4 4 0 0 0-5.66 0l-3 3a4 4 0 0 0 5.66 5.66l1-1",
    globe: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM3 12h18M12 3c2.5 2.6 3.8 5.6 3.8 9s-1.3 6.4-3.8 9M12 3C9.5 5.6 8.2 8.6 8.2 12s1.3 6.4 3.8 9",
    mail: "M3 6h18v12H3zM3 7l9 6 9-6",
    warning: "M12 4L2.5 20h19zM12 10v4.5M12 17.2v.3",
    settings: "M4 7h9M17 7h3M4 17h3M11 17h9M15 9a2 2 0 1 0 0-4 2 2 0 0 0 0 4zM9 19a2 2 0 1 0 0-4 2 2 0 0 0 0 4z",
    close: "M6 6l12 12M18 6L6 18",
    info: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM12 11v6M12 7.5v.5",
    refresh: "M20 11a8 8 0 1 0-2.3 5.7M20 5v6h-6",
    trash: "M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13",
    download: "M12 4v11M7 10l5 5 5-5M5 20h14",
    upload: "M12 20V9M7 14l5-5 5 5M5 4h14",
    clock: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM12 7v5l3 2",
    copy: "M9 9h11v11H9zM5 15V4h11",
    edit: "M4 20h4L19 9l-4-4L4 16z",
    "arrow-left": "M19 12H5M11 6l-6 6 6 6",
    "arrow-right": "M5 12h14M13 6l6 6-6 6",
    "arrow-up": "M12 19V5M6 11l6-6 6 6",
    "arrow-down": "M12 5v14M6 13l6 6 6-6",
    external: "M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5",
    dot: "M12 12h.01",
    palette: "M12 21a9 9 0 1 1 9-9c0 2-1.5 3-3.5 3H16a2 2 0 0 0-1.5 3.3c.4.5.5 1.2.1 1.8-.6.6-1.6.9-2.6.9zM7.5 11.5h.01M10.5 7.5h.01M15.5 8h.01",
    tag: "M3 12V4a1 1 0 0 1 1-1h8l9 9-9 9zM7.5 7.5h.01",
    undo: "M9 14L4 9l5-5M4 9h10a6 6 0 0 1 0 12h-3",
    bullet: "M12 12h.01",
    board: "M4 4h4v16H4zM10 4h4v10h-4zM16 4h4v13h-4z",
    expand: "M14 4h6v6M10 20H4v-6M20 4l-7 7M4 20l7-7",
    popout: "M10 5H5a1 1 0 0 0-1 1v13a1 1 0 0 0 1 1h13a1 1 0 0 0 1-1v-5M14 4h6v6M20 4l-8 8",
    dock: "M10 5H5a1 1 0 0 0-1 1v13a1 1 0 0 0 1 1h13a1 1 0 0 0 1-1v-5M20 4l-8 8M11 8v5h5",
  };
  const fat = new Set(["more", "grip", "dot", "bullet"]);
</script>

<svg
  class="icon {cls}"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width={fat.has(name) ? 3 : 1.8}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  <path d={P[name] ?? P.dot} />
</svg>

<style>
  .icon {
    flex: none;
    display: block;
  }
</style>
