import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

import {isCancelled} from './io.js';

/**
 * One long-lived subprocess whose stdout is read line by line.
 * communicate_utf8_async is the wrong call here: it resolves only when the
 * process exits, so a streaming tool would never deliver a sample.
 */
export class LineProcess {
    /**
     * @param {string[]} argv
     * @param {(line: string) => void} onLine
     * @param {{maxFailures?: number}} [options]
     */
    constructor(argv, onLine, options = {}) {
        this._argv = argv;
        this._onLine = onLine;
        this._maxFailures = options.maxFailures ?? Infinity;
        this._proc = null;
        this._stream = null;
        this._cancellable = null;
        this._restartId = 0;
        this._failures = 0;
        this._running = false;
        this._gaveUp = false;
    }

    get gaveUp() {
        return this._gaveUp;
    }

    start() {
        if (this._running || this._gaveUp)
            return;
        this._running = true;
        this._failures = 0;
        this._spawn();
    }

    stop() {
        this._running = false;
        this._clearRestart();
        this._shutdown();
    }

    _clearRestart() {
        if (this._restartId) {
            GLib.source_remove(this._restartId);
            this._restartId = 0;
        }
    }

    _shutdown() {
        this._cancellable?.cancel();
        this._cancellable = null;
        const proc = this._proc;
        this._proc = null;
        this._stream = null;
        if (!proc)
            return;
        try {
            proc.force_exit();
        } catch (error) {
            // Already exited.
        }
    }

    _spawn() {
        if (!this._running)
            return;
        this._shutdown();
        this._cancellable = new Gio.Cancellable();
        try {
            this._proc = Gio.Subprocess.new(
                this._argv,
                Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_SILENCE);
        } catch (error) {
            this._scheduleRestart();
            return;
        }
        this._stream = new Gio.DataInputStream({
            base_stream: this._proc.get_stdout_pipe(),
        });
        this._read(this._stream, this._cancellable);
    }

    /**
     * @param {Gio.DataInputStream} stream
     * @param {Gio.Cancellable} cancellable
     */
    async _read(stream, cancellable) {
        try {
            while (this._running && !cancellable.is_cancelled()) {
                const [line] = await stream.read_line_async(
                    GLib.PRIORITY_DEFAULT,
                    cancellable);
                if (line === null)
                    break;
                this._failures = 0;
                this._onLine(line);
            }
        } catch (error) {
            if (isCancelled(error) || !this._running)
                return;
        }
        if (this._running && !cancellable.is_cancelled())
            this._scheduleRestart();
    }

    _scheduleRestart() {
        if (!this._running)
            return;
        this._failures += 1;
        if (this._failures > this._maxFailures) {
            this._gaveUp = true;
            this.stop();
            return;
        }
        const delay = Math.min(30000, 1000 * (2 ** Math.min(this._failures - 1, 5)));
        this._restartId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, delay, () => {
            this._restartId = 0;
            this._spawn();
            return GLib.SOURCE_REMOVE;
        });
        GLib.Source.set_name_by_id(this._restartId, '[stats] subprocess restart');
    }
}
