import Clutter from 'gi://Clutter';
import Pango from 'gi://Pango';
import St from 'gi://St';

import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

import {Sparkline} from './sparkline.js';

/**
 * Small builders for the dashboard: cards, label/value rows, tiles, segmented
 * buttons, switches and buttons. Each returns the actors it made so the
 * dashboard can update them in place on every poll.
 */

const LEVEL_CLASSES = ['stats-level-warning', 'stats-level-critical'];

/**
 * @param {St.Widget} actor
 * @param {string} level 'normal', 'warning' or 'critical'
 */
export function setLevel(actor, level) {
    for (const name of LEVEL_CLASSES)
        actor.remove_style_class_name(name);
    if (level === 'warning' || level === 'critical')
        actor.add_style_class_name(`stats-level-${level}`);
}

/**
 * @param {St.Label} label
 * @param {string} text
 */
export function setText(label, text) {
    if (label.text !== text)
        label.text = text;
}

/**
 * @param {string} styleClass
 * @param {object} [extra]
 * @returns {St.Label}
 */
export function label(styleClass, extra = {}) {
    const actor = new St.Label({style_class: styleClass, y_align: Clutter.ActorAlign.CENTER, ...extra});
    actor.clutter_text.ellipsize = Pango.EllipsizeMode.END;
    return actor;
}

/**
 * A label that wraps onto several lines (notes and messages).
 *
 * @param {string} styleClass
 * @param {object} [extra]
 * @returns {St.Label}
 */
export function wrapLabel(styleClass, extra = {}) {
    const actor = new St.Label({style_class: styleClass, x_expand: true, ...extra});
    actor.clutter_text.line_wrap = true;
    actor.clutter_text.line_wrap_mode = Pango.WrapMode.WORD_CHAR;
    actor.clutter_text.ellipsize = Pango.EllipsizeMode.NONE;
    return actor;
}

/**
 * A rounded card with a heading that folds its body away.
 *
 * @param {string} title
 * @param {() => void} onToggle
 * @returns {{card: St.BoxLayout, header: St.Button, arrow: St.Icon, summary: St.Label, body: St.BoxLayout}}
 */
export function makeCard(title, onToggle) {
    const card = new St.BoxLayout({vertical: true, style_class: 'stats-card', x_expand: true});
    const arrow = new St.Icon({icon_name: 'pan-down-symbolic', style_class: 'stats-card-arrow', y_align: Clutter.ActorAlign.CENTER});
    const summary = label('stats-card-summary', {x_align: Clutter.ActorAlign.END});
    const heading = new St.BoxLayout({style_class: 'stats-card-heading', x_expand: true});
    heading.add_child(arrow);
    heading.add_child(label('stats-card-title', {text: title, x_expand: true}));
    heading.add_child(summary);
    const header = new St.Button({style_class: 'stats-card-header', can_focus: true, x_expand: true, child: heading});
    header.connect('clicked', () => onToggle());
    const body = new St.BoxLayout({vertical: true, style_class: 'stats-card-body', x_expand: true});
    card.add_child(header);
    card.add_child(body);
    return {card, header, arrow, summary, body};
}

/**
 * Label on the left, value on the right.
 *
 * @returns {{box: St.BoxLayout, key: St.Label, value: St.Label}}
 */
export function makeRow() {
    const box = new St.BoxLayout({style_class: 'stats-row', x_expand: true});
    const key = label('stats-row-key', {x_expand: true});
    const value = label('stats-row-value', {x_align: Clutter.ActorAlign.END});
    box.add_child(key);
    box.add_child(value);
    return {box, key, value};
}

/**
 * @param {{key: St.Label, value: St.Label}} row
 * @param {string} key
 * @param {string} value
 * @param {string} [level]
 */
export function setRow(row, key, value, level = 'normal') {
    setText(row.key, key);
    setText(row.value, value);
    setLevel(row.value, level);
}

/**
 * A live tile: label, big value, detail and a small graph. Clickable.
 *
 * @param {() => void} onClick
 */
export function makeTile(onClick) {
    const box = new St.BoxLayout({vertical: true, style_class: 'stats-tile-box', x_expand: true});
    const name = label('stats-tile-label');
    const value = label('stats-tile-value');
    const detail = label('stats-tile-detail');
    const spark = new Sparkline();
    spark.add_style_class_name('stats-tile-spark');
    box.add_child(name);
    box.add_child(value);
    box.add_child(detail);
    box.add_child(spark);
    const button = new St.Button({style_class: 'stats-tile', can_focus: true, x_expand: true, child: box});
    button.connect('clicked', () => onClick());
    return {button, name, value, detail, spark};
}

/**
 * A row of mutually exclusive buttons.
 *
 * @param {{label: string, value: string}[]} options
 * @param {(value: string) => void} onPick
 * @returns {{box: St.BoxLayout, select: (value: string) => void}}
 */
export function makeSegmented(options, onPick) {
    const box = new St.BoxLayout({style_class: 'stats-segmented', y_align: Clutter.ActorAlign.CENTER});
    const buttons = options.map(option => {
        const button = new St.Button({label: option.label, style_class: 'stats-segment', can_focus: true});
        button.connect('clicked', () => onPick(option.value));
        box.add_child(button);
        return {button, value: option.value};
    });
    const select = value => {
        for (const entry of buttons) {
            if (entry.value === value)
                entry.button.add_style_class_name('selected');
            else
                entry.button.remove_style_class_name('selected');
        }
    };
    return {box, select};
}

/**
 * A titled row with a control on the right.
 *
 * @param {string} title
 * @param {string} subtitle
 * @param {Clutter.Actor} control
 * @returns {St.BoxLayout}
 */
export function makeControlRow(title, subtitle, control) {
    const box = new St.BoxLayout({style_class: 'stats-control-row', x_expand: true});
    const text = new St.BoxLayout({vertical: true, x_expand: true, y_align: Clutter.ActorAlign.CENTER});
    text.add_child(label('stats-control-title', {text: title}));
    if (subtitle)
        text.add_child(label('stats-control-subtitle', {text: subtitle}));
    box.add_child(text);
    box.add_child(control);
    return box;
}

/**
 * An on/off row using the shell's own switch. The whole row is clickable.
 *
 * @param {string} title
 * @param {string} subtitle
 * @param {(on: boolean) => void} onToggle
 * @returns {{button: St.Button, setState: (on: boolean) => void}}
 */
export function makeSwitchRow(title, subtitle, onToggle) {
    const toggle = new PopupMenu.Switch(false);
    toggle.y_align = Clutter.ActorAlign.CENTER;
    const row = makeControlRow(title, subtitle, toggle);
    const button = new St.Button({style_class: 'stats-switch-row', can_focus: true, x_expand: true, child: row});
    button.connect('clicked', () => onToggle(!toggle.state));
    return {
        button,
        setState: on => {
            if (toggle.state !== on)
                toggle.state = on;
        },
    };
}

/**
 * @param {string} text
 * @param {string} iconName
 * @param {() => void} onClick
 * @param {string} [extraClass]
 * @returns {St.Button}
 */
export function makeButton(text, iconName, onClick, extraClass = '') {
    const box = new St.BoxLayout({style_class: 'stats-button-box'});
    if (iconName)
        box.add_child(new St.Icon({icon_name: iconName, style_class: 'stats-button-icon'}));
    box.add_child(new St.Label({text, y_align: Clutter.ActorAlign.CENTER}));
    const button = new St.Button({style_class: `stats-button ${extraClass}`.trim(), can_focus: true, child: box});
    button.connect('clicked', () => onClick());
    return button;
}
