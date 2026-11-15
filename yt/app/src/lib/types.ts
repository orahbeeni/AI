export type Status =
  | 'queued' | 'starting' | 'waiting' | 'downloading' | 'live'
  | 'processing' | 'done' | 'failed' | 'cancelled';

export interface WaitRange { min: number; max: number }

export interface TranscribeOptions {
  /** One of the Whisper model names offered by the backend (tiny … large-v3). */
  model: string;
  /** Any of 'txt' (with periodic [HH:MM:SS] marks), 'srt', 'vtt'. */
  formats: string[];
  /** ISO code such as 'en', or 'auto' to detect it. */
  language: string;
  /** Seconds between the [HH:MM:SS] marks in the text transcript. */
  intervalSecs: number;
  /** Label who is speaking: 'off', 'auto', or how many people ('2'…'8'). */
  speakers: string;
}

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
  /** Random pause before each video (set from the bulk mode). */
  pauseBetween?: WaitRange | null;
  /** Run alone, not alongside other downloads (set from the bulk mode). */
  oneAtATime?: boolean;
  /** How carefully to fetch many videos; the app turns it into the two options above. */
  bulkMode?: 'auto' | 'quick' | 'steady' | 'careful';
  proxy?: string | null;
  retries?: number | null;
  concurrentFragments?: number | null;
  restrictFilenames: boolean;
  transcribe?: TranscribeOptions | null;
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
  transcripts: string[];
  transcriptError: string | null;
  /** Seconds spent in each finished phase (wait, setup, prepare, download, record, convert, transcribe). */
  timings: Record<string, number>;
  /** The phase running now and when it began (unix seconds). */
  phase: string | null;
  phaseStarted: number | null;
  outputBytes: number;
  transcriptStats: { model: string; language: string; audioSecs: number; words: number; speakers: number } | null;
  isLive: boolean;
  /** A file that was already on the computer: only a transcript is made. */
  localFile?: boolean;
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

/** One earlier lookup, with everything that was chosen for it, so it can be reopened as it was. */
export interface RecentLookup {
  /** The link(s) looked up; the first is the key. */
  urls: string[];
  title: string;
  thumbnail?: string;
  kind?: 'video' | 'playlist' | 'live' | 'upcoming';
  /** Unix seconds of the last time it was looked up. */
  at: number;
  presetId?: string;
  /** Every option as last set for this link. */
  options: Partial<JobOptions>;
  /** Playlist videos that were ticked; absent means all of them. */
  picked?: number[];
  /** Whether the options panel was open, and on which tab, so the page looks as it was left. */
  view?: { showOptions: boolean; tab: string };
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
  /** A saved preset that every new link starts from; empty means "my last choices". */
  defaultPreset?: string;
  /** The options chosen for the last download, reused for the next link. */
  lastOptions?: Partial<JobOptions>;
  /** The last few lookups with their settings, newest first. */
  recent?: RecentLookup[];
  /** The Whisper model used for the last transcript. */
  lastModel?: string;
  /** Transcript choices last used on a file that was already on the computer. */
  lastFileTranscribe?: TranscribeOptions;
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
  buildNumber: string;
  buildTime: string;
  buildCommit: string;
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

export interface ModelStatus {
  name: string;
  label: string;
  sizeMb: number;
  hint: string;
  installed: boolean;
}

export interface WhisperStatus {
  engineInstalled: boolean;
  speakersInstalled: boolean;
  speakersMb: number;
  dir: string;
  models: ModelStatus[];
}

export interface BinaryProgress {
  /** A tool name, 'whisper' (the engine), 'speakers' or 'model:<name>'. */
  name: string;
  stage: 'downloading' | 'verifying' | 'unpacking' | 'setting up Python' | 'installing packages' | 'done';
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
