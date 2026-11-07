import Clutter from 'gi://Clutter';
import Cogl from 'gi://Cogl';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Config from 'resource:///org/gnome/shell/misc/config.js';
import {Peer, listen} from './bridge.js';
import {chromeParameters} from './chrome.js';
import {keyText} from './keyboard.js';
import {isWpsPresentation} from './presentation.js';

const TILE = 256;
const MAX_PIXELS = 67108864;
const backend = () => global.stage.context?.get_backend() ?? Clutter.get_default_backend();
const shellMajor = Number.parseInt(Config.PACKAGE_VERSION, 10);

function removeChromeActor(actor) {
    try { Main.layoutManager.removeChrome(actor); }
    finally { actor.destroy(); }
}

function addChromeActor(actor, input = false) {
    try { Main.layoutManager.addTopChrome(actor, chromeParameters(shellMajor, input)); }
    catch (error) {
        // addTopChrome attaches the actor before validating its parameters.
        // Never leave that partially added actor on the desktop after failure.
        try { removeChromeActor(actor); } catch (_) { /* Best effort cleanup. */ }
        throw error;
    }
}

// Visual tiles never become reactive or get unmapped when switching tools.
// Only the separate transparent input actors change their clickable area.
class Output {
    constructor(owner, monitor) {
        this.owner = owner;
        this.monitor = monitor;
        this.tiles = new Map();
        this.pending = null;
        this.inputs = [];
        this.buttons = new Set();
        this.touches = new Set();
        this.pointerGrab = null;
        this.grabRevokedId = 0;
        this.rects = [];
        this.inputDirty = true;
        this.ready = false;
        this.textInputOn = false;
        this.destroyed = false;
        this.root = new St.Widget({reactive: false, x: monitor.x, y: monitor.y,
            width: monitor.width, height: monitor.height, visible: false});
        this.root.set_clip(0, 0, monitor.width, monitor.height);
        addChromeActor(this.root);
    }

    begin(width, height) {
        if (!Number.isInteger(width) || !Number.isInteger(height) || width < 1 || height < 1 ||
            width * height > MAX_PIXELS || width !== Math.round(this.monitor.width * this.monitor.scale) ||
            height !== Math.round(this.monitor.height * this.monitor.scale))
            throw new Error('Invalid Sidera frame geometry');
        this.pending = {width, height, tiles: new Map()};
    }

    tile(packet) {
        if (!this.pending || packet.length < 32) throw new Error('Tile outside frame');
        const view = new DataView(packet.buffer, packet.byteOffset, packet.byteLength);
        const [width, height, x, y, w, h] = [8, 12, 16, 20, 24, 28].map(n => view.getUint32(n, false));
        if (width !== this.pending.width || height !== this.pending.height ||
            x % TILE || y % TILE || x >= width || y >= height ||
            w !== Math.min(TILE, width - x) || h !== Math.min(TILE, height - y) ||
            packet.length !== 32 + w * h * 4)
            throw new Error('Invalid Sidera tile');
        this.pending.tiles.set(`${x}:${y}`, {x, y, w, h, bytes: new GLib.Bytes(packet.subarray(32))});
    }

    end() {
        const frame = this.pending;
        if (!frame) throw new Error('Sidera frame end without begin');
        this.pending = null;
        for (const [key, tile] of frame.tiles) {
            let actor = this.tiles.get(key);
            if (!actor) {
                actor = new Clutter.Actor({reactive: false});
                this.tiles.set(key, actor);
                this.root.add_child(actor);
            }
            const content = St.ImageContent.new_with_preferred_size(tile.w, tile.h);
            // GNOME 48 added the Cogl-context argument (see upstream screenshot.js).
            const args = [tile.bytes, Cogl.PixelFormat.RGBA_8888_PRE, tile.w, tile.h, tile.w * 4];
            if (shellMajor < 48)
                content.set_bytes(...args);
            else
                content.set_bytes(backend().get_cogl_context(), ...args);
            const scale = this.monitor.scale;
            actor.set_position(tile.x / scale, tile.y / scale);
            actor.set_size(tile.w / scale, tile.h / scale);
            actor.set_content(content);
        }
        this.ready = true;
        this.visibility();
    }

