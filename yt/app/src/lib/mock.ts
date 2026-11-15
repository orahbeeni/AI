// A tiny in-browser stand-in for the Rust backend, used by `npm run dev` in a
// plain browser so the UI can be developed and tested without the native shell.
import type { AppInfo, BinInfo, Job, Settings, Subscription, WhisperStatus } from './types';

const listeners = new Map<string, Set<(p: unknown) => void>>();
const emit = (ev: string, p: unknown) => listeners.get(ev)?.forEach(h => h(p));
export function on(ev: string, h: (p: unknown) => void) {
  if (!listeners.has(ev)) listeners.set(ev, new Set());
  listeners.get(ev)!.add(h);
  return () => listeners.get(ev)!.delete(h);
}
export const pickFolder = async () => '/home/you/Videos/YTGrab';
export const pickMediaFiles = async () => ['/home/you/Videos/interview part 1.mp4', '/home/you/Music/podcast episode 12.mp3'];

let settings: Settings = {
  version: 1, downloadDir: '/home/you/Downloads', maxConcurrent: 2, closeToTray: true, notifications: true,
  binarySource: 'managed', customYtdlpPath: '', afterQueue: 'none',
};
let bins: BinInfo[] = [
  { name: 'yt-dlp', path: '/app/bin/yt-dlp', version: '2026.08.19', source: 'managed' },
  { name: 'ffmpeg', path: null, version: null, source: 'missing' },
  { name: 'ffprobe', path: null, version: null, source: 'missing' },
  { name: 'deno', path: null, version: null, source: 'missing' },
];
let whisper: WhisperStatus = {
  engineInstalled: false, speakersInstalled: false, speakersMb: 36, dir: '/home/you/.local/share/ytgrab/whisper',
  models: [
    { name: 'tiny', label: 'Tiny', sizeMb: 75, hint: 'Fastest. Rough accuracy.', installed: false },
    { name: 'base', label: 'Base', sizeMb: 145, hint: 'Fast.', installed: false },
    { name: 'small', label: 'Small', sizeMb: 485, hint: 'Good balance for clear speech.', installed: false },
    { name: 'medium', label: 'Medium', sizeMb: 1500, hint: 'More accurate, slower.', installed: false },
    { name: 'turbo', label: 'Turbo (large-v3)', sizeMb: 1620, hint: 'Near-best accuracy.', installed: false },
    { name: 'large-v3', label: 'Large v3', sizeMb: 3100, hint: 'Most accurate, very slow on CPU.', installed: false },
  ],
};
let jobs: Job[] = [];
// `npm run dev` + `?demo` pre-fills the queue so the job cards can be looked at.
if (typeof location !== 'undefined' && location.search.includes('demo')) {
  const t = Date.now() / 1000;
  const base = { url: 'https://youtu.be/x', thumbnail: null, options: { url: 'x', outputDir: '/home/you/Downloads', audioOnly: true, audioFormat: 'opus', liveFromStart: false, subtitles: [], autoSubs: false, embedSubs: false, embedThumbnail: false, embedMetadata: false, embedChapters: false, writeThumbnail: false, writeInfoJson: false, writeDescription: false, noPlaylist: false, playlistReverse: false, useArchive: false, sponsorblockRemove: [], sponsorblockMark: [], splitChapters: false, downloadSections: [], forceKeyframes: false, restrictFilenames: false }, percent: null, speed: null, eta: null, downloaded: 0, total: null, elapsed: null, stream: 1, playlist: null, message: null, error: null, hint: null, files: [], transcripts: [], transcriptError: null, timings: {}, phase: null, phaseStarted: null, outputBytes: 0, transcriptStats: null, isLive: false, source: null, command: 'yt-dlp …', createdAt: t, startedAt: t, finishedAt: null, log: [] } as unknown as Job;
  if (location.search.includes('paused')) setTimeout(() => emit('queue-paused', { until: Date.now() / 1000 + 14 * 60 + 20 }), 1200);
  jobs = [
    { ...base, id: 'a', title: 'Finished talk, downloaded and transcribed', status: 'done', percent: 100, finishedAt: t, files: ['/home/you/Downloads/talk.opus'], transcripts: ['/home/you/Downloads/talk.txt'], outputBytes: 10_612_815,
      timings: { wait: 4325, setup: 31, prepare: 3, download: 8, convert: 2, transcribe: 708, diarize: 1090 }, transcriptStats: { model: 'small', language: 'en', audioSecs: 3600, words: 10925, speakers: 3 } },
    { ...base, id: 'b', title: 'Transcribing right now', status: 'processing', percent: 42, message: 'Transcribing… 42%', phase: 'transcribe', phaseStarted: t - 95, timings: { prepare: 2, download: 12, convert: 3 }, files: ['/home/you/Downloads/talk2.opus'] },
    { ...base, id: 'c', title: 'Live stream being recorded', status: 'live', isLive: true, elapsed: 5400, downloaded: 301_000_000, phase: 'record', phaseStarted: t - 5400, timings: { wait: 640, prepare: 4 } },
  ];
}
let subs: Subscription[] = [];
let seq = 1;

