import type { EntryRow, FormatRow, Job, Probe, Status } from './types';

export function bytes(n?: number | null): string {
  if (n == null || !isFinite(n)) return '—';
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
  return `${n >= 100 || i === 0 ? n.toFixed(0) : n.toFixed(1)} ${u[i]}`;
}

export function speed(n?: number | null): string {
  return n ? `${bytes(n)}/s` : '';
}

/** 3725 -> "1:02:05", 65 -> "1:05" */
export function clock(sec?: number | null): string {
  if (sec == null || !isFinite(sec)) return '—';
  sec = Math.max(0, Math.round(sec));
  const h = Math.floor(sec / 3600), m = Math.floor((sec % 3600) / 60), s = sec % 60;
  const p = (x: number) => String(x).padStart(2, '0');
  return h ? `${h}:${p(m)}:${p(s)}` : `${m}:${p(s)}`;
}

/** "1:30" | "90" | "1:02:03" -> seconds; null when invalid */
export function parseClock(text: string): number | null {
  const t = text.trim();
  if (!t) return null;
  const parts = t.split(':').map(Number);
  if (parts.some(n => !isFinite(n) || n < 0) || parts.length > 3) return null;
  return parts.reduce((acc, n) => acc * 60 + n, 0);
}

export function until(ts: number, now = Date.now() / 1000): string {
  let d = Math.round(ts - now);
  if (d <= 0) return 'any moment now';
  const days = Math.floor(d / 86400); d -= days * 86400;
  const h = Math.floor(d / 3600); d -= h * 3600;
  const m = Math.floor(d / 60);
  if (days) return `${days}d ${h}h`;
  if (h) return `${h}h ${m}m`;
  return `${Math.max(m, 1)}m`;
}

export function ago(ts?: number | null, now = Date.now() / 1000): string {
  if (!ts) return 'never';
  const d = Math.round(now - ts);
  if (d < 60) return 'just now';
  if (d < 3600) return `${Math.floor(d / 60)} min ago`;
  if (d < 86400) return `${Math.floor(d / 3600)} h ago`;
  return `${Math.floor(d / 86400)} d ago`;
}

export function shortName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

export function isUrl(s: string): boolean {
  return /^https?:\/\/\S+$/i.test(s.trim());
}

export function extractUrls(text: string): string[] {
  return [...new Set(text.split(/\s+/).map(s => s.trim()).filter(isUrl))];
}

/** [1,2,3,5,7,8] -> "1-3,5,7-8" (yt-dlp --playlist-items syntax) */
export function toRanges(nums: number[]): string {
  const s = [...new Set(nums)].sort((a, b) => a - b);
  const out: string[] = [];
  for (let i = 0; i < s.length;) {
    let j = i;
    while (j + 1 < s.length && s[j + 1] === s[j] + 1) j++;
    out.push(j > i ? `${s[i]}-${s[j]}` : `${s[i]}`);
    i = j + 1;
  }
  return out.join(',');
}

export const ACTIVE: Status[] = ['starting', 'waiting', 'downloading', 'live', 'processing'];
export const isActive = (j: Job) => ACTIVE.includes(j.status);
export const isFinished = (j: Job) => ['done', 'failed', 'cancelled'].includes(j.status);

/* eslint-disable @typescript-eslint/no-explicit-any */
function formatKind(f: any): FormatRow['kind'] {
  const v = f.vcodec && f.vcodec !== 'none';
  const a = f.acodec && f.acodec !== 'none';
  return v && a ? 'both' : v ? 'video' : 'audio';
}

/** Reduce yt-dlp's big JSON to what the UI needs. */
export function normalizeProbe(raw: any, url: string): Probe {
  if (raw._upcoming) {
    return { kind: 'upcoming', url, title: 'Scheduled video', note: raw.note, formats: [], subtitleLangs: [], entries: [] };
  }
  const isPlaylist = raw._type === 'playlist' || Array.isArray(raw.entries);
  const thumb = raw.thumbnail ?? raw.thumbnails?.filter((t: any) => t.url).slice(-1)[0]?.url;
  const base = {
    url, title: raw.title ?? raw.id ?? url, channel: raw.uploader ?? raw.channel ?? raw.uploader_id,
    thumbnail: thumb, duration: raw.duration, releaseTimestamp: raw.release_timestamp,
  };
  if (isPlaylist) {
    const entries: EntryRow[] = (raw.entries ?? []).filter(Boolean).map((e: any, i: number) => ({
      index: i + 1, id: e.id ?? String(i), title: e.title ?? e.id ?? `Item ${i + 1}`,
      duration: e.duration, thumbnail: e.thumbnails?.[0]?.url ?? e.thumbnail,
    }));
    return { ...base, kind: 'playlist', thumbnail: base.thumbnail ?? entries[0]?.thumbnail, formats: [], subtitleLangs: [], entries };
  }
  const formats: FormatRow[] = (raw.formats ?? [])
    .filter((f: any) => f.format_id && f.protocol !== 'mhtml' && !String(f.format_note ?? '').includes('storyboard'))
    .map((f: any) => ({
      id: String(f.format_id), ext: f.ext ?? '', resolution: f.resolution ?? (f.height ? `${f.width ?? ''}x${f.height}` : 'audio only'),
      height: f.height ?? 0, fps: f.fps ?? undefined, vcodec: (f.vcodec ?? 'none').split('.')[0], acodec: (f.acodec ?? 'none').split('.')[0],
      size: f.filesize ?? f.filesize_approx ?? undefined, tbr: f.tbr ?? undefined, note: f.format_note ?? undefined,
      hdr: f.dynamic_range && f.dynamic_range !== 'SDR' ? f.dynamic_range : undefined, kind: formatKind(f),
    }));
  const status = raw.live_status;
  const kind: Probe['kind'] = status === 'is_upcoming' ? 'upcoming' : status === 'is_live' ? 'live' : 'video';
  return { ...base, kind, formats, subtitleLangs: Object.keys(raw.subtitles ?? {}), entries: [] };
}
