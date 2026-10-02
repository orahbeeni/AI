import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

Gio._promisify(Gio.Subprocess.prototype, 'wait_async');

export default class ScreenshotAnnotate extends Extension {
    enable() {
        this._stateFile = GLib.build_filenamev(
            [GLib.get_user_config_dir(), 'screenshot-annotator', 'state.json']);
        this._state = this._loadState();
        this._recorders = [];
        this._job = null;

        const ui = Main.screenshotUI;
        this._annotateBtn = this._makeToggle('document-edit-symbolic', 'Annotate after capture', 'annotate');
        this._sysBtn = this._makeToggle('audio-speakers-symbolic', 'Record system audio', 'system');
        this._micBtn = this._makeToggle('audio-input-microphone-symbolic', 'Record microphone', 'mic');
        for (const b of [this._micBtn, this._sysBtn, this._annotateBtn])
            ui._showPointerButtonContainer.insert_child_at_index(b, 0);

        // Annotate is for screenshots, the audio toggles are for screencasts.
        const syncMode = () => {
            const cast = ui._castButton.checked;
            this._annotateBtn.visible = !cast;
            this._sysBtn.visible = this._micBtn.visible = cast;
        };
        ui._castButton.connectObject('notify::checked', syncMode, this);
        syncMode();

        ui.connectObject(
            'screenshot-taken', (_ui, file) => {
                if (this._annotateBtn.checked && ui._currentMode !== 'SCREENSHOT_ONLY')
                    this._launchAnnotator(file);
            },
            'notify::screencast-in-progress', () => this._onScreencastState(ui), this);

        // After the stock recorder has stopped and finalised the video, add the audio.
        this._origStop = ui.stopScreencast;
        const self = this;
        ui.stopScreencast = async function (...args) {
            await self._origStop.apply(this, args);
            await self._finishAudio(this._screencastPath);
        };
    }

    disable() {
        const ui = Main.screenshotUI;
        ui.disconnectObject(this);
        ui._castButton.disconnectObject(this);
        if (this._origStop) {
            ui.stopScreencast = this._origStop;
            this._origStop = null;
        }
        this._killRecorders();
        for (const b of [this._annotateBtn, this._sysBtn, this._micBtn])
            b?.destroy();
        this._annotateBtn = this._sysBtn = this._micBtn = null;
    }

    // ---- UI helpers
    _makeToggle(icon, name, key) {
        const b = new St.Button({
            style_class: 'screenshot-annotate-button',
            toggle_mode: true,
            checked: this._state[key] === true,
            accessible_name: name,
            can_focus: true,
            y_align: Clutter.ActorAlign.CENTER,
            child: new St.Icon({icon_name: icon}),
        });
        b.connect('notify::checked', () => {
            this._state[key] = b.checked;
            this._saveState();
        });
        return b;
    }

    // ---- screenshot annotation
    _launchAnnotator(file) {
        const bin = GLib.build_filenamev([GLib.get_home_dir(), '.local', 'bin', 'screenshot-annotator']);
        try {
            Gio.Subprocess.new([bin, file.get_path()], Gio.SubprocessFlags.NONE);
        } catch (e) {
            logError(e, 'screenshot-annotate: failed to launch annotator');
            Main.notify('Screenshot Annotate', `Could not launch ${bin}`);
        }
    }

