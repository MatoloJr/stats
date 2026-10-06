import GObject from 'gi://GObject';
import St from 'gi://St';

/**
* Cairo sparkline. Colours come from the theme foreground so light and dark
* shell themes both work. Call cr.$dispose() before leaving repaint.
*/
export const Sparkline = GObject.registerClass(
    class Sparkline extends St.DrawingArea {
        _init() {
            super._init({
                style_class: 'stats-sparkline',
                x_expand: true,
                height: 36,
            });
            this._series = [];
            this._max = 100;
            this.connect('repaint', area => this._repaint(area));
        }

        /**
        * @param {(number|null)[][]} series
        * @param {number|null} max Explicit ceiling, or null to scale to the data.
        */
        setSeries(series, max = null) {
            const nextMax = max ?? autoscale(series);
            if (nextMax === this._max && sameSeries(this._series, series))
                return;
            this._series = series.map(values => values.slice());
            this._max = nextMax;
            this.queue_repaint();
        }

        /**
        * @param {St.DrawingArea} area
        */
        _repaint(area) {
            const cr = area.get_context();
            try {
                const [width, height] = area.get_surface_size();
                if (width < 2 || height < 2)
                    return;
                const color = area.get_theme_node().get_foreground_color();
                this._series.forEach((values, index) => {
                    const alpha = index === 0 ? 0.95 : 0.45;
                    drawLine(cr, values, width, height, this._max, [
                        channel(color.red),
                        channel(color.green),
                        channel(color.blue),
                        alpha,
                    ]);
                });
            } finally {
                cr.$dispose();
            }
        }
    });

/**
* Clutter.Color channels are 0..255. Some theme nodes report 0..1 floats.
*
* @param {number} value
* @returns {number}
*/
function channel(value) {
    return value > 1 ? value / 255 : value;
}

/**
* @param {(number|null)[][]} series
* @returns {number}
*/
function autoscale(series) {
    let max = 1;
    for (const values of series) {
        for (const value of values) {
            if (Number.isFinite(value))
                max = Math.max(max, value);
        }
    }
    return max;
}

/**
* @param {(number|null)[][]} left
* @param {(number|null)[][]} right
* @returns {boolean}
*/
function sameSeries(left, right) {
    if (left.length !== right.length)
        return false;
    for (let index = 0; index < left.length; index++) {
        const a = left[index];
        const b = right[index];
        if (a.length !== b.length)
            return false;
        for (let point = 0; point < a.length; point++) {
            if (a[point] !== b[point])
                return false;
        }
    }
    return true;
}

/**
* @param {cairo.Context} cr
* @param {(number|null)[]} values
* @param {number} width
* @param {number} height
* @param {number} max
* @param {number[]} rgba
*/
function drawLine(cr, values, width, height, max, rgba) {
    if (values.length < 2 || !(max > 0))
        return;
    cr.setSourceRGBA(rgba[0], rgba[1], rgba[2], rgba[3]);
    cr.setLineWidth(1.5);
    let drawing = false;
    const span = Math.max(values.length - 1, 1);
    for (let index = 0; index < values.length; index++) {
        const value = values[index];
        if (!Number.isFinite(value)) {
            drawing = false;
            continue;
        }
        const x = (index / span) * (width - 2) + 1;
        const y = height - 2 - (Math.min(value, max) / max) * (height - 4);
        if (!drawing) {
            cr.moveTo(x, y);
            drawing = true;
        } else {
            cr.lineTo(x, y);
        }
    }
    cr.stroke();
}
