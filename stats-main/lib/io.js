import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

/**
 * GJS 1.88 does not promisify Gio.File async methods until Gio._promisify runs.
 * Gnome-shell does not do this for load_contents / enumerate_children, so the
 * extension does it once. Calling it again is safe.
 */
let prepared = false;

export function ensureIo() {
    if (prepared)
        return;
    Gio._promisify(Gio.File.prototype, 'load_contents_async', 'load_contents_finish');
    Gio._promisify(
        Gio.File.prototype,
        'query_filesystem_info_async',
        'query_filesystem_info_finish');
    Gio._promisify(
        Gio.File.prototype,
        'enumerate_children_async',
        'enumerate_children_finish');
    Gio._promisify(
        Gio.FileEnumerator.prototype,
        'next_files_async',
        'next_files_finish');
    Gio._promisify(
        Gio.DataInputStream.prototype,
        'read_line_async',
        'read_line_finish_utf8');
    prepared = true;
}

/**
 * @param {Error} error
 * @returns {boolean}
 */
export function isCancelled(error) {
    return !!error?.matches?.(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED);
}

/**
 * @param {string} path
 * @param {Gio.Cancellable} cancellable
 * @returns {Promise<string>}
 */
export async function readText(path, cancellable) {
    ensureIo();
    const file = Gio.File.new_for_path(path);
    const [contents] = await file.load_contents_async(cancellable);
    return new TextDecoder().decode(contents);
}

/**
 * @param {string} path
 * @param {Gio.Cancellable} cancellable
 * @returns {Promise<string|null>}
 */
export async function readTextOrNull(path, cancellable) {
    try {
        return await readText(path, cancellable);
    } catch (error) {
        if (isCancelled(error))
            throw error;
        return null;
    }
}

/**
 * @param {string} path
 * @param {Gio.Cancellable} cancellable
 * @returns {Promise<string[]>}
 */
export async function listChildren(path, cancellable) {
    ensureIo();
    const directory = Gio.File.new_for_path(path);
    const enumerator = await directory.enumerate_children_async(
        'standard::name',
        Gio.FileQueryInfoFlags.NONE,
        GLib.PRIORITY_DEFAULT,
        cancellable);
    const names = [];
    try {
        while (true) {
            const batch = await enumerator.next_files_async(
                64,
                GLib.PRIORITY_DEFAULT,
                cancellable);
            if (batch.length === 0)
                break;
            for (const info of batch)
                names.push(info.get_name());
        }
    } finally {
        enumerator.close(null);
    }
    return names;
}

/**
 * @param {string} path
 * @param {string} attributes
 * @param {Gio.Cancellable} cancellable
 * @returns {Promise<Gio.FileInfo>}
 */
export async function queryFilesystem(path, attributes, cancellable) {
    ensureIo();
    const file = Gio.File.new_for_path(path);
    return file.query_filesystem_info_async(
        attributes,
        GLib.PRIORITY_DEFAULT,
        cancellable);
}
