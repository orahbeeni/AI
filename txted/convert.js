'use strict';
// JSON / XML  <->  Java POJO converters. Pure string -> string; output is meant for a new tab.

const JAVA_KEYWORDS = new Set('abstract assert boolean break byte case catch char class const continue default do double else enum extends final finally float for goto if implements import instanceof int interface long native new package private protected public return short static strictfp super switch synchronized this throw throws transient try void volatile while true false null record var'.split(' '));
const RESERVED_CLASSES = new Set(['String', 'Object', 'List', 'Map', 'Set', 'Integer', 'Long', 'Double', 'Boolean', 'Date']);

const upperFirst = s => s.charAt(0).toUpperCase() + s.slice(1);
const lowerFirst = s => s.charAt(0).toLowerCase() + s.slice(1);
const words = s => String(s).split(/[^A-Za-z0-9]+/).filter(Boolean);

function singular(s) {
  if (/ies$/i.test(s) && s.length > 3) return s.slice(0, -3) + 'y';
  if (/(ss|us|is)$/i.test(s)) return s;
  if (/s$/i.test(s) && s.length > 1) return s.slice(0, -1);
  return s;
}
function className(hint) {
  let n = words(hint).map(upperFirst).join('') || 'Item';
  if (/^\d/.test(n)) n = 'N' + n;
  return RESERVED_CLASSES.has(n) ? n + 'Model' : n;
}
function javaName(key) {
  const p = words(key);
  let n = p.map((w, i) => i ? upperFirst(w) : (/^[A-Z0-9]+$/.test(w) ? w.toLowerCase() : lowerFirst(w))).join('') || 'field';
  if (/^\d/.test(n)) n = '_' + n;
  return JAVA_KEYWORDS.has(n) ? n + '_' : n;
}

// ---------- type model ----------
// {k:'prim', t:'String'|'Integer'|'Long'|'Double'|'Boolean'|'Object', unknown?}  {k:'list', of}  {k:'obj', cls}
const UNKNOWN = { k: 'prim', t: 'Object', unknown: true };
const NUM = { Integer: 1, Long: 2, Double: 3 };

function merge(a, b, mode) {
  if (!a) return b;
  if (!b) return a;
  if (a.unknown) return b;
  if (b.unknown) return a;
  if (a.k === 'list' || b.k === 'list') {
    if (mode === 'json' && a.k !== b.k) return { k: 'prim', t: 'Object' };
    return { k: 'list', of: merge(a.k === 'list' ? a.of : a, b.k === 'list' ? b.of : b, mode) };
  }
  if (a.k === 'obj' && b.k === 'obj') return a;
  if (a.k === 'prim' && b.k === 'prim') {
    if (a.t === b.t) return a;
    if (NUM[a.t] && NUM[b.t]) return NUM[a.t] >= NUM[b.t] ? a : b;
    return { k: 'prim', t: mode === 'xml' ? 'String' : 'Object' };
  }
  return mode === 'xml' ? (a.k === 'obj' ? a : b) : { k: 'prim', t: 'Object' };
}

class Model {
  constructor(mode) { this.mode = mode; this.classes = new Map(); }
  cls(name) { if (!this.classes.has(name)) this.classes.set(name, new Map()); return this.classes.get(name); }
  field(cls, key, type, extra = {}) {
    const f = this.cls(cls), old = f.get(key);
    f.set(key, { ...old, ...extra, key, type: merge(old?.type, type, this.mode) });
  }
}

function numType(n) {
  if (!Number.isInteger(n)) return 'Double';
  return Math.abs(n) <= 2147483647 ? 'Integer' : 'Long';
}

function inferJson(m, v, hint) {
  if (v === null || v === undefined) return UNKNOWN;
  if (Array.isArray(v)) {
    let t;
    for (const x of v) t = merge(t, inferJson(m, x, singular(hint)), 'json');
    return { k: 'list', of: t || UNKNOWN };
  }
  switch (typeof v) {
    case 'string': return { k: 'prim', t: 'String' };
    case 'number': return { k: 'prim', t: numType(v) };
    case 'boolean': return { k: 'prim', t: 'Boolean' };
  }
  const cls = className(hint);
  m.cls(cls);
  for (const [k, x] of Object.entries(v)) m.field(cls, k, inferJson(m, x, k));
  return { k: 'obj', cls };
}

function textType(s) {
  s = s.trim();
  if (/^(true|false)$/.test(s)) return 'Boolean';
  if (/^-?\d+$/.test(s)) return Math.abs(+s) <= 2147483647 && s.length < 11 ? 'Integer' : 'Long';
  if (/^-?\d*\.\d+$/.test(s)) return 'Double';
  return 'String';
}

