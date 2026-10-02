import type { AddJob, ApiError, AppInfo, BinInfo, BinaryProgress, Job, JobOptions, Settings, Subscription } from './types';
import * as mock from './mock';

export const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!inTauri) return mock.call(cmd, args) as Promise<T>;
  const { invoke } = await import('@tauri-apps/api/core');
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw toApiError(e);
  }
}

export function toApiError(e: unknown): ApiError {
  if (e && typeof e === 'object' && 'message' in e) return e as ApiError;
  return { message: String(e) };
}

export const api = {
  appInfo: () => call<AppInfo>('app_info'),
  installTools: (names: string[]) => call<BinInfo[]>('install_tools', { names }),
  latestYtdlp: () => call<string>('latest_ytdlp_version'),
  getSettings: () => call<Settings>('get_settings'),
  saveSettings: (settings: Settings) => call<Settings>('save_settings', { settings }),
  probe: (req: { url: string; cookiesBrowser?: string | null; cookiesFile?: string | null; proxy?: string | null }) =>
    call<unknown>('probe', { req }),
  previewCommand: (options: JobOptions) => call<string>('preview_command', { options }),
  listJobs: () => call<Job[]>('list_jobs'),
  jobLog: (id: string) => call<string[]>('job_log', { id }),
  addJob: (job: AddJob) => call<Job>('add_job', { job }),
  cancelJob: (id: string) => call<void>('cancel_job', { id }),
  retryJob: (id: string) => call<void>('retry_job', { id }),
  removeJob: (id: string) => call<void>('remove_job', { id }),
  clearFinished: () => call<void>('clear_finished'),
  listSubs: () => call<Subscription[]>('list_subscriptions'),
  saveSub: (subscription: Subscription) => call<Subscription[]>('save_subscription', { subscription }),
  deleteSub: (id: string) => call<Subscription[]>('delete_subscription', { id }),
  checkSubNow: (id: string) => call<boolean>('check_subscription_now', { id }),
  revealPath: (path: string) => call<void>('reveal_path', { path }),
  openPath: (path: string) => call<void>('open_path', { path }),
  quit: () => call<void>('quit_app'),
};

/** Subscribe to a backend event; returns an unsubscribe function. */
export async function on<T>(event: string, handler: (payload: T) => void): Promise<() => void> {
  if (!inTauri) return mock.on(event, handler as (p: unknown) => void);
  const { listen } = await import('@tauri-apps/api/event');
  return listen<T>(event, e => handler(e.payload));
}

export async function pickFolder(current?: string): Promise<string | null> {
  if (!inTauri) return mock.pickFolder();
  const { open } = await import('@tauri-apps/plugin-dialog');
  const r = await open({ directory: true, multiple: false, defaultPath: current || undefined });
  return typeof r === 'string' ? r : null;
}

export async function pickFile(): Promise<string | null> {
  if (!inTauri) return null;
  const { open } = await import('@tauri-apps/plugin-dialog');
  const r = await open({ directory: false, multiple: false });
  return typeof r === 'string' ? r : null;
}

export async function readClipboard(): Promise<string> {
  try {
    if (inTauri) {
      const { readText } = await import('@tauri-apps/plugin-clipboard-manager');
      return (await readText()) ?? '';
    }
    return await navigator.clipboard.readText();
  } catch {
    return '';
  }
}

export async function copyText(text: string): Promise<void> {
  try { await navigator.clipboard.writeText(text); } catch { /* ignore */ }
}

export async function onWindowFocus(handler: () => void): Promise<() => void> {
  if (!inTauri) {
    window.addEventListener('focus', handler);
    return () => window.removeEventListener('focus', handler);
  }
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  return getCurrentWindow().onFocusChanged(({ payload }) => { if (payload) handler(); });
}
