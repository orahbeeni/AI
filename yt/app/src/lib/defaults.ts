import type { JobOptions, Preset, TranscribeOptions } from './types';

export function blankOptions(url = '', outputDir = ''): JobOptions {
  return {
    url, outputDir,
    audioOnly: false, liveFromStart: false,
    subtitles: [], autoSubs: false, embedSubs: false,
    embedThumbnail: false, embedMetadata: false, embedChapters: false,
    writeThumbnail: false, writeInfoJson: false, writeDescription: false,
    noPlaylist: false, playlistReverse: false, useArchive: false,
    sponsorblockRemove: [], sponsorblockMark: [], splitChapters: false,
    downloadSections: [], forceKeyframes: false,
    restrictFilenames: false,
  };
}

export const MP4_BEST = 'bv*[ext=mp4]+ba[ext=m4a]/b[ext=mp4]';
// Prefer MP4 but fall back to anything, so a cap never fails on unusual videos.
export const MP4_ANY = 'bv*[ext=mp4]+ba[ext=m4a]/b[ext=mp4]/bv*+ba/b';
/** `res:720` caps the SHORTER side, so vertical videos (720x1280) count as 720p too. */
export const capSort = (h: number) => `res:${h}`;

/** Built-in presets. Options here are merged over the blank defaults. */
export const BUILTIN_PRESETS: Preset[] = [
  {
    id: 'live-mp4', name: 'Scheduled / Live → MP4', icon: 'live', builtin: true,
    description: 'Waits for a scheduled video to start, then records it from the beginning as MP4.',
    options: { waitForVideo: { min: 30, max: 60 }, liveFromStart: true, format: MP4_BEST },
  },
  { id: 'best', name: 'Best quality', icon: 'film', builtin: true,
    description: 'The highest quality available. The file type is chosen for you.', options: {} },
  { id: 'mp4-best', name: 'Best MP4', icon: 'film', builtin: true,
    description: 'Highest quality that plays almost everywhere (MP4 + AAC).',
    options: { format: MP4_BEST, container: 'mp4' } },
  { id: 'mp4-1080', name: '1080p MP4', icon: 'film', builtin: true,
    description: 'Up to 1080p, MP4.', options: { format: MP4_ANY, formatSort: capSort(1080), container: 'mp4' } },
  { id: 'mp4-720', name: '720p MP4', icon: 'film', builtin: true,
    description: 'Up to 720p — smaller files.', options: { format: MP4_ANY, formatSort: capSort(720), container: 'mp4' } },
  { id: 'mp4-480', name: '480p MP4', icon: 'film', builtin: true,
    description: 'Up to 480p — smallest video files.', options: { format: MP4_ANY, formatSort: capSort(480), container: 'mp4' } },
  { id: 'mp3', name: 'Audio · MP3', icon: 'music', builtin: true,
    description: 'Audio only, converted to MP3.',
    options: { audioOnly: true, audioFormat: 'mp3', audioQuality: '0', embedThumbnail: true, embedMetadata: true } },
  { id: 'm4a', name: 'Audio · M4A', icon: 'music', builtin: true,
    description: 'Audio only, AAC in M4A.',
    options: { audioOnly: true, audioFormat: 'm4a', audioQuality: '0', embedThumbnail: true, embedMetadata: true } },
  { id: 'flac', name: 'Audio · FLAC', icon: 'music', builtin: true,
    description: 'Audio only, lossless FLAC.', options: { audioOnly: true, audioFormat: 'flac', embedMetadata: true } },
];

export const NETWORK_KEYS = ['cookiesBrowser', 'cookiesFile', 'proxy', 'rateLimit', 'retries', 'concurrentFragments'] as const;

export const SPONSOR_CATEGORIES: [string, string][] = [
  ['sponsor', 'Sponsor'], ['intro', 'Intro animation'], ['outro', 'Outro / end cards'],
  ['selfpromo', 'Self-promotion'], ['interaction', 'Subscribe reminders'],
  ['preview', 'Preview / recap'], ['music_offtopic', 'Non-music sections'], ['filler', 'Filler / tangents'],
];