function inferXml(m, el) {
  const kids = [...el.children];
  const text = [...el.childNodes].filter(n => n.nodeType === 3 || n.nodeType === 4).map(n => n.nodeValue).join('').trim();
  if (!el.attributes.length && !kids.length) return { k: 'prim', t: text ? textType(text) : 'String' };
  const cls = className(el.localName);
  m.cls(cls);
  for (const a of el.attributes) {
    if (a.name === 'xmlns' || a.name.startsWith('xmlns:')) continue;
    m.field(cls, a.name, { k: 'prim', t: textType(a.value) }, { attr: true });
  }
  if (text && kids.length === 0) m.field(cls, 'value', { k: 'prim', t: textType(text) }, { text: true });
  else if (text) m.field(cls, 'value', { k: 'prim', t: 'String' }, { text: true });
  const count = {};
  for (const k of kids) count[k.localName] = (count[k.localName] || 0) + 1;
  for (const k of kids) {
    const t = inferXml(m, k);
    m.field(cls, k.localName, count[k.localName] > 1 ? { k: 'list', of: t } : t);
  }
  return { k: 'obj', cls };
}

// ---------- Java emitter ----------
function emitJava(m, rootName, style) {
  const imports = new Set();
  const xml = m.mode === 'xml';
  const typeStr = t => {
    if (t.k === 'prim') return t.t;
    if (t.k === 'list') { imports.add('java.util.List'); return `List<${typeStr(t.of)}>`; }
    return t.cls;
  };
  const ann = (f, jn) => {
    const out = [];
    if (xml) {
      if (f.text) { imports.add('com.fasterxml.jackson.dataformat.xml.annotation.JacksonXmlText'); out.push('@JacksonXmlText'); return out; }
      if (f.type.k === 'list') { imports.add('com.fasterxml.jackson.dataformat.xml.annotation.JacksonXmlElementWrapper'); out.push('@JacksonXmlElementWrapper(useWrapping = false)'); }
      if (f.attr || f.type.k === 'list' || jn !== f.key) {
        imports.add('com.fasterxml.jackson.dataformat.xml.annotation.JacksonXmlProperty');
        out.push(`@JacksonXmlProperty(${f.attr ? 'isAttribute = true, ' : ''}localName = "${f.key}")`);
      }
    } else if (jn !== f.key) {
      imports.add('com.fasterxml.jackson.annotation.JsonProperty');
      out.push(`@JsonProperty("${f.key}")`);
    }
    return out;
  };

  const gen = (name, isRoot, pad) => {
    const fields = [...m.classes.get(name).values()].map(f => ({ ...f, jn: javaName(f.key), ts: typeStr(f.type) }));
    const L = [];
    if (style === 'record') {
      const params = fields.map(f => [...ann(f, f.jn), `${f.ts} ${f.jn}`].join(' '));
      return [`${pad}public record ${name}(${params.join(', ')}) {}`];
    }
    if (style === 'lombok') { imports.add('lombok.Data'); L.push(`${pad}@Data`); }
    L.push(`${pad}public ${isRoot ? '' : 'static '}class ${name} {`);
    for (const f of fields) {
      for (const a of ann(f, f.jn)) L.push(`${pad}    ${a}`);
      L.push(`${pad}    private ${f.ts} ${f.jn};`);
    }
    if (style === 'pojo') for (const f of fields) {
      const cap = upperFirst(f.jn.replace(/^_/, ''));
      L.push('', `${pad}    public ${f.ts} get${cap}() { return ${f.jn}; }`,
        `${pad}    public void set${cap}(${f.ts} ${f.jn}) { this.${f.jn} = ${f.jn}; }`);
    }
    L.push(`${pad}}`);
    return L;
  };

  const names = [rootName, ...[...m.classes.keys()].filter(n => n !== rootName)];
  const lines = gen(rootName, true, '');
  if (names.length > 1) {
    if (style === 'record') lines[0] = lines[0].replace(/ \{\}$/, ' {'); else lines.pop();
    for (const n of names.slice(1)) lines.push('', ...gen(n, false, '    '));
    lines.push('}');
  }
  const out = lines.join('\n');
  const imp = [...imports].sort().map(i => `import ${i};`).join('\n');
  return (imp ? imp + '\n\n' : '') + out + '\n';
}

function jsonToJava(text, rootName, style) {
  const m = new Model('json'), data = JSON.parse(text);
  const root = className(rootName);
  const t = inferJson(m, Array.isArray(data) ? data[0] ?? {} : data, root);
  if (t.k !== 'obj') throw new Error('Top-level JSON must be an object (or an array of objects).');
  if (t.cls !== root) { m.classes.set(root, m.classes.get(t.cls)); if (t.cls !== root) m.classes.delete(t.cls); }
  return emitJava(m, root, style);
}