    // ---- screencast audio
    _onScreencastState(ui) {
        if (ui.screencast_in_progress) {
            if (!this._sysBtn.checked && !this._micBtn.checked)
                return;
            // Wait until the recorder service has actually started the video.
            const id = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 50, () => {
                if (ui._screencastStarting)
                    return GLib.SOURCE_CONTINUE;
                if (ui.screencast_in_progress)
                    this._startRecorders();
                return GLib.SOURCE_REMOVE;
            });
            this._startWait = id;
        } else {
            this._stopRecorders();
        }
    }

    _startRecorders() {
        const stamp = GLib.DateTime.new_now_local().format('%Y%m%d-%H%M%S');
        const tmp = GLib.get_tmp_dir();
        const specs = [];
        if (this._sysBtn.checked)
            specs.push({kind: 'sys', sink: 'true'});
        if (this._micBtn.checked)
            specs.push({kind: 'mic', sink: 'false'});
        for (const s of specs) {
            s.file = GLib.build_filenamev([tmp, `screenshot-annotate-${stamp}-${s.kind}.wav`]);
            try {
                s.proc = Gio.Subprocess.new(
                    ['pw-record', '--rate', '48000', '-P', `{ stream.capture.sink = ${s.sink} }`, s.file],
                    Gio.SubprocessFlags.NONE);
                this._recorders.push(s);
            } catch (e) {
                logError(e, `screenshot-annotate: cannot start ${s.kind} recording`);
                Main.notify('Screenshot Annotate', 'Could not start audio recording (is pw-record installed?)');
            }
        }
    }

    // Stops capture (SIGINT lets pw-record finalise the wav) and parks the files for muxing.
    _stopRecorders() {
        if (this._startWait) {
            GLib.source_remove(this._startWait);
            this._startWait = null;
        }
        if (!this._recorders.length)
            return;
        const job = this._recorders;
        this._recorders = [];
        for (const s of job)
            s.proc.send_signal(2);
        this._job = job;
        // If the recording never produced a video (startup failure) nobody claims the job.
        this._cleanupId = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 30, () => {
            this._discard(this._job);
            this._job = null;
            return GLib.SOURCE_REMOVE;
        });
    }

    _killRecorders() {
        for (const s of this._recorders)
            s.proc.force_exit();
        this._recorders = [];
        this._discard(this._job);
        this._job = null;
    }

    _discard(job) {
        for (const s of job ?? []) {
            try {
                Gio.File.new_for_path(s.file).delete(null);
            } catch { /* already gone */ }
        }
    }

    async _finishAudio(videoPath) {
        const job = this._job;
        this._job = null;
        if (this._cleanupId) {
            GLib.source_remove(this._cleanupId);
            this._cleanupId = null;
        }
        if (!job || !videoPath)
            return;

        try {
            await Promise.all(job.map(s => s.proc.wait_async(null)));

            const ext = videoPath.split('.').pop();
            const out = `${videoPath}.with-audio.${ext}`;
            const args = ['ffmpeg', '-y', '-loglevel', 'error', '-i', videoPath];
            for (const s of job)
                args.push('-i', s.file);
            if (job.length === 2) {
                args.push('-filter_complex', '[1:a][2:a]amix=inputs=2:duration=longest:normalize=0[a]',
                    '-map', '0:v', '-map', '[a]');
            } else {
                args.push('-map', '0:v', '-map', '1:a');
            }
            args.push('-c:v', 'copy', '-c:a', ext === 'webm' ? 'libopus' : 'aac', '-shortest', out);

            const ff = Gio.Subprocess.new(args, Gio.SubprocessFlags.STDERR_PIPE);
            await ff.wait_async(null);
            if (!ff.get_successful())
                throw new Error('ffmpeg failed');

            Gio.File.new_for_path(out).move(Gio.File.new_for_path(videoPath),
                Gio.FileCopyFlags.OVERWRITE, null, null);
            Main.notify('Screencast recorded', 'Audio added to the video');
        } catch (e) {
            logError(e, 'screenshot-annotate: adding audio failed');
            Main.notify('Screenshot Annotate', 'Could not add audio; the video was kept without it');
        } finally {
            this._discard(job);
        }
    }

    // ---- persisted toggle state
    _loadState() {
        try {
            const [ok, bytes] = GLib.file_get_contents(this._stateFile);
            return ok ? JSON.parse(new TextDecoder().decode(bytes)) : {};
        } catch {
            return {};
        }
    }

    _saveState() {
        try {
            GLib.mkdir_with_parents(GLib.path_get_dirname(this._stateFile), 0o755);
            GLib.file_set_contents(this._stateFile, JSON.stringify(this._state));
        } catch (e) {
            logError(e, 'screenshot-annotate: cannot save state');
        }
    }
}
