// Local, length-prefixed transport. No HTTP listener, eval or arbitrary commands.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

const MAX_PACKET = 1024 * 1024;
const decoder = new TextDecoder();
const encoder = new TextEncoder();

export class Peer {
    constructor(connection, onPacket, onClose) {
        this.connection = connection;
        this.input = connection.get_input_stream();
        this.output = connection.get_output_stream();
        this.cancel = new Gio.Cancellable();
        this.onPacket = onPacket;
        this.onClose = onClose;
        this.closed = false;
        this.closing = false;
        this.queue = [];
        this.writing = false;
        this.lastSeen = GLib.get_monotonic_time();
    }

    async read(size) {
        const result = new Uint8Array(size);
        let offset = 0;
        while (offset < size) {
            const bytes = await new Promise((resolve, reject) => {
                this.input.read_bytes_async(size - offset, GLib.PRIORITY_DEFAULT, this.cancel, (stream, response) => {
                    try { resolve(stream.read_bytes_finish(response)); } catch (e) { reject(e); }
                });
            });
            const chunk = bytes.get_data();
            if (!chunk.length) throw new Error('Sidera disconnected');
            result.set(chunk, offset);
            offset += chunk.length;
        }
        return result;
    }

    async start() {
        try {
            while (!this.closed && !this.closing) {
                const header = await this.read(4);
                const size = new DataView(header.buffer).getUint32(0, false);
                if (size < 1 || size > MAX_PACKET) throw new Error('Invalid Sidera packet size');
                const packet = await this.read(size);
                this.lastSeen = GLib.get_monotonic_time();
                if (packet[0] === 1) {
                    if (size > 65536) throw new Error('Sidera control message too large');
                    this.onPacket(JSON.parse(decoder.decode(packet.subarray(1))), null);
                } else if (packet[0] === 2) {
                    this.onPacket(null, packet.subarray(1));
                } else {
                    throw new Error('Unknown Sidera packet type');
                }
            }
        } catch (e) {
            if (!this.closed && !this.closing) {
                if (e.message === 'Sidera disconnected' || e.matches?.(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED)) {
                    this.close(); return;
                }
                console.error(`Sidera bridge: ${e.stack ?? e}`);
                this.fail(`GNOME 扩展通信失败：${String(e.message ?? e).slice(0, 500)}`);
            }
        }
    }

    send(value) {
        if (this.closed || this.closing) return;
        const body = encoder.encode(JSON.stringify(value));
        if (body.length > 65535 || this.queue.length > 256) {
            this.close();
            return;
        }
        const packet = new Uint8Array(body.length + 5);
        new DataView(packet.buffer).setUint32(0, body.length + 1, false);
        packet[4] = 1;
        packet.set(body, 5);
        // Coalesce unsent motion events, preserving every press/release/touch edge.
        if (value.type === 'motion' && this.queue.at(-1)?.motion === value.id)
            this.queue.pop();
        this.queue.push({packet, motion: value.type === 'motion' ? value.id : null});
        this.writeNext();
    }

    writeNext() {
        if (this.closed || this.writing) return;
        if (!this.queue.length) {
            if (this.closing) this.close();
            return;
        }
        this.writing = true;
        const {packet} = this.queue.shift();
        try {
            this.output.write_all_async(packet, GLib.PRIORITY_DEFAULT, this.cancel, (stream, response) => {
                this.writing = false;
                try {
                    stream.write_all_finish(response);
                    this.writeNext();
                } catch (_) { this.close(); }
            });
        } catch (_) { this.writing = false; this.close(); }
    }

    fail(message) {
        if (this.closed || this.closing) return;
        this.queue = [];
        this.send({type: 'error', fatal: true, message});
        this.closing = true;
        // Close after the fatal message is written, not before the client reads it.
    }

    close() {
        if (this.closed) return;
        this.closed = true;
        this.cancel.cancel();
        this.queue = [];
        try { this.connection.close(null); } catch (_) { /* Already closed. */ }
        this.onClose();
    }
}

export function listen(onConnection, filename = 'bridge.sock') {
    const directory = GLib.build_filenamev([GLib.get_user_runtime_dir(), 'sidera-gnome']);
    GLib.mkdir_with_parents(directory, 0o700);
    const root = Gio.File.new_for_path(directory);
    const info = root.query_info('standard::type,unix::uid,unix::mode', Gio.FileQueryInfoFlags.NOFOLLOW_SYMLINKS, null);
    const uid = new Gio.Credentials().get_unix_user();
    if (info.get_file_type() !== Gio.FileType.DIRECTORY || info.get_attribute_uint32('unix::uid') !== uid)
        throw new Error('Unsafe Sidera runtime directory');
    root.set_attribute_uint32('unix::mode', 0o700, Gio.FileQueryInfoFlags.NOFOLLOW_SYMLINKS, null);
    const path = GLib.build_filenamev([directory, filename]);
    const file = Gio.File.new_for_path(path);
    try { file.delete(null); } catch (e) {
        if (!e.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.NOT_FOUND)) throw e;
    }
    const service = new Gio.SocketService();
    service.add_address(Gio.UnixSocketAddress.new(path), Gio.SocketType.STREAM, Gio.SocketProtocol.DEFAULT, null);
    file.set_attribute_uint32('unix::mode', 0o600, Gio.FileQueryInfoFlags.NOFOLLOW_SYMLINKS, null);
    service.connect('incoming', (_service, connection) => {
        try {
            if (connection.get_socket().get_credentials().get_unix_user() !== uid)
                throw new Error('Different user');
            onConnection(connection);
        } catch (_) { connection.close(null); }
        return true;
    });
    service.start();
    return () => {
        service.stop(); service.close();
        try { file.delete(null); } catch (_) { /* Removed already. */ }
    };
}