function xmlToJava(text, rootName, style) {
  const doc = new DOMParser().parseFromString(text, 'application/xml');
  const err = doc.querySelector('parsererror');
  if (err) throw new Error(err.textContent.split('\n')[0]);
  const m = new Model('xml'), root = className(rootName);
  const t = inferXml(m, doc.documentElement);
  if (t.k !== 'obj') { m.cls(root); m.field(root, doc.documentElement.localName, t); return emitJava(m, root, style); }
  if (t.cls !== root) { m.classes.set(root, m.classes.get(t.cls)); m.classes.delete(t.cls); }
  return emitJava(m, root, style);
}

// ---------- Java parser (fields only) ----------
function matchBrace(s, open) {
  let d = 0;
  for (let i = open; i < s.length; i++) {
    if (s[i] === '{') d++;
    else if (s[i] === '}' && --d === 0) return i;
  }
  return s.length - 1;
}
function matchParen(s, open) {
  let d = 0;
  for (let i = open; i < s.length; i++) {
    if (s[i] === '(') d++;
    else if (s[i] === ')' && --d === 0) return i;
  }
  return s.length - 1;
}
function splitTop(s, sep) {
  const out = []; let d = 0, cur = '';
  for (const c of s) {
    if ('<([{'.includes(c)) d++;
    else if ('>)]}'.includes(c)) d--;
    if (c === sep && d === 0) { out.push(cur); cur = ''; } else cur += c;
  }
  if (cur.trim()) out.push(cur);
  return out;
}

function parseDecl(st) {
  // "@A(..) @B private List<X> name" -> {type, name, key, attr, text}
  let key = null, attr = false, text = false;
  for (const a of st.matchAll(/@(\w+)\s*(?:\(([^)]*)\))?/g)) {
    const [, an, args = ''] = a;
    if (an === 'JsonProperty' || an === 'JacksonXmlProperty') {
      const k = args.match(/(?:localName|value)\s*=\s*"([^"]*)"/) || args.match(/^\s*"([^"]*)"/);
      if (k) key = k[1];
      if (/isAttribute\s*=\s*true/.test(args)) attr = true;
    }
    if (an === 'JacksonXmlText') text = true;
  }
  const s = st.replace(/@\w+\s*(?:\([^)]*\))?/g, ' ').replace(/\b(private|protected|public|final|transient|volatile)\b/g, ' ').trim();
  const mm = s.match(/^([\s\S]+?)\s+(\w+)$/);
  if (!mm) return null;
  return { type: mm[1].replace(/\s+/g, ' ').trim(), name: mm[2], key: key || mm[2], attr, text };
}

function parseJava(src) {
  src = src.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
  const classes = new Map(), enums = new Set();
  for (const m of src.matchAll(/\b(class|record|enum|interface)\s+(\w+)/g)) {
    const [, kind, name] = m;
    let i = m.index + m[0].length, params = '';
    if (kind === 'record') {
      const p = src.indexOf('(', i);
      if (p < 0) continue;
      const e = matchParen(src, p);
      params = src.slice(p + 1, e); i = e;
    }
    const open = src.indexOf('{', i);
    if (open < 0) continue;
    const bodyAll = src.slice(open + 1, matchBrace(src, open));
    if (kind === 'enum') { enums.add(name); continue; }
    const fields = [];
    if (kind === 'record') for (const p of splitTop(params, ',')) { const d = parseDecl(p.trim()); if (d) fields.push(d); }
    // flatten: nested blocks (methods, inner classes) -> ';'
    let flat = '', d = 0;
    for (const c of bodyAll) {
      if (c === '{') { if (d++ === 0) flat += ';'; }
      else if (c === '}') d--;
      else if (d === 0) flat += c;
    }
    if (kind !== 'interface') for (const st of flat.split(';')) {
      const annots = (st.match(/@\w+\s*(?:\([^)]*\))?/g) || []).join(' ');
      const bare = st.replace(/@\w+\s*(?:\([^)]*\))?/g, ' ').split('=')[0].trim();   // drop annotations and initialiser
      if (!bare || bare.includes('(') || /\b(static|import|package|return)\b/.test(bare)) continue;
      const f = parseDecl(annots + ' ' + bare);
      if (f && !/,/.test(f.type.replace(/<[^>]*>/g, ''))) fields.push(f);
    }
    classes.set(name, fields);
  }
  return { classes, enums };
}

