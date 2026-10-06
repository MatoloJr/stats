import {LOG_PREFIX} from './constants.js';

/**
* @param {string} message
*/
export function logInfo(message) {
    console.log(`${LOG_PREFIX} ${message}`);
}

/**
* @param {string} message
*/
export function logWarn(message) {
    console.warn(`${LOG_PREFIX} ${message}`);
}

/**
* @param {string} message
*/
export function logError(message) {
    console.error(`${LOG_PREFIX} ${message}`);
}
