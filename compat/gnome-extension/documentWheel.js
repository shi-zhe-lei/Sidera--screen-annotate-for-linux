import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';

let serial = 0;

// A gesture is born from a real rail press. The renderer can supply distances,
// but cannot choose a window, screen position, button or modifier.
export class DocumentWheel {
    constructor(finger, device) {
        this.id = ++serial;
        this.finger = finger;
        this.device = device;
        this.window = global.display.focus_window;
        this.remainder = 0;
    }

    overWindow(x, y) {
        const target = this.window?.get_compositor_private();
        for (let actor = global.stage.get_actor_at_pos(Clutter.PickMode.REACTIVE, x, y); actor; actor = actor.get_parent())
            if (actor === target) return true;
        return false;
    }

    point() {
        if (!this.window || this.window !== global.display.focus_window) return null;
        const rect = this.window.get_frame_rect();
        for (const [fx, fy] of [[0.5, 0.55], [0.65, 0.68], [0.36, 0.62]]) {
            const x = Math.floor(rect.x + rect.width * fx), y = Math.floor(rect.y + rect.height * fy);
            if (this.overWindow(x, y)) return [x, y];
        }
        return null;
    }

    park(pointer, x, y, point) {
        if (!this.returnPoint) {
            this.pointer = pointer;
            this.returnPoint = [x, y];
            if (this.finger === 'mouse') {
                // A physical mouse supplies relative motion; a tablet or VM may
                // supply absolute coordinates. Ignore our own virtual device.
                this.motion = global.stage.connect('captured-event', (_stage, e) => {
                    if (e.type() === Clutter.EventType.MOTION && e.get_source_device() === this.device) {
                        const [sx, sy] = e.get_coords();
                        if (sx !== this.deliveryPoint[0] || sy !== this.deliveryPoint[1]) {
                            const [relative, dx, dy] = e.get_relative_motion?.() ?? [];
                            this.returnPoint = relative ? [this.returnPoint[0]+dx, this.returnPoint[1]+dy] : [sx, sy];
                            this.sampled = [sx, sy];
                        }
                    }
                    return Clutter.EVENT_PROPAGATE;
                });
            }
        }
        this.sampled = [x, y];
        this.deliveryPoint = point;
        pointer.notify_absolute_motion(GLib.get_monotonic_time(), ...point);
    }

    position(x, y) {
        if (!this.returnPoint || this.finger !== 'mouse') return [x, y];
        const [lastX, lastY] = this.sampled;
        const [tx, ty] = this.deliveryPoint;
        const [rx, ry] = this.returnPoint;
        // XWayland can consume a physical motion before the stage capture sees
        // it. Recover absolute tablet coordinates or a relative mouse delta.
        if ((x !== lastX || y !== lastY) && (x !== tx || y !== ty)) {
            this.returnPoint = Math.hypot(x-rx, y-ry) < Math.hypot(x-tx, y-ty) ?
                [x,y] : [rx+x-lastX, ry+y-lastY];
        }
        this.sampled = [x,y];
        return this.returnPoint;
    }

    finish() {
        if (this.delivery) GLib.source_remove(this.delivery);
        this.delivery = 0;
        if (this.motion) global.stage.disconnect(this.motion);
        this.motion = 0;
        if (this.returnPoint) {
            const [x,y] = global.get_pointer();
            if (this.finger === 'mouse' || (x === this.deliveryPoint[0] && y === this.deliveryPoint[1]))
                this.pointer.notify_absolute_motion(GLib.get_monotonic_time(), ...this.returnPoint);
        }
        this.returnPoint = null;
        this.deliveryPoint = null;
        this.remainder = 0;
    }

    scroll(pointer, pixels) {
        const point = this.point();
        if (!point || !pointer || !Number.isFinite(pixels) || !pixels) return false;
        const [x, y, mask] = global.get_pointer();
        const mods = Clutter.ModifierType;
        if (mask & (mods.CONTROL_MASK | mods.SHIFT_MASK | (mods.ALT_MASK ?? mods.MOD1_MASK) | mods.SUPER_MASK)) return false;
        const distance = Math.max(-64, Math.min(64, pixels));
        // XWayland applications using older Qt expect wheel detents. Wayland
        // surfaces receive a continuous axis with the displacement of this frame.
        const discrete = this.window.get_client_type() === Meta.WindowClientType.X11;
        if (discrete) {
            if (Math.sign(this.remainder) !== Math.sign(distance)) this.remainder = 0;
            const detent = 100;
            this.remainder = Math.max(-2*detent, Math.min(2*detent, this.remainder + distance));
            if (Math.abs(this.remainder) < detent || this.delivery) return true;
            // XWayland and the embedded office viewport consume the events
            // asynchronously. Retain pointer focus on the document until release.
            this.park(pointer, x, y, point);
            this.delivery = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 18, () => {
                this.delivery = 0;
                const [px, py, state] = global.get_pointer();
                if (this.window !== global.display.focus_window || !this.overWindow(px, py) ||
                    (this.finger === 'mouse' && !(state & mods.BUTTON1_MASK)) ||
                    (state & (mods.CONTROL_MASK | mods.SHIFT_MASK | (mods.ALT_MASK ?? mods.MOD1_MASK) | mods.SUPER_MASK)))
                    return GLib.SOURCE_REMOVE;
                const sign = Math.sign(this.remainder);
                const direction = sign > 0 ? Clutter.ScrollDirection.DOWN : Clutter.ScrollDirection.UP;
                while (Math.abs(this.remainder) >= detent) {
                    pointer.notify_discrete_scroll(GLib.get_monotonic_time(), direction, Clutter.ScrollSource.WHEEL);
                    this.remainder -= sign * detent;
                }
                return GLib.SOURCE_REMOVE;
            });
            return true;
        }
        const t = GLib.get_monotonic_time();
        this.deliveryPoint = point;
        pointer.notify_absolute_motion(t, ...point);
        try {
            // Mutter converts Clutter wheel units to Wayland surface pixels.
            pointer.notify_scroll_continuous(t, 0, distance / 10, Clutter.ScrollSource.CONTINUOUS, 0);
        } finally {
            pointer.notify_absolute_motion(t, x, y);
        }
        return true;
    }
}
