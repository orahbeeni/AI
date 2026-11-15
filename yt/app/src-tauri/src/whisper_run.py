"""Runs inside YT Grab's private Whisper environment (faster-whisper, sherpa-onnx).

Commands, picked by the first argument:

  download MODEL OUT_DIR                  fetch a Whisper model into OUT_DIR
  download-speakers OUT_DIR               fetch the two small speaker models into OUT_DIR
  transcribe MODEL_DIR INPUT OUT_BASE FORMATS LANGUAGE INTERVAL THREADS SPEAKERS SPEAKER_DIR

SPEAKERS is "off", "auto" or a number (2-8). The app reads stdout line by line:

  PROGRESS|<percent>   LANG|<code>   DURATION|<seconds>   WORDS|<count>
  PHASE|diarize        SPEAKERS|<count>
  DONE|<file>;<file>   ERROR|<message>
"""
import hashlib
import json
import os
import sys
import tarfile
import urllib.request

os.environ.setdefault("PYTHONUTF8", "1")
os.environ.setdefault("HF_HUB_DISABLE_TELEMETRY", "1")
os.environ.setdefault("HF_HUB_DISABLE_SYMLINKS_WARNING", "1")

# Public GitHub release assets (no account needed). Both are checked against these hashes.
_BASE = "https://github.com/k2-fsa/sherpa-onnx/releases/download"
SEGMENTATION_URL = _BASE + "/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2"
SEGMENTATION_MEMBER = "sherpa-onnx-pyannote-segmentation-3-0/model.onnx"
SEGMENTATION_SHA256 = "220ad67ca923bef2fa91f2390c786097bf305bceb5e261d4af67b38e938e1079"
EMBEDDING_URL = _BASE + "/speaker-recongition-models/3dspeaker_speech_campplus_sv_en_voxceleb_16k.onnx"
EMBEDDING_SHA256 = "357a834f702b80161e5b981182c038e18553c1f2ca752ed6cec2052365d4129b"

# A voice that speaks less than this share of all speech (and under MIN_SECONDS) is treated as
# noise in the clustering and folded into the nearest real speaker. When the user says how many
# people are talking, the biggest voices are kept instead.
MIN_SHARE = 0.03
MIN_SECONDS = 5.0
# The clustering threshold: strict, because loose settings split one person into several.
# If the user expects more voices than were found, one retry with the loose setting.
THRESHOLD = 0.8
LOOSE_THRESHOLD = 0.5

# Memory. Whisper's voice detection and the speaker step both hold working copies of the audio, so a
# long recording needs gigabytes (measured: ~2.5 GB per audio hour for the transcript, ~1.2 GB per hour
# for speakers). So long files are transcribed in windows of about this many seconds, and speaker
# labels are skipped for recordings longer than DIARIZE_MAX_SECONDS instead of the whole run being
# killed by the system.
SAMPLE_RATE = 16000
WINDOW_SECONDS = 25 * 60
DIARIZE_MAX_SECONDS = 4 * 3600


def say(*parts):
    print("|".join(str(p) for p in parts), flush=True)


def clock(t, comma=False):
    ms = int(round(max(t, 0) * 1000))
    h, ms = divmod(ms, 3600000)
    m, ms = divmod(ms, 60000)
    s, ms = divmod(ms, 1000)
    return f"{h:02d}:{m:02d}:{s:02d}{',' if comma else '.'}{ms:03d}"


def stamp(t):
    return "[" + clock(t)[:8] + "]"


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def download(model, out_dir):
    from faster_whisper.utils import download_model

    download_model(model, output_dir=out_dir)
    say("DONE", out_dir)


def download_speakers(out_dir):
    os.makedirs(out_dir, exist_ok=True)
    archive = os.path.join(out_dir, "segmentation.tar.bz2")
    urllib.request.urlretrieve(SEGMENTATION_URL, archive)
    with tarfile.open(archive, "r:bz2") as tar:
        member = tar.getmember(SEGMENTATION_MEMBER)
        with tar.extractfile(member) as src, open(os.path.join(out_dir, "segmentation.onnx"), "wb") as dst:
            dst.write(src.read())
    os.remove(archive)
    urllib.request.urlretrieve(EMBEDDING_URL, os.path.join(out_dir, "embedding.onnx"))
    for name, want in (("segmentation.onnx", SEGMENTATION_SHA256), ("embedding.onnx", EMBEDDING_SHA256)):
        if sha256(os.path.join(out_dir, name)) != want:
            raise RuntimeError(f"{name} failed its integrity check")
    say("DONE", out_dir)


# ---- who spoke when ---------------------------------------------------------------------