const sleep = (ms: number) => new Promise(r => setTimeout(r, ms));

function sampleFormats() {
  const v = (id: string, h: number, ext: string, vc: string, size: number, fps = 30) =>
    ({ format_id: id, ext, height: h, width: Math.round(h * 16 / 9), resolution: `${Math.round(h * 16 / 9)}x${h}`, fps, vcodec: vc, acodec: 'none', filesize: size, tbr: size / 2000, format_note: `${h}p` });
  const a = (id: string, ext: string, ac: string, size: number) =>
    ({ format_id: id, ext, resolution: 'audio only', acodec: ac, vcodec: 'none', filesize: size, tbr: 128, format_note: 'medium' });
  return [
    a('139', 'm4a', 'mp4a', 4_100_000), a('140', 'm4a', 'mp4a', 10_800_000), a('251', 'webm', 'opus', 9_500_000),
    v('160', 144, 'mp4', 'avc1', 3_000_000), v('134', 360, 'mp4', 'avc1', 14_000_000), v('135', 480, 'mp4', 'avc1', 22_000_000),
    v('136', 720, 'mp4', 'avc1', 41_000_000), v('137', 1080, 'mp4', 'avc1', 88_000_000), v('248', 1080, 'webm', 'vp9', 62_000_000),
    v('271', 1440, 'webm', 'vp9', 160_000_000), v('313', 2160, 'webm', 'vp9', 380_000_000, 60),
  ];
}

async function probe(url: string) {
  await sleep(500);
  if (/bad|private/.test(url)) throw { message: 'This video is private.', hint: 'Only the owner can share it. If it was shared with you, use cookies from your signed-in browser.' };
  if (/upcoming|scheduled|GxCw7cN44fE/.test(url)) {
    return { title: 'Weekly Livestream — Episode 42', uploader: 'Example Channel', live_status: 'is_upcoming', release_timestamp: Date.now() / 1000 + 7800,
      thumbnail: 'https://i.ytimg.com/vi/GxCw7cN44fE/hqdefault.jpg', formats: [], subtitles: {} };
  }
  if (/live/.test(url)) {
    return { title: 'Lo-fi beats to relax to', uploader: 'Chill Radio', live_status: 'is_live', formats: sampleFormats(), subtitles: {} };
  }
  if (/playlist|list=/.test(url)) {
    return { _type: 'playlist', title: 'Conference talks 2026', uploader: 'Tech Conf',
      entries: Array.from({ length: 12 }, (_, i) => ({ id: `id${i}`, title: `Talk ${i + 1}: Building things that last`, duration: 900 + i * 173 })) };
  }
  return { title: 'Me at the zoo', uploader: 'jawed', duration: 19, live_status: 'not_live',
    thumbnail: 'https://i.ytimg.com/vi/jNQXAC9IVRw/hqdefault.jpg', formats: sampleFormats(), subtitles: { en: [{}], fr: [{}] } };
}

function simulate(job: Job) {
  const set = (p: Partial<Job>) => { Object.assign(job, p); emit('job-update', { ...job }); };
  if (job.options.waitForVideo) {
    set({ status: 'waiting', message: 'Waiting for 00:00:45 - Press Ctrl+C to try now' });
    let t = 45;
    const w = setInterval(() => {
      t -= 5;
      if (job.status === 'cancelled') { clearInterval(w); return; }
      if (t <= 0) {
        clearInterval(w);
        set({ status: 'live', isLive: true, message: null, percent: null });
        let el = 0;
        const l = setInterval(() => {
          if (job.status === 'cancelled') { clearInterval(l); return; }
          el += 1; set({ status: 'live', elapsed: el, downloaded: el * 900_000, speed: 900_000 });
        }, 1000);
      } else set({ message: `Waiting for 00:00:${String(t).padStart(2, '0')} - Press Ctrl+C to try now` });
    }, 1000);
    return;
  }
  set({ status: 'downloading', message: null, stream: 1 });
  let p = 0;
  const iv = setInterval(() => {
    if (job.status === 'cancelled') { clearInterval(iv); return; }
    p += 4 + Math.random() * 6;
    if (p >= 100) {
      clearInterval(iv);
      set({ status: 'processing', percent: null, message: 'Merging video and audio…', speed: null, eta: null });
      setTimeout(() => set({ status: 'done', percent: 100, message: null, timings: { prepare: 2, download: 38, convert: 4 }, outputBytes: 84_000_000, finishedAt: Date.now() / 1000, files: [`${job.options.outputDir}/${job.title ?? 'video'}.mp4`] }), 800);
      return;
    }
    set({ percent: p, downloaded: p * 1e6, total: 1e8, speed: 4_000_000 + Math.random() * 2e6, eta: Math.round((100 - p) / 6) });
  }, 400);
}

