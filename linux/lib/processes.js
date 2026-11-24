import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

import {isCancelled, listChildren, readTextOrNull} from './io.js';
import {logWarn} from './log.js';
import {groupProcesses, parseStatus} from './overview.js';
import {cpuShare, machineShare, parseProcessStat} from './pressure.js';

const INTERVAL_SECONDS = 3;
const READ_BATCH = 24;

/**
 * Reads the user's own processes for the dashboard's Programs card. It runs
 * only while the dashboard is open (start on open, stop on close), every 3 s,
 * reading /proc in small asynchronous batches so the shell never stalls.
 */
export class ProcessSampler {
    /**
     * @param {(groups: ReturnType<typeof groupProcesses>) => void} onUpdate
     */
    constructor(onUpdate) {
        this._onUpdate = onUpdate;
        this._timerId = 0;
        this._cancellable = null;
        this._scanning = false;
        this._previousTicks = new Map();
        this._previousUs = 0;
        this._self = null;
        this._cores = 1;
    }

    /**
     * @param {number} cores CPU count, to express CPU as a share of the machine.
     */
    start(cores) {
        this._cores = cores > 0 ? cores : 1;
        if (this._timerId)
            return;
        this._cancellable = new Gio.Cancellable();
        this._scan();
        this._timerId = GLib.timeout_add_seconds(GLib.PRIORITY_LOW, INTERVAL_SECONDS, () => {
            this._scan();
            return GLib.SOURCE_CONTINUE;
        });
        GLib.Source.set_name_by_id(this._timerId, '[stats] program list');
    }

    stop() {
        if (this._timerId) {
            GLib.source_remove(this._timerId);
            this._timerId = 0;
        }
        this._cancellable?.cancel();
        this._cancellable = null;
        // CPU shares need two samples a moment apart; start fresh next time.
        this._previousTicks = new Map();
        this._previousUs = 0;
    }

    destroy() {
        this.stop();
        this._onUpdate = null;
    }

    async _identity(cancellable) {
        if (!this._self) {
            const own = parseStatus(await readTextOrNull('/proc/self/status', cancellable));
            this._self = {uid: own?.uid ?? null, pid: own?.pid ?? null};
        }
        return this._self;
    }

    async _scan() {
        if (this._scanning || !this._cancellable)
            return;
        this._scanning = true;
        const cancellable = this._cancellable;
        try {
            const self = await this._identity(cancellable);
            const now = GLib.get_monotonic_time();
            const elapsed = this._previousUs > 0 ? (now - this._previousUs) / 1e6 : 0;
            const pids = (await listChildren('/proc', cancellable)).filter(name => /^\d+$/.test(name));
            const processes = [];
            const nextTicks = new Map();
            for (let index = 0; index < pids.length; index += READ_BATCH) {
                const batch = pids.slice(index, index + READ_BATCH);
                const rows = await Promise.all(batch.map(async pid => {
                    const [stat, statusText] = await Promise.all([
                        readTextOrNull(`/proc/${pid}/stat`, cancellable),
                        readTextOrNull(`/proc/${pid}/status`, cancellable),
                    ]);
                    return {pid: Number(pid), stat, status: parseStatus(statusText)};
                }));
                for (const row of rows) {
                    // Only our own processes, with memory (not kernel threads),
                    // and never the shell we are running in.
                    if (!row.status || row.status.rss == null || row.pid === self.pid)
                        continue;
                    if (self.uid != null && row.status.uid !== self.uid)
                        continue;
                    const parsed = row.stat ? parseProcessStat(row.stat) : null;
                    let cpu = null;
                    if (parsed) {
                        nextTicks.set(row.pid, parsed.ticks);
                        const previous = this._previousTicks.get(row.pid);
                        if (previous != null)
                            cpu = machineShare(cpuShare(previous, parsed.ticks, elapsed), this._cores);
                    }
                    processes.push({pid: row.pid, comm: parsed?.comm ?? row.status.name, rss: row.status.rss, cpu});
                }
            }
            this._previousTicks = nextTicks;
            this._previousUs = now;
            if (!cancellable.is_cancelled())
                this._onUpdate?.(groupProcesses(processes));
        } catch (error) {
            if (!isCancelled(error))
                logWarn(`program list: ${error.message}`);
        } finally {
            this._scanning = false;
        }
    }
}

/**
 * Send a signal to processes with the system `kill`: TERM asks a program to
 * quit (as at log-out), KILL ends it at once.
 *
 * @param {number[]} pids
 * @param {'TERM'|'KILL'} signal
 * @returns {Promise<boolean>} Whether `kill` reported success.
 */
export function signalProcesses(pids, signal) {
    const ids = pids.filter(pid => Number.isInteger(pid) && pid > 1).map(String);
    if (ids.length === 0)
        return Promise.resolve(false);
    return new Promise(resolve => {
        try {
            const proc = Gio.Subprocess.new(
                ['kill', `-${signal}`, '--', ...ids],
                Gio.SubprocessFlags.STDOUT_SILENCE | Gio.SubprocessFlags.STDERR_SILENCE);
            proc.wait_check_async(null, (source, result) => {
                try {
                    resolve(source.wait_check_finish(result));
                } catch {
                    // Some pids may already be gone; the caller checks.
                    resolve(false);
                }
            });
        } catch (error) {
            logWarn(`kill -${signal}: ${error.message}`);
            resolve(false);
        }
    });
}

/**
 * Which of `pids` are still running (not gone and not zombies).
 *
 * @param {number[]} pids
 * @returns {Promise<number[]>}
 */
export async function stillRunning(pids) {
    const alive = await Promise.all(pids.map(async pid => {
        const stat = await readTextOrNull(`/proc/${pid}/stat`, null);
        if (!stat)
            return null;
        const state = stat.slice(stat.lastIndexOf(')') + 1).trim().charAt(0);
        return state === 'Z' || state === 'X' ? null : pid;
    }));
    return alive.filter(pid => pid != null);
}