export const FILENAME_TOKENS: [string, string][] = [
  ['%(title)s', 'Title'], ['%(uploader)s', 'Channel'], ['%(upload_date)s', 'Upload date'],
  ['%(id)s', 'Video ID'], ['%(resolution)s', 'Resolution'], ['%(playlist)s', 'Playlist'],
  ['%(playlist_index)s', 'Number in playlist'], ['%(ext)s', 'Extension'],
];

/** Used when no pattern is chosen. Keep in step with DEFAULT_TEMPLATE in args.rs. */
export const DEFAULT_TEMPLATE = '%(upload_date>%Y-%m-%d|unknown-date)s - %(title)s [%(id)s].%(ext)s';

export const FILENAME_TEMPLATES: [string, string][] = [
  [DEFAULT_TEMPLATE, 'Date - Title [id] (default)'],
  ['%(title)s [%(id)s].%(ext)s', 'Title [id]'],
  ['%(title)s.%(ext)s', 'Title only'],
  ['%(uploader)s/%(title)s.%(ext)s', 'Channel folder / Title'],
  ['%(upload_date)s - %(title)s.%(ext)s', 'Date - Title'],
  ['%(playlist)s/%(playlist_index)s - %(title)s.%(ext)s', 'Playlist folder / 01 - Title'],
];

export const BROWSERS = ['chrome', 'firefox', 'edge', 'brave', 'chromium', 'opera', 'safari', 'vivaldi'];

export type BulkMode = 'quick' | 'steady' | 'careful';

/**
 * How carefully to fetch many videos. YouTube slows or blocks IP addresses that ask for lots of
 * videos quickly, so the careful modes go one at a time with random pauses between videos.
 */
export const BULK_MODES: Record<BulkMode, { label: string; short: string; hint: string; pause: { min: number; max: number } | null }> = {
  quick: { label: 'Quick', short: 'No pauses', pause: null,
    hint: 'No pauses, a couple of videos at the same time. Fine for a handful of videos.' },
  steady: { label: 'Steady', short: 'One at a time, short pauses', pause: { min: 5, max: 20 },
    hint: 'One video at a time with a random 5–20 second pause. A good default for 10–100 videos.' },
  careful: { label: 'Careful', short: 'One at a time, long pauses', pause: { min: 20, max: 60 },
    hint: 'One video at a time with a random 20–60 second pause. For hundreds of videos, overnight runs, or after YouTube has blocked you.' },
};
/** "Auto" switches from Quick to Steady at this many videos. */
export const AUTO_BULK_FROM = 10;
export const resolveBulkMode = (mode: BulkMode | 'auto' | null | undefined, count: number): BulkMode =>
  mode && mode !== 'auto' ? mode : count >= AUTO_BULK_FROM ? 'steady' : 'quick';

export const DEFAULT_TRANSCRIBE: TranscribeOptions = { model: 'small', formats: ['txt'], language: 'auto', intervalSecs: 30, speakers: 'off' };

export const SPEAKER_CHOICES: [string, string][] = [
  ['off', 'Don\'t label speakers'], ['auto', 'Detect speakers automatically'],
  ['2', '2 people'], ['3', '3 people'], ['4', '4 people'], ['5', '5 people'], ['6', '6 people'],
];

export const TRANSCRIBE_LANGUAGES: [string, string][] = [
  ['auto', 'Detect automatically'], ['en', 'English'], ['af', 'Afrikaans'], ['ar', 'Arabic'], ['fa', 'Persian'], ['ur', 'Urdu'],
  ['hi', 'Hindi'], ['id', 'Indonesian'], ['tr', 'Turkish'], ['fr', 'French'], ['de', 'German'], ['es', 'Spanish'],
  ['pt', 'Portuguese'], ['it', 'Italian'], ['nl', 'Dutch'], ['ru', 'Russian'], ['zh', 'Chinese'], ['ja', 'Japanese'],
];

export const TIMESTAMP_INTERVALS: [number, string][] = [
  [15, 'Every 15 seconds'], [30, 'Every 30 seconds'], [60, 'Every minute'], [120, 'Every 2 minutes'], [300, 'Every 5 minutes'],
];