export async function call(cmd: string, a: Record<string, any> = {}): Promise<unknown> {
  switch (cmd) {
    case 'app_info': return { version: '0.1.0', buildNumber: '20261025.1200', buildTime: '2026-10-25 12:00 UTC', buildCommit: 'abc1234', os: 'linux', arch: 'x64', portable: false, dataDir: '/home/you/.local/share/ytgrab', binDir: '/home/you/.local/share/ytgrab/bin', defaultDownloadDir: '/home/you/Downloads', bins } satisfies AppInfo;
    case 'install_tools':
      for (const n of a.names as string[]) {
        for (let i = 0; i <= 10; i++) { emit('binary-progress', { name: n, stage: 'downloading', downloaded: i * 8e6, total: 8e7 }); await sleep(120); }
        bins = bins.map(b => b.name === n ? { name: n, path: `/app/bin/${n}`, version: n === 'deno' ? '2.9.7' : n === 'yt-dlp' ? '2026.08.19' : '6.1.1-static', source: 'managed' } : b);
        emit('binary-progress', { name: n, stage: 'done', downloaded: 0, total: null });
      }
      return bins;
    case 'queue_paused_until': return null;
    case 'resume_queue': return;
    case 'whisper_status': return whisper;
    case 'whisper_install_engine': {
      for (const stage of ['downloading', 'setting up Python', 'installing packages']) { emit('binary-progress', { name: 'whisper', stage, downloaded: 0, total: null }); await sleep(700); }
      whisper = { ...whisper, engineInstalled: true }; emit('binary-progress', { name: 'whisper', stage: 'done', downloaded: 0, total: null }); return whisper;
    }
    case 'whisper_remove_engine': whisper = { ...whisper, engineInstalled: false }; return whisper;
    case 'whisper_download_model': {
      const m = whisper.models.find(x => x.name === a.name)!; const total = m.sizeMb * 1048576;
      for (let i = 1; i <= 10; i++) { emit('binary-progress', { name: `model:${m.name}`, stage: 'downloading', downloaded: total * i / 10, total }); await sleep(250); }
      whisper = { engineInstalled: true, speakersInstalled: whisper.speakersInstalled, speakersMb: 36, dir: whisper.dir, models: whisper.models.map(x => x.name === a.name ? { ...x, installed: true } : x) }; return whisper;
    }
    case 'whisper_download_speakers': {
      for (let i = 1; i <= 8; i++) { emit('binary-progress', { name: 'speakers', stage: 'downloading', downloaded: 36e6 * i / 8, total: 36e6 }); await sleep(250); }
      whisper = { ...whisper, engineInstalled: true, speakersInstalled: true }; return whisper;
    }
    case 'whisper_delete_speakers': whisper = { ...whisper, speakersInstalled: false }; return whisper;
    case 'whisper_delete_model': whisper = { ...whisper, models: whisper.models.map(x => x.name === a.name ? { ...x, installed: false } : x) }; return whisper;
    case 'transcribe_job': {
      const j = jobs.find(x => x.id === a.id); if (!j) return;
      j.status = 'processing'; j.percent = 0; emit('job-update', { ...j });
      for (let i = 1; i <= 10; i++) { await sleep(300); j.percent = i * 10; j.message = `Transcribing… ${i * 10}%`; emit('job-update', { ...j }); }
      j.status = 'done'; j.message = null; j.transcripts = [j.files[0].replace(/\.\w+$/, '.txt')]; emit('job-update', { ...j }); return;
    }
    case 'transcribe_file': {
      const name = String(a.path).split('/').pop()!.replace(/\.\w+$/, '');
      const j: Job = { id: `m${seq++}`, url: a.path, title: name, thumbnail: null, options: { url: a.path, transcribe: a.options } as never,
        status: 'processing', percent: 0, speed: null, eta: null, downloaded: 0, total: null, elapsed: null, stream: 0, playlist: null, message: 'Transcribing… 0%', error: null, hint: null,
        files: [a.path], transcripts: [], transcriptError: null, timings: {}, phase: 'transcribe', phaseStarted: Date.now() / 1000, outputBytes: 0, transcriptStats: null, isLive: false, localFile: true, source: null,
        command: `Whisper transcript of ${a.path}`, createdAt: Date.now() / 1000, startedAt: Date.now() / 1000, finishedAt: null, log: [] };
      jobs = [...jobs, j]; emit('job-update', { ...j });
      (async () => {
        for (let i = 1; i <= 10; i++) { await sleep(300); j.percent = i * 10; j.message = `Transcribing… ${i * 10}%`; emit('job-update', { ...j }); }
        j.status = 'done'; j.message = null; j.transcripts = [a.path.replace(/\.\w+$/, '.txt')]; emit('job-update', { ...j });
      })();
      return j;
    }
    case 'latest_ytdlp_version': return '2026.09.20';
    case 'get_settings': return settings;
    case 'save_settings': settings = a.settings; return settings;
    case 'probe': return probe(a.req.url);
    case 'preview_command': {
      const o = a.options; const p = ['yt-dlp'];
      if (o.waitForVideo) p.push('--wait-for-video', `${o.waitForVideo.min}-${o.waitForVideo.max}`);
      if (o.liveFromStart) p.push('--live-from-start');
      if (o.audioOnly) p.push('-x', '--audio-format', o.audioFormat ?? 'mp3');
      if (o.oneAtATime) p.push('# one at a time');
      if (o.pauseBetween?.max) p.push('--sleep-interval', String(o.pauseBetween.min), '--max-sleep-interval', String(o.pauseBetween.max));
      if (o.format) p.push('-f', `'${o.format}'`);
      p.push('--', `'${o.url}'`); return p.join(' ');
    }
    case 'list_jobs': return jobs;
    case 'job_log_path': return '/home/you/.local/share/app.ytgrab.desktop/logs/' + a.id + '.log';
    case 'job_log': return ['[youtube] Extracting URL', '[info] Downloading 1 format(s): 395+140'];
    case 'add_job': {
      const j: Job = { id: `m${seq++}`, url: a.job.options.url, title: a.job.title ?? null, thumbnail: a.job.thumbnail ?? null, options: a.job.options,
        status: 'queued', percent: null, speed: null, eta: null, downloaded: 0, total: null, elapsed: null, stream: 0, playlist: null, message: null, error: null, hint: null,
        files: [], transcripts: [], transcriptError: null, timings: {}, phase: null, phaseStarted: null, outputBytes: 0, transcriptStats: null, isLive: !!a.job.isLive, source: a.job.source ?? null, command: 'yt-dlp …', createdAt: Date.now() / 1000, startedAt: null, finishedAt: null, log: [] };
      jobs = [...jobs, j]; emit('job-update', { ...j }); setTimeout(() => simulate(jobs.find(x => x.id === j.id)!), 300); return j;
    }
    case 'cancel_job': { const j = jobs.find(x => x.id === a.id); if (j) { j.status = 'cancelled'; j.message = 'Stopped'; emit('job-update', { ...j }); } return; }
    case 'retry_job': { const j = jobs.find(x => x.id === a.id); if (j) { j.status = 'queued'; j.error = null; j.hint = null; emit('job-update', { ...j }); setTimeout(() => simulate(j), 300); } return; }
    case 'remove_job': jobs = jobs.filter(j => j.id !== a.id); emit('job-removed', a.id); return;
    case 'clear_finished': jobs.filter(j => ['done', 'failed', 'cancelled'].includes(j.status)).forEach(j => emit('job-removed', j.id)); jobs = jobs.filter(j => !['done', 'failed', 'cancelled'].includes(j.status)); return;
    case 'list_subscriptions': return subs;
    case 'save_subscription': { const s = { ...a.subscription }; if (!s.id) s.id = `s${seq++}`; subs = subs.some(x => x.id === s.id) ? subs.map(x => x.id === s.id ? s : x) : [...subs, s]; return subs; }
    case 'delete_subscription': subs = subs.filter(s => s.id !== a.id); return subs;
    case 'check_subscription_now': { const s = subs.find(x => x.id === a.id); if (s) { s.lastCheck = Date.now() / 1000; s.lastNote = 'Checked — nothing new'; } return true; }
    default: return undefined;
  }
}