const STR_TYPES = /^(String|char|Character|CharSequence|UUID|Date|LocalDate|LocalDateTime|LocalTime|Instant|ZonedDateTime|OffsetDateTime|URI|URL|Locale|Duration|Path|File)$/;
const INT_TYPES = /^(int|Integer|long|Long|short|Short|byte|Byte|BigInteger|AtomicInteger|AtomicLong)$/;
const DEC_TYPES = /^(double|Double|float|Float|BigDecimal|Number)$/;
const LIST_TYPES = /^(List|Set|Collection|Iterable|ArrayList|LinkedList|HashSet|LinkedHashSet|TreeSet|Queue|Deque|Stream)$/;
const MAP_TYPES = /^(Map|HashMap|LinkedHashMap|TreeMap|SortedMap|Properties)$/;

function resolveType(t, J) {
  t = t.replace(/\s+/g, '');
  if (t.endsWith('[]')) return { k: 'list', of: resolveType(t.slice(0, -2), J) };
  const m = t.match(/^([\w.]+?)(?:<(.*)>)?$/);
  if (!m) return { k: 'null' };
  const base = m[1].split('.').pop(), args = m[2] ? splitTop(m[2], ',') : [];
  if (base === 'Optional' || base === 'AtomicReference') return args[0] ? resolveType(args[0], J) : { k: 'null' };
  if (STR_TYPES.test(base) || J.enums.has(base)) return { k: 'prim', v: base === 'Date' || /Date|Time|Instant/.test(base) ? '2024-01-01T00:00:00Z' : 'string' };
  if (INT_TYPES.test(base)) return { k: 'prim', v: 0 };
  if (DEC_TYPES.test(base)) return { k: 'prim', v: 0.0 };
  if (base === 'boolean' || base === 'Boolean') return { k: 'prim', v: false };
  if (LIST_TYPES.test(base)) return { k: 'list', of: args[0] ? resolveType(args[0], J) : { k: 'null' } };
  if (MAP_TYPES.test(base)) return { k: 'map' };
  if (J.classes.has(base)) return { k: 'cls', name: base };
  return { k: 'null' };
}

function javaRoot(J) {
  const referenced = new Set();
  for (const fs of J.classes.values()) for (const f of fs) for (const m of f.type.matchAll(/\w+/g)) referenced.add(m[0]);
  const names = [...J.classes.keys()];
  return names.find(n => !referenced.has(n)) || names[0];
}

function sampleJson(t, J, seen) {
  switch (t.k) {
    case 'prim': return t.v;
    case 'list': return [sampleJson(t.of, J, seen)];
    case 'map': return {};
    case 'cls': {
      if (seen.includes(t.name)) return null;
      const o = {};
      for (const f of J.classes.get(t.name)) o[f.key] = sampleJson(resolveType(f.type, J), J, [...seen, t.name]);
      return o;
    }
    default: return null;
  }
}

function javaToJson(src) {
  const J = parseJava(src);
  if (!J.classes.size) throw new Error('No Java class or record found.');
  return JSON.stringify(sampleJson({ k: 'cls', name: javaRoot(J) }, J, []), null, 2);
}

const esc = s => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

function javaToXml(src) {
  const J = parseJava(src);
  if (!J.classes.size) throw new Error('No Java class or record found.');
  const el = (tag, t, seen, pad) => {
    switch (t.k) {
      case 'prim': return `${pad}<${tag}>${esc(t.v)}</${tag}>`;
      case 'list': return el(tag, t.of, seen, pad);
      case 'cls': {
        if (seen.includes(t.name)) return `${pad}<${tag}/>`;
        const fs = J.classes.get(t.name);
        const attrs = fs.filter(f => f.attr).map(f => ` ${f.key}="${esc(resolveType(f.type, J).v ?? '')}"`).join('');
        const kids = fs.filter(f => !f.attr).map(f => {
          const rt = resolveType(f.type, J);
          return f.text ? { text: esc(rt.v ?? 'text') } : { xml: el(f.key, rt, [...seen, t.name], pad + '  ') };
        });
        if (!kids.length) return `${pad}<${tag}${attrs}/>`;
        if (kids.length === 1 && kids[0].text !== undefined) return `${pad}<${tag}${attrs}>${kids[0].text}</${tag}>`;
        return `${pad}<${tag}${attrs}>\n${kids.map(k => k.xml ?? `${pad}  ${k.text}`).join('\n')}\n${pad}</${tag}>`;
      }
      default: return `${pad}<${tag}/>`;
    }
  };
  const root = javaRoot(J);
  return '<?xml version="1.0" encoding="UTF-8"?>\n' + el(lowerFirst(root), { k: 'cls', name: root }, [], '') + '\n';
}

window.Convert = { jsonToJava, xmlToJava, javaToJson, javaToXml };