def diarize(audio, speaker_dir, expected, threads):
    """Return [(start, end, speaker_id)] for the whole recording (16 kHz samples). `expected` is how
    many people the user says are talking (0 = unknown); it only decides whether to retry with a
    looser setting."""
    turns = _diarize_once(audio, speaker_dir, THRESHOLD, threads)
    if expected and len({t[2] for t in turns}) < expected:
        say("PROGRESS", 0)
        turns = _diarize_once(audio, speaker_dir, LOOSE_THRESHOLD, threads)
    return turns


def _diarize_once(audio, speaker_dir, threshold, threads):
    import sherpa_onnx

    config = sherpa_onnx.OfflineSpeakerDiarizationConfig(
        segmentation=sherpa_onnx.OfflineSpeakerSegmentationModelConfig(
            pyannote=sherpa_onnx.OfflineSpeakerSegmentationPyannoteModelConfig(model=os.path.join(speaker_dir, "segmentation.onnx")),
            num_threads=threads,
        ),
        embedding=sherpa_onnx.SpeakerEmbeddingExtractorConfig(model=os.path.join(speaker_dir, "embedding.onnx"), num_threads=threads),
        # Forcing a number of clusters did worse than letting the threshold decide (tested), so
        # the count is always worked out here and trimmed afterwards.
        clustering=sherpa_onnx.FastClusteringConfig(num_clusters=-1, threshold=threshold),
        min_duration_on=0.3,
        min_duration_off=0.5,
    )
    sd = sherpa_onnx.OfflineSpeakerDiarization(config)
    last = [-1]

    def progress(done, total):
        pct = int(done * 100 / max(total, 1))
        if pct != last[0]:
            last[0] = pct
            say("PROGRESS", min(pct, 99))
        return 0

    return [(r.start, r.end, r.speaker) for r in sd.process(audio, callback=progress).sort_by_start_time()]


def merge_small_speakers(turns, keep_top=0):
    """Fold minor voices into the closest-in-time real speaker. By default a voice must speak for
    a few percent of the time to count; with `keep_top` the biggest N voices are kept."""
    totals = {}
    for a, b, s in turns:
        totals[s] = totals.get(s, 0.0) + (b - a)
    if not totals:
        return turns
    everything = sum(totals.values())
    if keep_top:
        keep = set(sorted(totals, key=totals.get, reverse=True)[:keep_top])
    else:
        keep = {s for s, v in totals.items() if v >= max(MIN_SHARE * everything, MIN_SECONDS)}
    if not keep:
        keep = {max(totals, key=totals.get)}
    real = [t for t in turns if t[2] in keep]
    out = []
    for a, b, s in turns:
        if s in keep:
            out.append((a, b, s))
            continue
        mid = (a + b) / 2
        nearest = min(real, key=lambda t: 0 if t[0] <= mid <= t[1] else min(abs(mid - t[0]), abs(mid - t[1])))
        out.append((a, b, nearest[2]))
    return out


def assign_speakers(segments, turns):
    """Give each transcript segment the speaker who talks most during it; number speakers 1, 2, 3…
    in the order they first appear."""
    order = {}
    for seg in segments:
        best, best_overlap = None, 0.0
        for a, b, s in turns:
            overlap = min(seg["end"], b) - max(seg["start"], a)
            if overlap > best_overlap:
                best, best_overlap = s, overlap
        if best is None and turns:
            mid = (seg["start"] + seg["end"]) / 2
            best = min(turns, key=lambda t: min(abs(mid - t[0]), abs(mid - t[1])))[2]
        if best is not None:
            seg["speaker"] = order.setdefault(best, len(order) + 1)
    return len(order)


# ---- output ------------------------------------------------------------------------------

def paragraphs(segments, interval):
    """Group segments into paragraphs that each start with a [HH:MM:SS] stamp.

    A new paragraph starts once `interval` seconds have passed since the last stamp, and
    whenever the speaker changes, so any line can be found again in the original audio.
    """
    out, cur, mark = [], [], None
    for s in segments:
        changed = bool(cur) and s.get("speaker") != cur[-1].get("speaker")
        if mark is None or s["start"] >= mark or changed:
            if cur:
                out.append(cur)
            cur = [s]
            mark = s["start"] + interval
        else:
            cur.append(s)
    if cur:
        out.append(cur)
    return out


def about_line():
    """A header line with what the app knows about the video (upload date, channel, link), or nothing."""
    try:
        info = json.loads(os.environ.get("YTGRAB_ABOUT", "") or "{}")
    except ValueError:
        info = {}
    parts = []
    if info.get("uploaded"):
        parts.append(f"Uploaded {info['uploaded']}")
    if info.get("channel"):
        parts.append(f"Channel: {info['channel']}")
    if info.get("url"):
        parts.append(info["url"])
    return "# " + " · ".join(parts) + "\n" if parts else ""