    input(rects, full) {
        if (!Array.isArray(rects) || rects.length > 32) throw new Error('Invalid input region');
        const {width, height} = this.monitor;
        const bounded = [];
        for (const r of rects) {
            if (!Array.isArray(r) || r.length !== 4 || !r.every(Number.isFinite))
                throw new Error('Invalid input rectangle');
            const x = Math.max(0, Math.min(width, r[0]));
            const y = Math.max(0, Math.min(height, r[1]));
            const w = Math.min(width - x, Math.max(0, r[2]));
            const h = Math.min(height - y, Math.max(0, r[3]));
            if (w && h) bounded.push([x, y, w, h]);
        }
        const next = full === true ? [[0, 0, width, height]] : bounded;
        if (JSON.stringify(next) === JSON.stringify(this.rects)) return;
        this.rects = next;
        this.inputDirty = true;
        this.applyInput();
    }

    clearInput() {
        this.releasePointer();
        const inputs = this.inputs;
        if (inputs.includes(global.stage.get_key_focus())) global.stage.set_key_focus(null);
        this.inputs = [];
        this.inputDirty = true;
        for (const actor of inputs) {
            try { removeChromeActor(actor); }
            catch (error) { console.error(`Sidera input cleanup: ${error.stack ?? error}`); }
        }
    }

    applyInput() {
        if (this.destroyed) return;
        // Keep the grabbed actor and touch sequence targets alive until release.
        if (this.buttons.size || this.touches.size) return;
        if (!this.inputDirty) return;
        this.clearInput();
        if (!this.ready || !this.owner.active()) return;
        for (const [x, y, width, height] of this.rects) {
            const actor = new St.Widget({reactive: true, can_focus: this.textInputOn,
                x: this.monitor.x + x, y: this.monitor.y + y, width, height});
            actor.connect('event', (_actor, event) => this.event(actor, event));
            addChromeActor(actor, true);
            this.inputs.push(actor);
        }
        this.inputDirty = false;
        if (this.textInputOn) this.inputs[0]?.grab_key_focus();
    }

    textInput(enabled) {
        this.textInputOn = enabled === true;
        for (const actor of this.inputs) actor.can_focus = this.textInputOn;
        if (this.textInputOn) this.inputs[0]?.grab_key_focus();
        else if (this.inputs.includes(global.stage.get_key_focus())) global.stage.set_key_focus(null);
    }

    grabPointer(actor) {
        if (this.pointerGrab) return true;
        try {
            // Reactive chrome only receives events over its input region.
            // Hold a stage grab so dragging continues outside that region and
            // over the panel/dock/other monitors. Do not change keyboard focus
            // or Main.modalCount; this is a held pointer, not a modal dialog.
            const grab = global.stage.grab(actor);
            if (!grab) throw new Error('Could not capture pointer');
            this.pointerGrab = grab;
            this.grabRevokedId = grab.connect('notify::revoked', () => {
                if (!grab.is_revoked()) return;
                // Avoid destroying an actor during Clutter's grab-stack update.
                // Once another Shell grab interrupts this one, never resume the
                // stale drag when that dialog/menu closes.
                this.owner.later(() => {
                    if (this.pointerGrab !== grab) return;
                    this.cancel();
                    this.applyInput();
                });
            });
            return true;
        } catch (error) {
            console.error(`Sidera pointer capture: ${error.stack ?? error}`);
            this.cancel();
            this.owner.later(() => this.applyInput());
            return false;
        }
    }

    releasePointer() {
        const grab = this.pointerGrab;
        this.pointerGrab = null;
        if (!grab) return;
        if (this.grabRevokedId) grab.disconnect(this.grabRevokedId);
        this.grabRevokedId = 0;
        grab.dismiss();
    }

