'use strict';
// Format / minify helpers. Pure string -> string (terser is lazy-loaded for JS).

let terserP;
function loadTerser() {
  // Self-contained bundle (vendor/terser.min.js) that sets globalThis.Terser; run with `define` hidden
  // so Monaco's AMD loader doesn't intercept it.
  return terserP ||= fetch('vendor/terser.min.js').then(r => r.text()).then(src => {
    new Function('define', src).call(globalThis, undefined);
    return globalThis.Terser;
  });
}

async function minJS(code) {
  const T = await loadTerser();
  const r = await T.minify(code, { compress: true, mangle: true });
  if (r.error) throw r.error;
  return r.code;
}

// Swap string literals / comments for placeholders so regex passes can't touch them.
function hide(code, re) {
  const stash = [];
  const out = code.replace(re, m => (stash.push(m), `\u0000${stash.length - 1}\u0000`));
  return { out, show: s => s.replace(/\u0000(\d+)\u0000/g, (_, i) => stash[i]) };
}

function minCSS(css) {
  const h = hide(css.replace(/\/\*[\s\S]*?\*\//g, ''), /"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|url\([^)]*\)/g);
  const out = h.out
    .replace(/\s+/g, ' ')
    .replace(/\s*([{};,>~])\s*/g, '$1')
    .replace(/\s*:\s*/g, ':')
    .replace(/;}/g, '}')
    .trim();
  return h.show(out);
}

async function minHTML(html) {
  const blocks = [];
  const raw = [];
  let out = html.replace(/<(pre|textarea|script|style)\b[^>]*>[\s\S]*?<\/\1>/gi, m => (raw.push(m), `<ph-${raw.length - 1}/>`));
  for (let i = 0; i < raw.length; i++) {
    const m = raw[i].match(/^<(script|style)\b([^>]*)>([\s\S]*?)<\/\1>$/i);
    if (!m) { blocks.push(raw[i]); continue; }
    const [, tag, attrs, body] = m;
    let min = body;
    try {
      if (tag.toLowerCase() === 'style') min = minCSS(body);
      else if (!/type\s*=\s*["']?(?!module|text\/javascript|application\/javascript)/i.test(attrs)) min = await minJS(body);
    } catch { /* leave untouched */ }
    blocks.push(`<${tag}${attrs}>${min}</${tag}>`);
  }
  out = out
    .replace(/<!--(?!\[if|<!)[\s\S]*?-->/g, '')
    .replace(/\s+/g, ' ')
    .replace(/>\s+</g, '><')
    .trim();
  return out.replace(/<ph-(\d+)\/>/g, (_, i) => blocks[i]);
}

function minXML(xml) {
  return xml.replace(/<!--[\s\S]*?-->/g, '').replace(/>\s+</g, '><').replace(/\s+/g, ' ').trim();
}

function formatXML(xml) {
  const tokens = minXML(xml).match(/<!\[CDATA\[[\s\S]*?\]\]>|<[^>]+>|[^<]+/g) || [];
  let depth = 0, out = '';
  const pad = () => '  '.repeat(Math.max(depth, 0));
  for (let i = 0; i < tokens.length; i++) {
    const t = tokens[i];
    if (t.startsWith('</')) { depth--; out += pad() + t + '\n'; }
    else if (t[0] === '<' && !/^<[?!]/.test(t) && !t.endsWith('/>')) {
      // <a>text</a> stays on one line
      if (tokens[i + 1] && tokens[i + 1][0] !== '<' && tokens[i + 2] === '</' + t.slice(1).split(/[\s>]/)[0] + '>') {
        out += pad() + t + tokens[i + 1] + tokens[i + 2] + '\n'; i += 2;
      } else { out += pad() + t + '\n'; depth++; }
    } else out += pad() + t + '\n';
  }
  return out.trimEnd();
}

const MINIFIERS = {
  json: s => JSON.stringify(JSON.parse(s)),
  javascript: minJS, css: minCSS, scss: minCSS, less: minCSS,
  html: minHTML, xml: minXML,
};
const FORMATTERS = { xml: async s => formatXML(s) };  // others use Monaco's built-in providers

// ---- line operations: array of lines -> array of lines ----
const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });
const byLen = (a, b) => a.length - b.length;
const unique = (lines, key = x => x) => { const seen = new Set(); return lines.filter(l => { const k = key(l); return seen.has(k) ? false : (seen.add(k), true); }); };
const LINE_OPS = {
  'Sort Lines (A→Z)': l => [...l].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0)),
  'Sort Lines (Z→A)': l => [...l].sort((a, b) => (a < b ? 1 : a > b ? -1 : 0)),
  'Sort Lines (Case-Insensitive)': l => [...l].sort((a, b) => a.toLowerCase().localeCompare(b.toLowerCase())),
  'Sort Lines (Natural / Numeric)': l => [...l].sort(collator.compare),
  'Sort Lines (By Length)': l => [...l].sort(byLen),
  'Reverse Lines': l => [...l].reverse(),
  'Shuffle Lines': l => { const a = [...l]; for (let i = a.length - 1; i > 0; i--) { const j = Math.floor(Math.random() * (i + 1)); [a[i], a[j]] = [a[j], a[i]]; } return a; },
  'Unique Lines (Keep First)': l => unique(l),
  'Unique Lines (Case-Insensitive)': l => unique(l, x => x.toLowerCase()),
  'Unique Lines (Ignore Surrounding Whitespace)': l => unique(l, x => x.trim()),
  'Sort + Unique Lines': l => unique([...l].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0))),
  'Keep Only Duplicate Lines': l => { const c = new Map(); l.forEach(x => c.set(x, (c.get(x) || 0) + 1)); return unique(l.filter(x => c.get(x) > 1)); },
  'Remove Empty Lines': l => l.filter(x => x.trim() !== ''),
  'Trim Trailing Whitespace': l => l.map(x => x.replace(/[ \t]+$/, '')),
};

window.Transform = { MINIFIERS, FORMATTERS, LINE_OPS };
