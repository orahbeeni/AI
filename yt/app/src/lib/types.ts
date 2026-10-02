export type Status =
  | 'queued' | 'starting' | 'waiting' | 'downloading' | 'live'
  | 'processing' | 'done' | 'failed' | 'cancelled';

export interface WaitRange { min: number; max: number }

export interface JobOptions {
  url: string;
  outputDir: string;
  outputTemplate?: string | null;
  format?: string | null;
  formatSort?: string | null;
  container?: string | null;
  audioOnly: boolean;
  audioFormat?: string | null;
  audioQuality?: string | null;
  waitForVideo?: WaitRange | null;
  liveFromStart: boolean;
  subtitles: string[];
  autoSubs: boolean;
  embedSubs: boolean;
  subFormat?: string | null;
  embedThumbnail: boolean;
  embedMetadata: boolean;
  embedChapters: boolean;
  writeThumbnail: boolean;
  writeInfoJson: boolean;
  writeDescription: boolean;
  playlistItems?: string | null;
  noPlaylist: boolean;
  playlistReverse: boolean;
  useArchive: boolean;
  archiveName?: string | null;
  sponsorblockRemove: string[];
  sponsorblockMark: string[];
  splitChapters: boolean;
  downloadSections: string[];
  forceKeyframes: boolean;
  cookiesBrowser?: string | null;
  cookiesFile?: string | null;
  rateLimit?: string | null;
  proxy?: string | null;
  retries?: number | null;
  concurrentFragments?: number | null;
  restrictFilenames: boolean;
  extraArgs?: string | null;
}

export interface Job {
  id: string;
  url: string;
  title: string | null;
  thumbnail: string | null;
  options: JobOptions;
  status: Status;
  percent: number | null;
  speed: number | null;
  eta: number | null;
  downloaded: number;
  total: number | null;
  elapsed: number | null;
  stream: number;
  playlist: [number, number] | null;
  message: string | null;
  error: string | null;
  hint: string | null;
  files: string[];
  isLive: boolean;
  source: string | null;
  command: string;
  createdAt: number;
  startedAt: number | null;
  finishedAt: number | null;
  log: string[];
}

export interface AddJob {
  options: JobOptions;
  title?: string | null;
  thumbnail?: string | null;
  isLive?: boolean;
  source?: string | null;
}

export interface Preset {
  id: string;
  name: string;
  description: string;
  icon: string;
  builtin?: boolean;
  options: Partial<JobOptions>;
}

export interface Settings {
  version: number;
  downloadDir: string;
  maxConcurrent: number;
  closeToTray: boolean;
  notifications: boolean;
  binarySource: 'managed' | 'system';
  customYtdlpPath: string;
  afterQueue: 'none' | 'notify' | 'quit' | 'shutdown';
  // UI-owned
  theme?: 'system' | 'light' | 'dark';
  clipboardWatch?: boolean;
  presets?: Preset[];
  lastPreset?: string;
  network?: Partial<Pick<JobOptions, 'cookiesBrowser' | 'cookiesFile' | 'proxy' | 'rateLimit' | 'retries' | 'concurrentFragments'>>;
  [k: string]: unknown;
}

export interface BinInfo {
  name: string;
  path: string | null;
  version: string | null;
  source: 'managed' | 'system' | 'custom' | 'missing';
}

export interface AppInfo {
  version: string;
  os: string;
  arch: string;
  portable: boolean;
  dataDir: string;
  binDir: string;
  defaultDownloadDir: string;
  bins: BinInfo[];
}

export interface Subscription {
  id: string;
  name: string;
  url: string;
  intervalMinutes: number;
  enabled: boolean;
  options: JobOptions;
  lastCheck: number | null;
  lastNote: string | null;
}

export interface ApiError { message: string; hint?: string | null }

export interface BinaryProgress {
  name: string;
  stage: 'downloading' | 'verifying' | 'unpacking' | 'done';
  downloaded: number;
  total: number | null;
}

/** A trimmed view of yt-dlp's `-J` output. */
export interface Probe {
  kind: 'video' | 'playlist' | 'live' | 'upcoming';
  url: string;
  title: string;
  channel?: string;
  thumbnail?: string;
  duration?: number;
  releaseTimestamp?: number;
  note?: string;
  formats: FormatRow[];
  subtitleLangs: string[];
  entries: EntryRow[];
}

export interface EntryRow {
  index: number;
  id: string;
  title: string;
  duration?: number;
  thumbnail?: string;
}

export interface FormatRow {
  id: string;
  ext: string;
  resolution: string;
  height: number;
  fps?: number;
  vcodec: string;
  acodec: string;
  size?: number;
  tbr?: number;
  note?: string;
  hdr?: string;
  kind: 'video' | 'audio' | 'both';
}