    event(actor, event) {
        if (this.destroyed || !this.owner.active()) {
            if (this.pointerGrab) this.cancel();
            return Clutter.EVENT_PROPAGATE;
        }
        const kind = event.type();
        if (kind === Clutter.EventType.KEY_PRESS && this.textInputOn) {
            const sym = event.get_key_symbol();
            const ctrl = (event.get_state() & Clutter.ModifierType.CONTROL_MASK) !== 0;
            const names = new Map([[0xff0d, 'enter'], [0xff8d, 'enter'], [0xff1b, 'escape'],
                [0xff08, 'backspace'], [0xffff, 'delete'], [0xff51, 'left'], [0xff53, 'right'],
                [0xff50, 'home'], [0xff57, 'end']]);
            let key = names.get(sym);
            if (ctrl && (sym === 65 || sym === 97)) key = 'select_all';
            if (ctrl && (sym === 86 || sym === 118)) key = 'paste';
            const text = !ctrl && !key ? keyText(event.get_key_unicode(), sym) : '';
            if (key || text) this.owner.peer?.send({type: 'value_key', generation: this.owner.generation,
                id: this.monitor.id, key: key ?? '', text});
            return Clutter.EVENT_STOP;
        }
        // Clutter supplies real touch events as well as optional emulated mouse
        // events. Processing both would draw and activate buttons twice.
        if (event.is_pointer_emulated?.()) return Clutter.EVENT_STOP;
        if (kind === Clutter.EventType.MOTION) this.reconcileButtons(event.get_state());
        const [sx, sy] = event.get_coords();
        const x = Math.round(sx - this.monitor.x), y = Math.round(sy - this.monitor.y);
        const value = {generation: this.owner.generation, id: this.monitor.id, x, y};
        switch (kind) {
        case Clutter.EventType.ENTER:
        case Clutter.EventType.MOTION:
            value.type = 'motion'; break;
        case Clutter.EventType.LEAVE:
            value.type = 'leave'; break;
        case Clutter.EventType.SCROLL: {
            const direction = event.get_scroll_direction();
            let delta = 0;
            if (direction === Clutter.ScrollDirection.UP) delta = -1;
            else if (direction === Clutter.ScrollDirection.DOWN) delta = 1;
            else if (direction === Clutter.ScrollDirection.SMOOTH) {
                const [, dy] = event.get_scroll_delta();
                delta = dy;
            }
            if (!Number.isFinite(delta) || delta === 0) return Clutter.EVENT_STOP;
            value.type = 'scroll'; value.delta = Math.max(-8, Math.min(8, delta));
            break;
        }
        case Clutter.EventType.BUTTON_PRESS:
            value.type = 'press'; value.button = event.get_button();
            this.buttons.add(value.button);
            if ([1, 3].includes(value.button) && !this.grabPointer(actor)) return Clutter.EVENT_STOP;
            break;
        case Clutter.EventType.BUTTON_RELEASE:
            value.type = 'release'; value.button = event.get_button();
            this.buttons.delete(value.button);
            if (!this.buttons.size) this.releasePointer();
            break;
        case Clutter.EventType.TOUCH_BEGIN:
        case Clutter.EventType.TOUCH_UPDATE:
        case Clutter.EventType.TOUCH_END:
        case Clutter.EventType.TOUCH_CANCEL: {
            const touch = event.get_event_sequence().get_slot();
            value.type = 'touch'; value.touch = touch;
            value.kind = kind === Clutter.EventType.TOUCH_BEGIN ? 'begin' :
                kind === Clutter.EventType.TOUCH_UPDATE ? 'update' :
                    kind === Clutter.EventType.TOUCH_END ? 'end' : 'cancel';
            if (value.kind === 'begin') this.touches.add(touch);
            if (value.kind === 'end' || value.kind === 'cancel') this.touches.delete(touch);
            break;
        }
        default: return Clutter.EVENT_PROPAGATE;
        }
        this.owner.peer?.send(value);
        if (value.type === 'release' || (value.type === 'touch' && ['end', 'cancel'].includes(value.kind)))
            this.owner.later(() => this.applyInput());
        return Clutter.EVENT_STOP;
    }

    reconcileButtons(state) {
        // A grab can be interrupted without delivering its final release.
        // Use the next real motion's button mask, never an inactivity timeout:
        // a stationary held mouse button must continue to own its drag.
        const masks = {1: Clutter.ModifierType.BUTTON1_MASK, 2: Clutter.ModifierType.BUTTON2_MASK,
            3: Clutter.ModifierType.BUTTON3_MASK, 4: Clutter.ModifierType.BUTTON4_MASK,
            5: Clutter.ModifierType.BUTTON5_MASK};
        const primaryWasDown = this.buttons.has(1);
        let changed = false;
        for (const button of this.buttons) {
            if (masks[button] !== undefined && !(state & masks[button])) {
                this.buttons.delete(button);
                changed = true;
            }
        }
        if (!changed) return;
        if (!this.buttons.size) this.releasePointer();
        // Cancel a stale click rather than activating whatever is now below it.
        // A physical mouse must not cancel an active touchscreen interaction.
        if (!this.touches.size && ((primaryWasDown && !this.buttons.has(1)) || !this.buttons.size))
            this.owner.peer?.send({type: 'cancel', id: this.monitor.id, generation: this.owner.generation});
        this.owner.later(() => this.applyInput());
    }

