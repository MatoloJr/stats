/** GSettings schema id. The extension UUID lives only in metadata.json. */
export const SCHEMA_ID = 'org.gnome.shell.extensions.stats';

export const LOG_PREFIX = '[stats]';

/**
 * Metrics the user can show in the top bar.
 * `order` in the `bar-metrics` setting is the priority order.
 * Compact mode shows the first enabled id until the indicator is hovered.
 */
export const METRICS = [
    {id: 'cpu', label: 'CPU', icon: 'computer-symbolic'},
    {id: 'memory', label: 'Memory', icon: 'media-flash-symbolic'},
    {id: 'storage', label: 'Storage', icon: 'drive-harddisk-symbolic'},
    {id: 'gpu', label: 'GPU', icon: 'video-display-symbolic'},
    {id: 'temperature', label: 'Temperature', icon: 'power-profile-performance-symbolic'},
    {id: 'refresh', label: 'Refresh rate', icon: 'video-display-symbolic'},
];

export const METRIC_IDS = METRICS.map(metric => metric.id);
