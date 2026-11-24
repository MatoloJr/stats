import Clutter from 'gi://Clutter';
import GObject from 'gi://GObject';
import St from 'gi://St';

/**
 * Horizontal usage bar. The fill is allocated explicitly so the width tracks
 * the percentage on GNOME 46 and 50 without relying on CSS background-size.
 */
export const Meter = GObject.registerClass(
    class Meter extends St.Widget {
        _init() {
            super._init({
                style_class: 'stats-meter-track',
                x_expand: true,
                height: 8,
                y_align: Clutter.ActorAlign.CENTER,
            });
            this._percent = 0;
            this._fill = new St.Widget({style_class: 'stats-meter-fill'});
            this.add_child(this._fill);
        }

        /**
         * @param {number|null|undefined} percent
         */
        setPercent(percent) {
            const next = Math.max(0, Math.min(100, Number.isFinite(percent) ? percent : 0));
            if (next === this._percent)
                return;
            this._percent = next;
            this.queue_relayout();
        }

        vfunc_allocate(box) {
            this.set_allocation(box);
            const childBox = new Clutter.ActorBox();
            childBox.set_origin(0, 0);
            childBox.set_size(
                Math.round(box.get_width() * (this._percent / 100)),
                box.get_height());
            this._fill.allocate(childBox);
        }
    });