    cancel() {
        this.buttons.clear(); this.touches.clear();
        this.owner.peer?.send({type: 'cancel', id: this.monitor.id, generation: this.owner.generation});
        this.clearInput();
    }
    visibility() {
        this.root.visible = this.ready && this.owner.active();
        if (!this.root.visible && this.inputs.length) this.cancel();
        this.applyInput();
    }
    destroy() {
        this.destroyed = true;
        this.cancel();
        this.pending = null;
        this.tiles.clear();
        removeChromeActor(this.root);
    }
}

export default class SideraNative extends Extension {
    enable() {
        // GNOME 50 only has a Wayland session and removed this function.
        // Older releases still need the check to reject their X11 sessions.
        if (typeof Meta.is_wayland_compositor === 'function' && !Meta.is_wayland_compositor())
            throw new Error('Sidera 原生扩展需要 GNOME Wayland 会话');
        this.enabled = true;
        this.generation = 0;
        this.refreshSerial = 0;
        this.outputs = new Map();
        this.sources = new Set();
        this.signals = [];
        this.monitors = [];
        this.peer = null;
        this.shooting = false;
        this.keyboard = null;
        this.pointer = null;
        this.keysBound = false;
        this.hotkey = false;
        this.focusWindow = null;
        this.focusTitleSignal = 0;
        try {
            this.stopListening = listen(connection => this.connected(connection));
            const watch = (object, signal, callback) => this.signals.push([object, object.connect(signal, callback)]);
            watch(Main.layoutManager, 'monitors-changed', () => this.refreshMonitors());
            const visibilityChanged = () => { this.visibility(); this.sendState(); };
            watch(Main.sessionMode, 'updated', visibilityChanged);
            watch(Main.overview, 'showing', visibilityChanged);
            watch(Main.overview, 'hidden', visibilityChanged);
            watch(global.display, 'notify::focus-window', () => this.trackFocus());
            this.trackFocus();
            this.settings = this.getSettings();
            this.timer = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 1, () => {
                if (this.peer && GLib.get_monotonic_time() - this.peer.lastSeen > 5000000)
                    this.peer.close();
                return GLib.SOURCE_CONTINUE;
            });
            this.refreshMonitors();
        } catch (e) { this.disable(); throw e; }
    }

    active() {
        return this.enabled && this.peer && !Main.sessionMode.isLocked &&
            !Main.sessionMode.isGreeter && !Main.overview.visible && Main.modalCount === 0;
    }
    later(callback) {
        const id = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            this.sources.delete(id);
            if (this.enabled) callback();
            return GLib.SOURCE_REMOVE;
        });
        this.sources.add(id);
    }
    visibility() { for (const output of this.outputs.values()) output.visibility(); }
    wpsForeground() {
        const window = global.display.focus_window;
        return Boolean(this.active() && window &&
            isWpsPresentation(window.get_wm_class_instance(), window.get_wm_class(), window.get_title()));
    }
    sendState() {
        const window = global.display.focus_window;
        this.peer?.send({type: 'state', generation: this.generation,
            fullscreen: window?.is_fullscreen() ? window.get_monitor() : -1,
            wpsPresentation: this.wpsForeground()});
    }
    trackFocus() {
        const window = global.display.focus_window;
        if (window !== this.focusWindow) {
            this.clearFocus();
            this.focusWindow = window;
            if (window)
                this.focusTitleSignal = window.connect('notify::title', () => this.sendState());
        }
        this.sendState();
    }
    clearFocus() {
        if (this.focusTitleSignal)
            this.focusWindow.disconnect(this.focusTitleSignal);
        this.focusTitleSignal = 0;
        this.focusWindow = null;
    }
    clearOutputs() {
        const outputs = [...this.outputs.values()];
        this.outputs.clear();
        for (const output of outputs) {
            try { output.destroy(); }
            catch (error) { console.error(`Sidera overlay cleanup: ${error.stack ?? error}`); }
        }
    }

    bindKeys() {
        this.keysBound = true;
        this.hotkey = Main.wm.addKeybinding('toggle-annotation', this.settings, Meta.KeyBindingFlags.NONE,
            Shell.ActionMode.NORMAL, () => this.peer?.send({type: 'toggle'})) !== Meta.KeyBindingAction.NONE;
        Main.wm.addKeybinding('release-input', this.settings, Meta.KeyBindingFlags.NONE,
            Shell.ActionMode.ALL, () => this.peer?.close());
    }
    unbindKeys() {
        if (!this.keysBound) return;
        Main.wm.removeKeybinding('toggle-annotation');
        Main.wm.removeKeybinding('release-input');
        this.keysBound = false;
        this.hotkey = false;
    }

    connected(connection) {
        // A running instance owns the bridge until it exits or times out.
        if (this.peer) { connection.close(null); return; }
        const peer = new Peer(connection, (message, tile) => this.packet(message, tile), () => {
            if (this.peer === peer) {
                this.peer = null; this.clearOutputs(); this.unbindKeys();
                this.keyboard = null; this.pointer = null;
            }
        });
        this.peer = peer;
        // Leave the compatibility backend and nested test desktop's shortcuts
        // available when no native client owns the extension.
        try {
            this.keyboard = backend().get_default_seat().create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
            this.pointer = backend().get_default_seat().create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);
            this.bindKeys();
        } catch (e) { this.connectionFailed(e, peer); return; }
        this.refreshMonitors();
        peer.start();
    }

    connectionFailed(error, peer) {
        console.error(`Sidera native initialization: ${error.stack ?? error}`);
        if (!peer || peer !== this.peer) return;
        // Send the failure before cleanup can emit cancel events ahead of hello.
        peer.fail(`GNOME 覆盖层初始化失败：${String(error.message ?? error).slice(0, 500)}`);
        this.clearOutputs();
        try { this.unbindKeys(); } catch (_) { /* Report the original failure. */ }
        this.keyboard = null; this.pointer = null;
    }

    async refreshMonitors() {
        const serial = ++this.refreshSerial;
        const peer = this.peer;
        let logical = [];
        try {
            // Never synchronously call the Shell's own D-Bus service.
            const state = await new Promise((resolve, reject) => Gio.DBus.session.call(
                'org.gnome.Mutter.DisplayConfig', '/org/gnome/Mutter/DisplayConfig',
                'org.gnome.Mutter.DisplayConfig', 'GetCurrentState', null, null,
                Gio.DBusCallFlags.NONE, 1500, null, (bus, result) => {
                    try { resolve(bus.call_finish(result).deep_unpack()); } catch (e) { reject(e); }
                }));
            logical = state[2];
        } catch (e) { console.debug(`Sidera monitor IDs: ${e.message}`); }
        if (!this.enabled || serial !== this.refreshSerial || peer !== this.peer) return;
        try {
            this.generation++;
            this.monitors = Main.layoutManager.monitors.slice(0, 16).map((m, index) => {
                const l = logical.find(item => item[0] === m.x && item[1] === m.y);
                const specs = l?.[5] ?? [];
                return {id: specs.length ? specs.map(s => `${s[0]}:${s[3]}`).join('+') : `gnome-${index}`,
                    index, x: m.x, y: m.y, width: m.width, height: m.height,
                    scale: Math.max(1, Math.min(4, l?.[2] ?? m.geometryScale ?? 1))};
            });
            this.clearOutputs();
            if (!peer) return;
            for (const monitor of this.monitors) this.outputs.set(monitor.index, new Output(this, monitor));
            peer.send({type: 'hello', protocol: 1, generation: this.generation, monitors: this.monitors,
                capabilities: {keyboard: true, rightClick: true, screenshot: true, hotkey: this.hotkey, touchShape: false}});
            this.trackFocus();
        } catch (error) { this.connectionFailed(error, peer); }
    }

    packet(message, tile) {
        if (tile) {
            if (tile.length < 32) throw new Error('Short Sidera tile');
            const view = new DataView(tile.buffer, tile.byteOffset, tile.byteLength);
            if (view.getUint32(0, false) !== this.generation) return;
            const output = this.outputs.get(view.getUint32(4, false));
            if (!output) throw new Error('Unknown Sidera output');
            output.tile(tile); return;
        }
        if (message.type === 'ping') {
            this.visibility();
            this.sendState();
            return;
        }
        if (message.generation !== this.generation) return;
        const output = this.outputs.get(message.index);
        switch (message.type) {
        case 'begin': output?.begin(message.width, message.height); break;
        case 'end': output?.end(); break;
        case 'input': output?.input(message.rects, message.full); break;
        case 'text_input': output?.textInput(message.enabled); break;
        case 'screenshot': if (this.active()) this.screenshot(); break;
        case 'key': {
            if (!this.active()) break;
            if ((message.presentationOnly || message.key === 'StartShow') && !this.wpsForeground()) break;
            const time = GLib.get_monotonic_time();
            if (message.key === 'StartShow') {
                const shift = Clutter.KEY_Shift_L;
                const f5 = Clutter.KEY_F5;
                this.keyboard.notify_keyval(time, shift, Clutter.KeyState.PRESSED);
                this.keyboard.notify_keyval(time + 10, f5, Clutter.KeyState.PRESSED);
                this.keyboard.notify_keyval(time + 20, f5, Clutter.KeyState.RELEASED);
                this.keyboard.notify_keyval(time + 30, shift, Clutter.KeyState.RELEASED);
                break;
            }
            const key = {Up: Clutter.KEY_Up, Down: Clutter.KEY_Down, Escape: Clutter.KEY_Escape}[message.key];
            if (key) {
                this.keyboard.notify_keyval(time, key, Clutter.KeyState.PRESSED);
                this.keyboard.notify_keyval(time, key, Clutter.KeyState.RELEASED);
            }
            break;
        }
        case 'rightClick':
            if (output && this.active() && !output.buttons.size && !output.touches.size &&
                Number.isFinite(message.x) && Number.isFinite(message.y)) {
                const {x, y, width, height} = output.monitor;
                if (message.x < 0 || message.y < 0 || message.x >= width || message.y >= height) break;
                // Let Clutter commit the updated input actors before forwarding.
                this.later(() => {
                    if (!this.active() || output.destroyed) return;
                    const time = GLib.get_monotonic_time();
                    this.pointer.notify_absolute_motion(time, x + message.x, y + message.y);
                    this.pointer.notify_button(time, Clutter.BUTTON_SECONDARY, Clutter.ButtonState.PRESSED);
                    this.pointer.notify_button(time, Clutter.BUTTON_SECONDARY, Clutter.ButtonState.RELEASED);
                });
            }
            break;
        default: throw new Error('Unsupported Sidera command');
        }
    }

    screenshot() {
        if (this.shooting) return;
        this.shooting = true;
        const peer = this.peer;
        const stream = Gio.MemoryOutputStream.new_resizable();
        const shooter = new Shell.Screenshot();
        shooter.screenshot(false, stream, async (object, result) => {
            try {
                const [ok] = object.screenshot_finish(result);
                if (!ok) throw new Error('Screenshot failed');
                stream.close(null);
                const bytes = stream.steal_as_bytes();
                if (!this.enabled || this.peer !== peer || !this.active()) return;
                St.Clipboard.get_default().set_content(St.ClipboardType.CLIPBOARD, 'image/png', bytes);
                const directory = GLib.get_user_special_dir(GLib.UserDirectory.DIRECTORY_PICTURES) ?? GLib.get_home_dir();
                const name = `Sidera-${GLib.DateTime.new_now_local().format('%Y%m%d-%H%M%S')}-${GLib.uuid_string_random().slice(0, 8)}.png`;
                const path = GLib.build_filenamev([directory, name]);
                const file = Gio.File.new_for_path(path);
                await new Promise((resolve, reject) => file.replace_contents_bytes_async(bytes, null, false,
                    Gio.FileCreateFlags.PRIVATE, null, (f, response) => {
                        try { f.replace_contents_finish(response); resolve(); } catch (e) { reject(e); }
                    }));
                if (this.peer === peer) peer.send({type: 'screenshot', path});
            } catch (e) {
                console.warn(`Sidera screenshot: ${e.message}`);
                if (this.peer === peer) peer.send({type: 'error', message: `截图失败：${e.message}`});
            } finally { this.shooting = false; }
        });
    }

    disable() {
        this.enabled = false;
        this.refreshSerial++;
        this.peer?.close(); this.peer = null;
        this.clearOutputs();
        this.stopListening?.(); this.stopListening = null;
        this.clearFocus();
        for (const [object, id] of this.signals ?? []) object.disconnect(id);
        this.signals = [];
        if (this.timer) GLib.source_remove(this.timer);
        this.timer = 0;
        for (const id of this.sources ?? []) GLib.source_remove(id);
        this.sources?.clear();
        this.unbindKeys();
        this.settings = null;
        this.keyboard = null; this.pointer = null;
    }
}