def label(seg):
    return f"Speaker {seg['speaker']}: " if seg.get("speaker") else ""


def windows(total):
    """Split `total` samples into equal windows of about WINDOW_SECONDS; one window when it is short."""
    size = WINDOW_SECONDS * SAMPLE_RATE
    if total <= size * 1.25:
        return [(0, total)]
    n = -(-total // size)
    step = -(-total // n)
    return [(a, min(a + step, total)) for a in range(0, total, step)]


def transcribe(model_dir, src, out_base, formats, language, interval, threads, speakers, speaker_dir):
    from faster_whisper import WhisperModel
    from faster_whisper.audio import decode_audio

    model = WhisperModel(model_dir, device="cpu", compute_type="int8", cpu_threads=threads)
    audio = decode_audio(src, sampling_rate=SAMPLE_RATE)
    total = max(len(audio) / SAMPLE_RATE, 1.0)
    say("DURATION", f"{total:.1f}")
    lang = None if language in ("", "auto") else language
    done = []
    last_pct = -1
    for a, b in windows(len(audio)):
        offset = a / SAMPLE_RATE
        segs, info = model.transcribe(
            audio[a:b],
            language=lang,
            vad_filter=True,
            beam_size=1,
            condition_on_previous_text=False,
        )
        if lang is None:
            lang = info.language  # decided on the first window, kept for the rest
        if a == 0:
            say("LANG", lang)
        for s in segs:
            text = s.text.strip()
            if text:
                done.append({"start": s.start + offset, "end": s.end + offset, "text": text})
            pct = min(int((s.end + offset) / total * 100), 99)
            if pct != last_pct:
                last_pct = pct
                say("PROGRESS", pct)

    speaker_count = 0
    skipped = ""
    if speakers != "off" and done and total > DIARIZE_MAX_SECONDS:
        skipped = f" Speaker labels skipped: the recording is longer than {DIARIZE_MAX_SECONDS // 3600} hours."
        print(skipped.strip(), file=sys.stderr, flush=True)
    elif speakers != "off" and done:
        say("PHASE", "diarize")  # the progress bar starts over for this step
        wanted = int(speakers) if speakers.isdigit() else 0
        turns = merge_small_speakers(diarize(audio, speaker_dir, wanted, max(1, min(threads, 4))), wanted)
        speaker_count = assign_speakers(done, turns)
        say("SPEAKERS", speaker_count)
    del audio

    written = []
    name = os.path.basename(src)
    who = f" {speaker_count} speaker{'s' if speaker_count != 1 else ''} (approximate)." if speaker_count else skipped
    if "txt" in formats:
        path = out_base + ".txt"
        with open(path, "w", encoding="utf-8") as f:
            f.write(f"# {name}\n{about_line()}# Whisper transcript, language: {lang}.{who} "
                    f"[HH:MM:SS] marks are positions in the original file.\n\n")
            for para in paragraphs(done, interval):
                f.write(stamp(para[0]["start"]) + " " + label(para[0]) + " ".join(p["text"] for p in para) + "\n\n")
        written.append(path)
    if "srt" in formats:
        path = out_base + ".srt"
        with open(path, "w", encoding="utf-8") as f:
            for i, s in enumerate(done, 1):
                f.write(f"{i}\n{clock(s['start'], True)} --> {clock(s['end'], True)}\n{label(s)}{s['text']}\n\n")
        written.append(path)
    if "vtt" in formats:
        path = out_base + ".vtt"
        with open(path, "w", encoding="utf-8") as f:
            f.write("WEBVTT\n\n")
            for s in done:
                voice = f"<v Speaker {s['speaker']}>" if s.get("speaker") else ""
                f.write(f"{clock(s['start'])} --> {clock(s['end'])}\n{voice}{s['text']}\n\n")
        written.append(path)
    say("WORDS", sum(len(s["text"].split()) for s in done))
    say("PROGRESS", 100)
    say("DONE", ";".join(written))


def main(argv):
    try:
        if argv[1] == "download":
            download(argv[2], argv[3])
        elif argv[1] == "download-speakers":
            download_speakers(argv[2])
        elif argv[1] == "transcribe":
            model_dir, src, out_base, formats, language, interval, threads, speakers, speaker_dir = argv[2:11]
            transcribe(model_dir, src, out_base, formats.split(","), language, max(int(interval), 5), int(threads), speakers, speaker_dir)
        else:
            raise SystemExit(f"unknown command {argv[1]}")
    except SystemExit:
        raise
    except BaseException as e:  # report instead of dumping a traceback into the UI
        say("ERROR", f"{type(e).__name__}: {e}")
        sys.exit(1)


if __name__ == "__main__":
    main(sys.argv)
