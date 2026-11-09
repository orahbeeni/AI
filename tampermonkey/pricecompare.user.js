// ==UserScript==
// @name         ZA Price Compare
// @namespace    omar.pricecompare
// @version      0.1.0
// @description  On any .co.za site, Takealot, Temu or Shein product page: search the other South African shops for the same / a comparable product, see prices side by side, and open any match in a new tab.
// @match        *://*.co.za/*
// @match        *://*.takealot.com/*
// @match        *://*.temu.com/*
// @match        *://*.shein.com/*
// @run-at       document-idle
// @noframes
// @grant        GM_xmlhttpRequest
// @grant        GM_openInTab
// @grant        GM_getValue
// @grant        GM_setValue
// @grant        GM_deleteValue
// @grant        GM_addValueChangeListener
// @grant        GM_removeValueChangeListener
// @grant        GM_registerMenuCommand
// @connect      *
// ==/UserScript==

(function () {
  'use strict';

  /* ======================================================================
   * 1. Pure helpers (prices, tokens, scoring). No DOM globals needed so they
   *    can be unit-tested in Node.
   * ==================================================================== */

  const PRICE_RE = /(?<![A-Za-z])(?:R|ZAR)\s?(\d{1,3}(?:[  ,]\d{3})+(?:[.,]\d{1,2})?|\d+(?:[.,]\d{1,2})?)(?!\d)/i;

  function toNum(s) {
    s = String(s).replace(/[\s ]/g, '');
    const lc = s.lastIndexOf(','), ld = s.lastIndexOf('.');
    if (lc > -1 && ld > -1) {
      s = lc > ld ? s.replace(/\./g, '').replace(',', '.') : s.replace(/,/g, '');
    } else if (lc > -1) {
      s = /,\d{1,2}$/.test(s) ? s.replace(',', '.') : s.replace(/,/g, '');
    }
    const n = parseFloat(s);
    return isFinite(n) ? n : null;
  }

  function parsePrice(text) {
    const m = PRICE_RE.exec(String(text || ''));
    return m ? toNum(m[1]) : null;
  }

  function fmtPrice(n) {
    if (n == null) return '—';
    const hasCents = Math.abs(n - Math.round(n)) > 0.004;
    const [i, d] = n.toFixed(hasCents ? 2 : 0).split('.');
    return 'R ' + i.replace(/\B(?=(\d{3})+(?!\d))/g, ' ') + (d ? '.' + d : '');
  }

  const STOP = new Set(['the', 'and', 'for', 'with', 'of', 'in', 'a', 'an', 'to', 'new', 'original', 'genuine',
    'official', 'free', 'delivery', 'pack', 'set', 'by', 'from', 'on', 'or', 'your', 'you', 'our', 'buy', 'online']);
  const COLORS = new Set(['black', 'white', 'grey', 'gray', 'silver', 'gold', 'blue', 'red', 'green', 'pink',
    'purple', 'orange', 'yellow', 'brown', 'beige', 'navy', 'rose', 'titanium', 'graphite', 'midnight', 'starlight']);

  function normTitle(t) {
    return String(t || '').toLowerCase()
      .replace(/["”″]/g, 'inch ')
      .replace(/([a-z0-9])-(?=[a-z0-9])/g, '$1')
      .replace(/(\d)\s+(gb|tb|mb|mah|mm|cm|inch|kg|ml|hz|w|l|g)\b/g, '$1$2')
      .replace(/[^a-z0-9.]+/g, ' ')
      .replace(/(^|\s)\.+|\.+(\s|$)/g, ' ')
      .replace(/(^|\s)([a-z]{3,})(\d)(?=\s|$)/g, '$1$2 $3')
      .trim();
  }

  function tokenWeight(t) {
    if (STOP.has(t)) return 0;
    if (/[a-z]/.test(t) && /\d/.test(t) && t.length >= 4 && !/^\d+(gb|tb|mb|mah|mm|cm|inch|kg|ml|hz|w|l|g)$/.test(t)) return 3; // model-ish
    if (/^\d+(\.\d+)?(gb|tb|mb|mah|mm|cm|inch|kg|ml|hz|w|l|g)?$/.test(t)) return 2;
    if (COLORS.has(t)) return 0.5;
    return 1;
  }

  function tokenMap(title) {
    const m = new Map();
    for (const t of normTitle(title).split(' ')) {
      if (!t) continue;
      const w = tokenWeight(t);
      if (w > 0) m.set(t, w);
    }
    return m;
  }

  const alnum = s => String(s || '').toLowerCase().replace(/[^a-z0-9]/g, '');

  /** 0..1 – how likely candidate is the same product as source. */
  function matchScore(source, candTitle) {
    const a = tokenMap(source.name), b = tokenMap(candTitle);
    let wa = 0, wb = 0, common = 0;
    for (const w of a.values()) wa += w;
    for (const w of b.values()) wb += w;
    for (const [t, w] of a) if (b.has(t)) common += w;
    if (!wa || !wb) return 0;
    const cover = common / wa;
    const dice = (2 * common) / (wa + wb);
    let score = 0.6 * cover + 0.4 * dice;
    const cn = alnum(candTitle);
    for (const key of [source.mpn, source.model]) {
      const k = alnum(key);
      if (k.length >= 5 && /\d/.test(k) && /[a-z]/.test(k) && cn.includes(k)) { score = Math.max(score, 0.92); }
    }
    return Math.min(1, score);
  }

  const matchLabel = s => (s >= 0.85 ? 'same' : s >= 0.45 ? 'similar' : 'loose');

  const MODEL_RE = /\b(?=[A-Z0-9-]*\d)(?=[A-Z0-9-]*[A-Z])[A-Z0-9][A-Z0-9-]{3,}\b/g;
  function findModel(title) {
    const found = String(title || '').match(MODEL_RE) || [];
    return found.find(t => !/^\d+(GB|TB|MB|MM|CM|KG|ML|W|V|MAH|HZ|INCH|L|G|K)$/i.test(t) && !/^\d/.test(t)) ||
      found.find(t => !/^\d+(GB|TB|MB|MM|CM|KG|ML|W|V|MAH|HZ|INCH|L|G|K)$/i.test(t)) || '';
  }

  function cleanTitle(raw, host) {
    let t = String(raw || '').replace(/\s+/g, ' ').trim();
    const brand = (host || '').replace(/^www\./, '').split('.')[0];
    // drop a trailing "| Site name" / "– Site.co.za" segment
    t = t.replace(/\s*[|–—]\s*([^|–—]*)$/, (m, seg) =>
      (brand && seg.toLowerCase().replace(/[^a-z]/g, '').includes(brand.toLowerCase())) || /\.(com|co\.za)\b/i.test(seg) ? '' : m);
    t = t.replace(/^buy\s+/i, '').replace(/\s+online( in south africa)?$/i, '').replace(/\s+at best price.*$/i, '');
    return t.trim();
  }

  function buildQueries(p) {
    const clean = String(p.name || '').replace(/[^\p{L}\p{N}\s.+/-]/gu, ' ').replace(/\s+/g, ' ').trim();
    const words = clean.split(' ');
    const title = words.slice(0, 9).join(' ');
    const model = p.mpn || p.model || findModel(p.name);
    const out = [{ id: 'title', label: 'Title', q: title }];
    if (model) {
      if (p.brand && !alnum(model).includes(alnum(p.brand))) out.push({ id: 'brandmodel', label: 'Brand + model', q: `${p.brand} ${model}` });
      out.push({ id: 'model', label: 'Model ' + model, q: model });
    }
    if (p.gtin) out.push({ id: 'gtin', label: 'Barcode', q: String(p.gtin) });
    return out;
  }

  /* ======================================================================
   * 2. DOM extraction – works on a live document or a DOMParser document
   * ==================================================================== */

  const BAD_CTX = 'header, nav, footer, [role=navigation], script, style, noscript, template, select, option';
  const OLD_PRICE_CLS = /(^|[-_\s])(old|was|strike|strikethrough|original|orig|before|rrp|list-?price|line-?through|crossed|compare|prev|previous)([-_\s]|$)/i;
  const NOT_PRICE_TXT = /\b(save|saving|you save|off|instal|per month|p\/m|\/mo|monthly|deposit|min(imum)? order|spend|over R)\b/i;

  const imgSrc = img => {
    if (!img) return '';
    const cands = [img.getAttribute('data-src'), img.getAttribute('data-lazy-src'), img.getAttribute('data-original'),
      img.getAttribute('src'), (img.getAttribute('srcset') || img.getAttribute('data-srcset') || '').split(',')[0].trim().split(/\s+/)[0]];
    return cands.find(s => s && !/^data:/.test(s) && !/\.svg(\?|$)/i.test(s) && !/sprite|placeholder|blank|pixel|loader|spinner/i.test(s)) || '';
  };

  function isStruck(el) {
    if (el.closest('del, s, strike')) return true;
    let n = el;
    for (let i = 0; i < 3 && n && n.nodeType === 1; i++, n = n.parentElement) {
      const cls = (n.getAttribute('class') || '') + ' ' + (n.getAttribute('id') || '');
      if (OLD_PRICE_CLS.test(cls)) return true;
      const st = n.getAttribute('style') || '';
      if (/line-through/.test(st)) return true;
    }
    return false;
  }

  function findPriceElements(root) {
    const out = [];
    const els = root.querySelectorAll('*');
    for (const el of els) {
      if (el.childElementCount > 10) continue;
      const txt = el.textContent;
      if (!txt || txt.length > 36 || !PRICE_RE.test(txt)) continue;
      let childMatches = false;
      for (const c of el.children) { if (c.textContent.length <= 36 && PRICE_RE.test(c.textContent)) { childMatches = true; break; } }
      if (childMatches) continue;
      if (el.closest(BAD_CTX)) continue;
      if (NOT_PRICE_TXT.test(txt)) continue;
      const par = el.parentElement;
      if (par && par.textContent.length < 40 && NOT_PRICE_TXT.test(par.textContent)) continue;
      out.push({ el, value: parsePrice(txt), struck: isStruck(el) });
    }
    return out;
  }

  const hrefOf = (a, base) => {
    const h = a.getAttribute('href');
    if (!h || /^(#|javascript:|mailto:|tel:)/i.test(h)) return '';
    try { return new URL(h, base).href; } catch (e) { return ''; }
  };
  const BAD_LINK = /\/(cart|basket|wishlist|login|signin|account|compare|checkout)(\/|\?|$)|[?&](add|wishlist)/i;

  function cardFrom(node, base) {
    const anchors = [...(node.matches('a[href]') ? [node] : []), ...node.querySelectorAll('a[href]')].filter(a => hrefOf(a, base) && !BAD_LINK.test(hrefOf(a, base)));
    if (!anchors.length) return null;
    const heading = node.querySelector('h1, h2, h3, h4, h5, [class*="title" i], [class*="name" i], [data-testid*="title" i]');
    const img = [...node.querySelectorAll('img')].find(i => imgSrc(i));
    const strip = s => String(s || '').replace(PRICE_RE, ' ').replace(/\s+/g, ' ').trim();
    let title = strip(heading && heading.textContent);
    if (title.length < 8 || title.length > 220) {
      title = '';
      const cands = [
        ...anchors.map(a => a.getAttribute('title')),
        img && img.getAttribute('alt'),
        ...anchors.map(a => strip(a.textContent)),
      ].map(s => strip(s)).filter(s => s.length >= 8 && s.length <= 220);
      title = cands[0] || '';
    }
    if (!title) return null;
    // pick the anchor most likely to be the product link
    const key = title.slice(0, 20).toLowerCase();
    const anchor = anchors.find(a => heading && a.contains(heading)) ||
      anchors.find(a => strip(a.textContent).toLowerCase().includes(key)) ||
      anchors.find(a => a.querySelector('img')) || anchors[0];
    return { title, url: hrefOf(anchor, base), image: new URL(imgSrc(img) || 'about:blank', base).href.replace(/^about:blank$/, '') };
  }

  /** Product cards from JSON-LD (only if they carry prices). */
  function fromJsonLd(doc, base) {
    const items = [];
    const walk = o => {
      if (!o) return;
      if (Array.isArray(o)) return o.forEach(walk);
      if (typeof o !== 'object') return;
      const type = [].concat(o['@type'] || []).join(' ');
      if (/Product/.test(type) && o.name) {
        const offer = [].concat(o.offers || [])[0] || {};
        const price = toNum(offer.price != null ? offer.price : offer.lowPrice);
        const url = o.url || offer.url;
        if (price != null && url) {
          let img = [].concat(o.image || [])[0];
          if (img && typeof img === 'object') img = img.url;
          try { items.push({ title: String(o.name), url: new URL(url, base).href, price, image: img ? new URL(img, base).href : '' }); } catch (e) { /* ignore */ }
        }
      }
      if (o.itemListElement) walk(o.itemListElement);
      if (o.item && typeof o.item === 'object') walk(o.item);
      if (o['@graph']) walk(o['@graph']);
    };
    doc.querySelectorAll('script[type="application/ld+json"]').forEach(s => {
      try { walk(JSON.parse(s.textContent)); } catch (e) { /* ignore bad json */ }
    });
    return items;
  }

  /** Generic "find product cards on a search results page". */
  function extractCards(doc, base, limit = 14) {
    const ld = fromJsonLd(doc, base);
    if (ld.length >= 3) return dedupe(ld).slice(0, limit);

    const root = doc.body || doc.documentElement;
    const prices = findPriceElements(root).filter(p => !p.struck && p.value != null);
    const cards = [];
    const seenNodes = new Set();
    for (const p of prices) {
      let cardNode = null;
      const wrapA = p.el.closest('a[href]');
      if (wrapA && prices.filter(q => wrapA.contains(q.el)).length === 1) cardNode = wrapA; // whole card is one link
      if (!cardNode) {
        let node = p.el, firstLinked = null;
        while (node.parentElement && node !== root) {
          const par = node.parentElement;
          const hasLink = node.querySelector('a[href]');
          if (hasLink && !firstLinked) firstLinked = node;
          let sibsWithPrice = 0;
          for (const sib of par.children) {
            if (sib !== node && sib.tagName === node.tagName && sib.textContent.length < 3000 && PRICE_RE.test(sib.textContent)) sibsWithPrice++;
          }
          if (hasLink && sibsWithPrice >= 1) { cardNode = node; break; }
          node = par;
          if (node.textContent.length > 4000) break;
        }
        cardNode = cardNode || firstLinked;
      }
      if (!cardNode || seenNodes.has(cardNode)) continue;
      seenNodes.add(cardNode);
      const c = cardFrom(cardNode, base);
      if (c) cards.push(Object.assign({}, c, { price: p.value }));
    }
    return dedupe(cards).slice(0, limit);
  }

  function dedupe(items) {
    const seen = new Set(), out = [];
    for (const it of items) {
      let k;
      try { const u = new URL(it.url); k = u.origin + u.pathname; } catch (e) { k = it.url; }
      if (seen.has(k)) continue;
      seen.add(k); out.push(it);
    }
    return out;
  }

  /** Detect the product the user is looking at. */
  function detectProduct(doc, loc) {
    const href = loc.href, host = loc.hostname;
    const meta = n => { const m = doc.querySelector(`meta[property="${n}"], meta[name="${n}"], meta[itemprop="${n}"]`); return m ? m.getAttribute('content') : ''; };
    let prod = null;
    const find = o => {
      if (!o || prod) return;
      if (Array.isArray(o)) return o.forEach(find);
      if (typeof o !== 'object') return;
      if (/Product/.test([].concat(o['@type'] || []).join(' ')) && o.name) { prod = o; return; }
      if (o['@graph']) find(o['@graph']);
      if (o.mainEntity) find(o.mainEntity);
    };
    doc.querySelectorAll('script[type="application/ld+json"]').forEach(s => { try { find(JSON.parse(s.textContent)); } catch (e) { /* ignore */ } });

    const h1 = doc.querySelector('h1');
    const looksLikeProduct = !!prod || /product/i.test(meta('og:type')) || !!meta('product:price:amount') ||
      /\/PLID\d+|-g-\d+\.html|-p-\d+|\/p\/|\/product/i.test(href);
    // Only trust a name on product pages. Site-wide og:title ("Takealot.com Online Shopping...") must never become the query,
    // and SPA shops often leave og:title stale, so prefer the on-page heading over it.
    const h1Text = h1 ? h1.textContent.trim() : '';
    const rawName = prod ? prod.name : looksLikeProduct ? (h1Text || meta('og:title') || '') : '';
    const name = cleanTitle(rawName, host);
    let price = null;
    if (prod) {
      const offer = [].concat(prod.offers || [])[0] || {};
      price = toNum(offer.price != null ? offer.price : offer.lowPrice);
    }
    if (price == null) price = toNum(meta('product:price:amount') || meta('price') || '') || null;
    if (price == null) {
      const ps = findPriceElements(doc.body || doc.documentElement).filter(p => !p.struck && p.value != null);
      let pick = ps[0];
      if (h1) pick = ps.find(p => h1.compareDocumentPosition(p.el) & 4 /* FOLLOWING */) || pick;
      price = pick ? pick.value : null;
    }
    let image = prod && prod.image ? [].concat(prod.image)[0] : meta('og:image');
    if (image && typeof image === 'object') image = image.url;
    const brand = prod && prod.brand ? (typeof prod.brand === 'object' ? prod.brand.name : prod.brand) : '';
    const gtin = prod ? (prod.gtin13 || prod.gtin14 || prod.gtin12 || prod.gtin8 || prod.gtin || '') : '';
    const mpn = prod ? (prod.mpn || '') : '';
    return {
      isProduct: looksLikeProduct && !!name, name, price, image: image ? new URL(image, href).href : '',
      brand: String(brand || ''), gtin: String(gtin || ''), mpn: String(mpn || ''), model: findModel(name), url: href, host,
    };
  }

  /** Work out a search URL template ("{q}") from the current site's search form. */
  function detectSearchTemplate(doc, loc) {
    const forms = [...doc.querySelectorAll('form')];
    const score = f => (/search/i.test((f.getAttribute('role') || '') + f.id + f.className + f.action) ? 2 : 0) +
      (f.querySelector('input[type=search]') ? 2 : 0);
    forms.sort((a, b) => score(b) - score(a));
    for (const f of forms) {
      if ((f.method || 'get').toLowerCase() !== 'get') continue;
      const input = f.querySelector('input[type=search], input[name=q], input[name=s], input[name=text], input[name=search], input[name=query], input[name=terms], input[name=k], input[name=keyword], input[name=searchTerm]');
      if (!input || !input.name) continue;
      try {
        const u = new URL(f.getAttribute('action') || loc.pathname, loc.href);
        f.querySelectorAll('input[type=hidden][name]').forEach(h => { if (h.value) u.searchParams.set(h.name, h.value); });
        u.searchParams.set(input.name, '__Q__');
        return u.href.replace('__Q__', '{q}');
      } catch (e) { /* try next */ }
    }
    return '';
  }

  /* ======================================================================
   * 3. Stores
   * ==================================================================== */

  // mode: 'api' (Takealot JSON) | 'html' (fetch + parse; falls back to tab) | 'tab' (needs a real browser tab)
  const DEFAULT_STORES = [
    { id: 'takealot', name: 'Takealot', url: 'https://www.takealot.com/all?qsearch={q}', mode: 'api' },
    { id: 'temu', name: 'Temu', url: 'https://www.temu.com/search_result.html?search_key={q}', mode: 'tab' },
    { id: 'shein', name: 'Shein', url: 'https://za.shein.com/pdsearch/{q}/', mode: 'tab' },
    { id: 'amazon', name: 'Amazon.co.za', url: 'https://www.amazon.co.za/s?k={q}', mode: 'html' },
    { id: 'makro', name: 'Makro', url: 'https://www.makro.co.za/search?q={q}', mode: 'html' },
    { id: 'game', name: 'Game', url: 'https://www.game.co.za/game-za/en/search/?text={q}', mode: 'html' },
    { id: 'loot', name: 'Loot', url: 'https://www.loot.co.za/search?terms={q}', mode: 'html' },
    { id: 'evetech', name: 'Evetech', url: 'https://www.evetech.co.za/search/?q={q}', mode: 'html' },
    { id: 'incredible', name: 'Incredible', url: 'https://www.incredible.co.za/catalogsearch/result/?q={q}', mode: 'html' },
    { id: 'wootware', name: 'Wootware', url: 'https://www.wootware.co.za/catalogsearch/result/?q={q}', mode: 'html' },
  ];

  const searchUrl = (store, q) => store.url.replace('{q}', encodeURIComponent(q));

  function regDomain(host) {
    const p = String(host).replace(/^www\./, '').split('.');
    return /\.(co|org|net|ac|gov)\.za$/.test(host) ? p.slice(-3).join('.') : p.slice(-2).join('.');
  }

  function gmFetch(url, opts = {}) {
    return new Promise((resolve, reject) => {
      GM_xmlhttpRequest({
        method: 'GET', url, timeout: 25000, anonymous: false,
        headers: Object.assign({ Accept: opts.json ? 'application/json' : 'text/html,application/xhtml+xml', 'Accept-Language': 'en-ZA,en;q=0.9' }, opts.headers || {}),
        onload: r => resolve(r), onerror: () => reject(new Error('network error')), ontimeout: () => reject(new Error('timeout')),
      });
    });
  }

  async function searchTakealot(q) {
    const r = await gmFetch('https://api.takealot.com/rest/v-1-14-0/searches/products?rows=12&qsearch=' + encodeURIComponent(q), { json: true });
    if (r.status !== 200) throw new Error('HTTP ' + r.status);
    const data = JSON.parse(r.responseText);
    const res = (data.sections && data.sections.products && data.sections.products.results) || [];
    return res.map(x => x.product_views).filter(Boolean).map(v => {
      const core = v.core || {}, bb = v.buybox_summary || {};
      const prices = (bb.prices || []).filter(n => typeof n === 'number');
      const img = v.gallery && v.gallery.images && v.gallery.images[0];
      return {
        title: core.title, price: prices.length ? Math.min(...prices) : parsePrice(bb.pretty_price),
        url: `https://www.takealot.com/${core.slug}/PLID${core.id}`,
        image: img ? img.replace('{size}', 'list') : '',
        note: [bb.pretty_price && /from/i.test(bb.pretty_price) ? 'multiple options' : '', v.stock_availability_summary && v.stock_availability_summary.status].filter(Boolean).join(' · '),
      };
    }).filter(i => i.title && i.price != null);
  }

  async function searchHtml(store, q) {
    const url = searchUrl(store, q);
    const r = await gmFetch(url);
    const blocked = [202, 403, 429, 503].includes(r.status) || !r.responseText || r.responseText.length < 4000;
    if (blocked) { const e = new Error('blocked (HTTP ' + r.status + ')'); e.blocked = true; throw e; }
    const doc = new DOMParser().parseFromString(r.responseText, 'text/html');
    return extractCards(doc, r.finalUrl || url);
  }

  /* ---- background-tab bridge (for JS-rendered / bot-protected shops) ---- */

  function searchViaTab(store, q) {
    const id = Math.random().toString(36).slice(2, 10);
    const url = searchUrl(store, q);
    const host = regDomain(new URL(url).hostname);
    return new Promise((resolve, reject) => {
      let tab, timer, listener;
      const done = (fn, v) => {
        clearTimeout(timer);
        try { GM_removeValueChangeListener(listener); } catch (e) { /* ignore */ }
        try { tab && tab.close(); } catch (e) { /* ignore */ }
        GM_deleteValue('pc_res_' + id); GM_deleteValue('pc_pend_' + host);
        fn(v);
      };
      GM_setValue('pc_pend_' + host, { id, t: Date.now() });
      listener = GM_addValueChangeListener('pc_res_' + id, (n, o, nv) => {
        if (!nv) return;
        if (nv.error) done(reject, new Error(nv.error)); else done(resolve, nv.items || []);
      });
      timer = setTimeout(() => done(reject, new Error('timed out – site may need a captcha / login (open it once manually)')), 50000);
      tab = GM_openInTab(url + (url.includes('#') ? '' : '#pc-job=' + id), { active: false, insert: true, setParent: true });
    });
  }

  /** Runs inside the background tab opened above. */
  function runScrapeMode(jobId) {
    const t0 = Date.now();
    let best = [];
    const finish = (items, error) => { GM_setValue('pc_res_' + jobId, { items, error, t: Date.now() }); };
    const tick = () => {
      let items = [];
      try { items = extractCards(document, location.href); } catch (e) { /* page not ready */ }
      if (items.length > best.length) best = items;
      const age = Date.now() - t0;
      if (best.length >= 6 || (best.length >= 1 && age > 10000)) return finish(best);
      if (age > 38000) return finish(best, best.length ? '' : 'no products found (captcha / login wall / layout not recognised)');
      window.scrollBy(0, 600);
      setTimeout(tick, 900);
    };
    setTimeout(tick, 1500);
  }

  /* ======================================================================
   * 4. Settings
   * ==================================================================== */

  const SKEY = 'pc_settings';
  const defaults = () => ({ disabled: {}, custom: [], autoTabs: true, sort: 'match', showLoose: false });
  const loadSettings = () => Object.assign(defaults(), GM_getValue(SKEY, {}));
  const saveSettings = s => GM_setValue(SKEY, s);

  /* ======================================================================
   * 5. Entry – scrape mode or normal mode
   * ==================================================================== */

  function entry() {
    const m = /pc-job=([a-z0-9]+)/.exec(location.hash);
    let job = m && m[1];
    if (!job) {
      const pend = GM_getValue('pc_pend_' + regDomain(location.hostname));
      if (pend && Date.now() - pend.t < 60000 && !GM_getValue('pc_claimed_' + pend.id)) {
        // only adopt if this tab was opened without an opener-visible UI (best effort)
        if (document.referrer === '' && window.history.length <= 2) { job = pend.id; GM_setValue('pc_claimed_' + pend.id, 1); }
      }
    }
    if (job) return runScrapeMode(job);
    startUi();
  }

  /* ======================================================================
   * 6. UI
   * ==================================================================== */

  function startUi() {
    const product = detectProduct(document, location);
    const state = {
      product, settings: loadSettings(), queries: buildQueries(product), qIndex: 0,
      query: '', results: {}, open: false, view: 'results', runId: 0, cache: new Map(),
    };
    state.query = (state.queries[0] && state.queries[0].q) || '';
    const mySite = regDomain(location.hostname);

    const host = document.createElement('div');
    host.id = 'za-price-compare';
    host.style.cssText = 'all:initial;position:fixed;z-index:2147483647;right:0;bottom:0;';
    const root = host.attachShadow({ mode: 'open' });
    document.documentElement.appendChild(host);

    root.innerHTML = `<style>${CSS}</style>
      <button class="fab" title="Compare prices (Alt+Shift+P)">⚖<span class="dot"></span></button>
      <div class="panel" hidden></div>`;
    const fab = root.querySelector('.fab'), panel = root.querySelector('.panel');
    if (product.isProduct) fab.classList.add('hit');

    const stores = () => {
      const s = state.settings;
      return DEFAULT_STORES.concat(s.custom.map(c => Object.assign({ custom: true }, c)))
        .filter(st => !s.disabled[st.id] && regDomain(new URL(st.url.replace('{q}', 'x')).hostname) !== mySite);
    };

    const el = (tag, cls, html) => { const e = document.createElement(tag); if (cls) e.className = cls; if (html != null) e.innerHTML = html; return e; };
    const esc = s => String(s == null ? '' : s).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

    /* ---------- running searches ---------- */
    async function runStore(st, q, runId, forceTab) {
      const key = st.id + '|' + q;
      const set = patch => { if (state.runId !== runId) return; state.results[st.id] = Object.assign(state.results[st.id] || {}, patch); render(); };
      if (state.cache.has(key) && !forceTab) { set(state.cache.get(key)); return; }
      set({ status: 'loading', items: [], error: '', via: '' });
      const finish = (items, via) => {
        const scored = items.map(i => Object.assign({}, i, { score: matchScore(state.product, i.title) }));
        const patch = { status: scored.length ? 'done' : 'empty', items: scored, via };
        state.cache.set(key, patch); set(patch);
      };
      try {
        if (st.mode === 'api') return finish(await searchTakealot(q), 'api');
        if (st.mode === 'tab' || forceTab) {
          if (!forceTab && !state.settings.autoTabs) return set({ status: 'needtab' });
          set({ status: 'loading', via: 'tab' });
          return finish(await searchViaTab(st, q), 'tab');
        }
        const items = await searchHtml(st, q);
        if (!items.length) return set({ status: 'needtab', note: 'No products found in the page HTML (the site probably renders results with JavaScript).' });
        finish(items, 'html');
      } catch (e) {
        if (e.blocked || st.mode === 'html') set({ status: 'needtab', error: e.message });
        else set({ status: 'error', error: e.message });
      }
    }

    function runAll() {
      const q = state.query.trim();
      state.runId++;
      state.results = {};
      if (!q) return render();
      stores().forEach(st => { state.results[st.id] = { status: 'loading', items: [] }; });
      render();
      stores().forEach(st => runStore(st, q, state.runId, false));
    }

    /* ---------- rendering ---------- */
    function render() {
      panel.hidden = !state.open;
      if (!state.open) return;
      panel.innerHTML = '';
      const head = el('div', 'head');
      head.append(el('b', 'ttl', '⚖ Price compare'));
      const right = el('div', 'hbtns');
      const gear = el('button', 'ic', state.view === 'settings' ? '← Back' : '⚙'); gear.title = 'Settings';
      gear.onclick = () => { state.view = state.view === 'settings' ? 'results' : 'settings'; render(); };
      const x = el('button', 'ic', '✕'); x.onclick = () => { state.open = false; render(); };
      right.append(gear, x); head.append(right); panel.append(head);
      const body = el('div', 'body'); panel.append(body);
      if (state.view === 'settings') return renderSettings(body);
      renderResults(body);
    }

    function renderResults(body) {
      const p = state.product;
      const src = el('div', 'src');
      src.innerHTML = `${p.image ? `<img src="${esc(p.image)}" alt="">` : '<div class="noimg"></div>'}
        <div class="si"><div class="stitle">${esc(p.name || 'No product detected on this page')}</div>
        <div class="sprice">${p.price != null ? esc(fmtPrice(p.price)) : ''} <span class="muted">${esc(p.host.replace(/^www\./, ''))}</span></div></div>`;
      body.append(src);

      const form = el('form', 'q');
      const inp = el('input'); inp.value = state.query; inp.placeholder = 'Search all shops for…';
      const go = el('button', 'go', 'Search'); go.type = 'submit';
      form.append(inp, go);
      form.onsubmit = ev => { ev.preventDefault(); state.query = inp.value; state.qIndex = -1; runAll(); };
      body.append(form);

      if (state.queries.length > 1) {
        const chips = el('div', 'chips');
        state.queries.forEach((c, i) => {
          const b = el('button', 'chip' + (i === state.qIndex ? ' on' : ''), esc(c.label)); b.title = c.q;
          b.onclick = () => { state.qIndex = i; state.query = c.q; runAll(); };
          chips.append(b);
        });
        body.append(chips);
      }

      const tools = el('div', 'tools');
      const sortSel = el('select'); sortSel.innerHTML = '<option value="match">Best match first</option><option value="price">Cheapest first</option>';
      sortSel.value = state.settings.sort; sortSel.onchange = () => { state.settings.sort = sortSel.value; saveSettings(state.settings); render(); };
      const lab = el('label', 'chk'); const cb = el('input'); cb.type = 'checkbox'; cb.checked = state.settings.showLoose;
      cb.onchange = () => { state.settings.showLoose = cb.checked; saveSettings(state.settings); render(); };
      lab.append(cb, document.createTextNode(' show loose matches'));
      tools.append(sortSel, lab); body.append(tools);

      // best-price banner
      const all = [];
      stores().forEach(st => { const r = state.results[st.id]; if (r && r.items) r.items.forEach(i => { if (i.score >= 0.7 && i.price != null && (p.price == null || i.price >= p.price * 0.4)) all.push(Object.assign({ store: st.name }, i)); }); });
      if (all.length) {
        all.sort((a, b) => a.price - b.price);
        const b = all[0];
        const diff = p.price != null ? b.price - p.price : null;
        const ban = el('div', 'banner');
        ban.innerHTML = `Cheapest match: <a href="${esc(b.url)}" target="_blank" rel="noopener noreferrer"><b>${esc(b.store)} ${esc(fmtPrice(b.price))}</b></a>` +
          (diff != null ? ` <span class="${diff < 0 ? 'good' : 'bad'}">${diff < 0 ? '▼ ' + fmtPrice(-diff) + ' cheaper' : diff > 0 ? '▲ ' + fmtPrice(diff) + ' more' : 'same price'} than this page</span>` : '');
        body.append(ban);
      }

      if (!state.query.trim()) body.append(el('div', 'empty', 'Type a product name above and press Search.'));

      for (const st of stores()) {
        const r = state.results[st.id];
        if (!r) continue;
        const sec = el('section', 'store');
        const h = el('div', 'sh');
        h.innerHTML = `<b>${esc(st.name)}</b>${r.via === 'tab' ? '<span class="tag">browser tab</span>' : ''}`;
        const open = el('a', 'ext', 'open search ↗'); open.href = searchUrl(st, state.query); open.target = '_blank'; open.rel = 'noopener noreferrer';
        h.append(open); sec.append(h);

        if (r.status === 'loading') sec.append(el('div', 'status', `<span class="spin"></span> ${r.via === 'tab' ? 'Loading in a background tab…' : 'Searching…'}`));
        else if (r.status === 'needtab') {
          const d = el('div', 'status', esc(r.error || r.note || 'Needs a real browser to render results.') + ' ');
          const b = el('button', 'mini', 'Load via background tab'); b.onclick = () => runStore(st, state.query, state.runId, true);
          d.append(b); sec.append(d);
        } else if (r.status === 'error') sec.append(el('div', 'status bad', 'Failed: ' + esc(r.error)));
        else if (r.status === 'empty') sec.append(el('div', 'status', 'No results.'));
        else if (r.items) {
          let items = r.items.slice();
          if (state.settings.sort === 'price') items.sort((a, b) => (b.score >= 0.45) - (a.score >= 0.45) || a.price - b.price);
          else items.sort((a, b) => b.score - a.score || a.price - b.price);
          const good = items.filter(i => i.score >= 0.45);
          let shown = state.settings.showLoose ? items : (good.length ? good : items.slice(0, 2));
          const hidden = items.length - shown.length;
          shown = shown.slice(0, 6);
          if (!good.length) sec.append(el('div', 'status muted', 'No close match – showing nearest results.'));
          shown.forEach(i => sec.append(card(i)));
          if (hidden > 0 && !state.settings.showLoose) sec.append(el('div', 'status muted', `${hidden} loose match${hidden > 1 ? 'es' : ''} hidden`));
        }
        body.append(sec);
      }
    }

    function card(i) {
      const p = state.product, lbl = matchLabel(i.score);
      const a = el('a', 'card ' + lbl); a.href = i.url; a.target = '_blank'; a.rel = 'noopener noreferrer';
      const diff = p.price != null && i.price != null ? i.price - p.price : null;
      a.innerHTML = `${i.image ? `<img src="${esc(i.image)}" alt="" loading="lazy" referrerpolicy="no-referrer">` : '<div class="noimg"></div>'}
        <div class="ci"><div class="ct">${esc(i.title)}</div>
        <div class="cp"><b>${esc(fmtPrice(i.price))}</b>
        ${diff != null && Math.abs(diff) >= 1 ? `<span class="${diff < 0 ? 'good' : 'bad'}">${diff < 0 ? '▼' : '▲'} ${esc(fmtPrice(Math.abs(diff)))}</span>` : ''}
        <span class="badge ${lbl}">${lbl === 'same' ? 'Same' : lbl === 'similar' ? 'Similar' : 'Loose'} ${Math.round(i.score * 100)}%</span></div>
        ${i.note ? `<div class="muted small">${esc(i.note)}</div>` : ''}</div><span class="go2">↗</span>`;
      return a;
    }

    function renderSettings(body) {
      const s = state.settings;
      body.append(el('h4', null, 'Shops to search'));
      DEFAULT_STORES.concat(s.custom).forEach(st => {
        const row = el('label', 'row');
        const c = el('input'); c.type = 'checkbox'; c.checked = !s.disabled[st.id];
        c.onchange = () => { if (c.checked) delete s.disabled[st.id]; else s.disabled[st.id] = true; saveSettings(s); };
        row.append(c, document.createTextNode(' ' + st.name));
        if (s.custom.includes(st)) {
          const rm = el('button', 'mini', 'remove'); rm.onclick = ev => { ev.preventDefault(); s.custom = s.custom.filter(x => x !== st); saveSettings(s); render(); };
          row.append(rm);
        }
        body.append(row);
      });

      const auto = el('label', 'row');
      const ac = el('input'); ac.type = 'checkbox'; ac.checked = s.autoTabs;
      ac.onchange = () => { s.autoTabs = ac.checked; saveSettings(s); };
      auto.append(ac, document.createTextNode(' Automatically use background tabs for Temu / Shein'));
      body.append(el('h4', null, 'Browser tabs'), auto,
        el('div', 'muted small', 'Temu and Shein build their results with JavaScript and block plain requests, so the script opens the search in a background tab, reads it, and closes the tab.'));

      body.append(el('h4', null, 'Add a shop'));
      const tpl = detectSearchTemplate(document, location);
      if (tpl && !DEFAULT_STORES.concat(s.custom).some(x => regDomain(new URL(x.url.replace('{q}', 'x')).hostname) === mySite)) {
        const b = el('button', 'go', 'Add this site (' + esc(location.hostname) + ')');
        b.onclick = () => {
          s.custom.push({ id: 'c' + Date.now(), name: location.hostname.replace(/^www\./, ''), url: tpl, mode: 'html' });
          saveSettings(s); render();
        };
        body.append(b);
      }
      const f = el('form', 'add');
      f.innerHTML = '<input name="n" placeholder="Name, e.g. Hirsch\'s" required><input name="u" placeholder="Search URL with {q}, e.g. https://www.example.co.za/search?q={q}" required>';
      const ab = el('button', 'go', 'Add'); ab.type = 'submit'; f.append(ab);
      f.onsubmit = ev => {
        ev.preventDefault();
        const n = f.n.value.trim(), u = f.u.value.trim();
        if (!u.includes('{q}')) return alert('The URL must contain {q} where the search words go.');
        try { new URL(u.replace('{q}', 'x')); } catch (e) { return alert('That is not a valid URL.'); }
        s.custom.push({ id: 'c' + Date.now(), name: n, url: u, mode: 'html' }); saveSettings(s); render();
      };
      body.append(f);
    }

    /* ---------- wiring ---------- */
    function toggle(force) {
      state.open = force != null ? force : !state.open;
      render();
      if (state.open && !Object.keys(state.results).length && state.query.trim()) runAll();
    }
    // SPA shops render late and change URL without reloading, so re-detect on a few retries, on URL change and on open
    function redetect() {
      const np = detectProduct(document, location);
      if (np.name === state.product.name && np.price === state.product.price && np.url === state.product.url) return;
      const changedProduct = np.name !== state.product.name;
      state.product = np;
      fab.classList.toggle('hit', np.isProduct);
      if (changedProduct && state.qIndex !== -1) {
        state.queries = buildQueries(np); state.qIndex = 0; state.query = (state.queries[0] && state.queries[0].q) || '';
        state.results = {}; state.cache.clear();
      }
      if (state.open) { render(); if (changedProduct && state.query.trim()) runAll(); }
    }
    [1500, 4000, 9000].forEach(ms => setTimeout(redetect, ms));
    let lastHref = location.href;
    setInterval(() => { if (location.href !== lastHref) { lastHref = location.href; setTimeout(redetect, 1200); setTimeout(redetect, 4000); } }, 1000);
    fab.onclick = () => { redetect(); toggle(); };
    document.addEventListener('keydown', e => { if (e.altKey && e.shiftKey && e.code === 'KeyP') toggle(); });
    try { GM_registerMenuCommand('Compare prices', () => toggle(true)); } catch (e) { /* ignore */ }
  }

  const CSS = `
  :host{all:initial}
  *{box-sizing:border-box;font-family:system-ui,-apple-system,"Segoe UI",Roboto,sans-serif}
  .fab{position:fixed;right:16px;bottom:92px;width:46px;height:46px;border-radius:50%;border:0;cursor:pointer;
    background:#1f2937;color:#fff;font-size:22px;box-shadow:0 4px 14px rgba(0,0,0,.35);opacity:.55;transition:opacity .15s,transform .15s}
  .fab:hover,.fab.hit{opacity:1}.fab:hover{transform:scale(1.07)}
  .fab .dot{display:none;position:absolute;top:4px;right:4px;width:11px;height:11px;border-radius:50%;background:#22c55e;border:2px solid #1f2937}
  .fab.hit .dot{display:block}
  .panel{position:fixed;right:12px;bottom:12px;top:12px;width:min(440px,calc(100vw - 24px));display:flex;flex-direction:column;
    background:#fff;color:#111827;border:1px solid #d1d5db;border-radius:14px;box-shadow:0 12px 40px rgba(0,0,0,.3);overflow:hidden;font-size:13px}
  .panel[hidden]{display:none}
  .head{display:flex;align-items:center;justify-content:space-between;padding:10px 12px;background:#111827;color:#fff}
  .ttl{font-size:14px}.hbtns{display:flex;gap:6px}
  .ic{background:transparent;border:0;color:#fff;cursor:pointer;font-size:15px;padding:2px 8px;border-radius:6px}.ic:hover{background:rgba(255,255,255,.15)}
  .body{overflow:auto;padding:10px 12px 16px;flex:1}
  .src{display:flex;gap:10px;padding:8px;border-radius:10px;background:#f3f4f6;margin-bottom:10px}
  .src img,.noimg{width:54px;height:54px;object-fit:contain;border-radius:8px;background:#fff;flex:none}.noimg{background:#e5e7eb}
  .stitle{font-weight:600;line-height:1.25;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}
  .sprice{margin-top:3px;font-size:15px;font-weight:700}.muted{color:#6b7280;font-weight:400;font-size:12px}.small{font-size:11px}
  form.q{display:flex;gap:6px}form.q input{flex:1;padding:7px 9px;border:1px solid #d1d5db;border-radius:8px;font-size:13px;background:#fff;color:inherit}
  .go{background:#2563eb;color:#fff;border:0;border-radius:8px;padding:7px 12px;cursor:pointer;font-size:13px}.go:hover{background:#1d4ed8}
  .chips{display:flex;flex-wrap:wrap;gap:5px;margin:7px 0 2px}
  .chip{border:1px solid #d1d5db;background:#fff;color:inherit;border-radius:99px;padding:2px 9px;font-size:11px;cursor:pointer;max-width:200px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .chip.on{background:#dbeafe;border-color:#2563eb;color:#1e3a8a}
  .tools{display:flex;justify-content:space-between;align-items:center;margin:8px 0}
  .tools select{padding:3px 6px;border:1px solid #d1d5db;border-radius:6px;background:#fff;color:inherit;font-size:12px}.chk{font-size:12px;color:#4b5563}
  .banner{background:#ecfdf5;border:1px solid #a7f3d0;border-radius:10px;padding:8px 10px;margin:6px 0 10px;line-height:1.5}
  .banner a{color:#047857}
  .store{margin-top:12px}.sh{display:flex;align-items:center;gap:8px;border-bottom:1px solid #e5e7eb;padding-bottom:4px;margin-bottom:6px}
  .sh b{font-size:13px}.ext{margin-left:auto;font-size:11px;color:#2563eb;text-decoration:none}.ext:hover{text-decoration:underline}
  .tag{font-size:10px;background:#fef3c7;color:#92400e;border-radius:4px;padding:1px 5px}
  .status{padding:5px 2px;color:#4b5563;font-size:12px}.status.bad,.bad{color:#b91c1c}.good{color:#047857;font-weight:600}
  .mini{border:1px solid #2563eb;color:#2563eb;background:#fff;border-radius:6px;padding:2px 8px;font-size:11px;cursor:pointer}.mini:hover{background:#eff6ff}
  .spin{display:inline-block;width:11px;height:11px;border:2px solid #9ca3af;border-top-color:transparent;border-radius:50%;animation:s .8s linear infinite;vertical-align:-1px}
  @keyframes s{to{transform:rotate(360deg)}}
  .card{display:flex;gap:9px;padding:7px;border-radius:10px;text-decoration:none;color:inherit;border:1px solid transparent;position:relative}
  .card:hover{background:#f3f4f6;border-color:#e5e7eb}.card img,.card .noimg{width:48px;height:48px;object-fit:contain;border-radius:6px;background:#fff;flex:none}
  .card.loose{opacity:.6}.ci{min-width:0;flex:1}
  .ct{line-height:1.25;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}
  .cp{display:flex;align-items:center;gap:7px;margin-top:3px;flex-wrap:wrap}.cp b{font-size:14px}
  .badge{font-size:10px;border-radius:4px;padding:1px 5px}.badge.same{background:#d1fae5;color:#065f46}.badge.similar{background:#fef3c7;color:#92400e}.badge.loose{background:#e5e7eb;color:#4b5563}
  .go2{color:#9ca3af;align-self:center}.card:hover .go2{color:#2563eb}
  .empty{padding:18px 4px;color:#6b7280}
  h4{margin:14px 0 6px;font-size:12px;text-transform:uppercase;letter-spacing:.04em;color:#6b7280}
  .row{display:flex;align-items:center;gap:4px;padding:3px 0}.row .mini{margin-left:auto}
  form.add{display:flex;flex-direction:column;gap:6px;margin-top:8px}form.add input{padding:7px 9px;border:1px solid #d1d5db;border-radius:8px;background:#fff;color:inherit}
  @media (prefers-color-scheme:dark){
    .panel{background:#1f2937;color:#f3f4f6;border-color:#374151}.src{background:#111827}
    form.q input,.chip,.tools select,form.add input,.mini{background:#111827;color:#f3f4f6;border-color:#374151}
    .chip.on{background:#1e3a8a;color:#dbeafe}.card:hover{background:#111827;border-color:#374151}.sh{border-color:#374151}
    .banner{background:#064e3b;border-color:#047857}.banner a{color:#6ee7b7}.good{color:#34d399}.bad,.status.bad{color:#f87171}
    .muted,.status,.chk,h4,.empty{color:#9ca3af}.src img,.card img,.noimg{background:#fff}
  }`;

  /* ---- test hook (Node) / browser start ---- */
  if (typeof module !== 'undefined' && module.exports) {
    module.exports = { parsePrice, fmtPrice, matchScore, matchLabel, extractCards, detectProduct, buildQueries, cleanTitle, findModel, detectSearchTemplate, normTitle };
  } else {
    entry();
  }
})();
