import { api, on, onWindowFocus, readClipboard, toApiError } from './api';
import { BUILTIN_PRESETS } from './defaults';
import { isUrl } from './format';
import type { ApiError, AppInfo, BinaryProgress, Job, Preset, Settings, Subscription } from './types';

export type Page = 'download' | 'queue' | 'subs' | 'settings';

export interface Toast {
  id: number;
  kind: 'info' | 'ok' | 'error';
  text: string;
  hint?: string | null;
  action?: { label: string; run: () => void };
}

export const app = $state({
  ready: false,
  page: 'download' as Page,
  jobs: [] as Job[],
  subs: [] as Subscription[],
  settings: null as Settings | null,
  info: null as AppInfo | null,
  toasts: [] as Toast[],
  binProgress: {} as Record<string, BinaryProgress>,
  installing: false,
  latestYtdlp: null as string | null,
  /** A URL handed over from the clipboard watcher or drag & drop. */
  incomingUrl: null as string | null,
});

let toastSeq = 1;
export function toast(kind: Toast['kind'], text: string, opts: Partial<Pick<Toast, 'hint' | 'action'>> = {}, ms = 6000) {
  const t: Toast = { id: toastSeq++, kind, text, ...opts };
  app.toasts.push(t);
  setTimeout(() => dismissToast(t.id), kind === 'error' || opts.action ? ms * 2 : ms);
}
export const dismissToast = (id: number) => { app.toasts = app.toasts.filter(t => t.id !== id); };
export function toastError(e: unknown) {
  const err: ApiError = toApiError(e);
  toast('error', err.message, { hint: err.hint });
}

export function allPresets(): Preset[] {
  return [...BUILTIN_PRESETS, ...(app.settings?.presets ?? [])];
}

export function applyTheme(theme: Settings['theme']) {
  const el = document.documentElement;
  if (theme === 'light' || theme === 'dark') el.dataset.theme = theme;
  else delete el.dataset.theme;
}

export async function saveSettings(patch: Partial<Settings>) {
  if (!app.settings) return;
  const next = { ...$state.snapshot(app.settings), ...patch } as Settings;
  app.settings = next;
  applyTheme(next.theme);
  try {
    app.settings = await api.saveSettings(next);
  } catch (e) { toastError(e); }
}

export function upsertJob(j: Job) {
  const i = app.jobs.findIndex(x => x.id === j.id);
  if (i >= 0) app.jobs[i] = { ...j, log: j.log.length ? j.log : app.jobs[i].log };
  else app.jobs.push(j);
}

export const missingTools = () =>
  (app.info?.bins ?? []).filter(b => b.source === 'missing' && ['yt-dlp', 'ffmpeg', 'deno'].includes(b.name));

export async function refreshInfo() {
  app.info = await api.appInfo();
}

export async function installTools(names: string[]) {
  if (app.installing || !names.length) return;
  app.installing = true;
  try {
    await api.installTools(names);
    await refreshInfo();
    toast('ok', `Installed ${names.join(', ')}`);
  } catch (e) {
    toastError(e);
  } finally {
    app.installing = false;
    app.binProgress = {};
  }
}

export async function checkForYtdlpUpdate() {
  try {
    app.latestYtdlp = await api.latestYtdlp();
  } catch { app.latestYtdlp = null; }
}

export const ytdlpUpdateAvailable = () => {
  const cur = app.info?.bins.find(b => b.name === 'yt-dlp');
  return !!(cur?.source === 'managed' && cur.version && app.latestYtdlp && cur.version !== app.latestYtdlp);
};

let lastClip = '';
async function checkClipboard() {
  if (!app.settings?.clipboardWatch) return;
  const t = (await readClipboard()).trim();
  if (!t || t === lastClip || !isUrl(t) || t.includes('\n')) return;
  lastClip = t;
  toast('info', 'Link copied — download it?', { hint: t.length > 70 ? t.slice(0, 70) + '…' : t, action: { label: 'Use link', run: () => { app.incomingUrl = t; app.page = 'download'; } } });
}

export async function init() {
  try {
    const [settings, info, jobs, subs] = await Promise.all([api.getSettings(), api.appInfo(), api.listJobs(), api.listSubs()]);
    app.settings = settings; app.info = info; app.jobs = jobs; app.subs = subs;
    applyTheme(settings.theme);
  } catch (e) { toastError(e); }
  await on<Job>('job-update', upsertJob);
  await on<string>('job-removed', id => { app.jobs = app.jobs.filter(j => j.id !== id); });
  await on<BinaryProgress>('binary-progress', p => { app.binProgress[p.name] = p; });
  await on<void>('subscriptions-changed', async () => { app.subs = await api.listSubs(); });
  await onWindowFocus(checkClipboard);
  app.ready = true;
  checkForYtdlpUpdate();
}
