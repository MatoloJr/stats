const VENDORS = {
    '0x10de': 'nvidia',
    '0x1002': 'amd',
    '0x8086': 'intel',
};

/**
* @param {string} text contents of a DRM vendor file
* @returns {'nvidia'|'amd'|'intel'|null}
*/
export function parseVendor(text) {
    return VENDORS[String(text).trim().toLowerCase()] ?? null;
}

/**
* @param {string|null|undefined} text runtime_status
* @returns {boolean}
*/
export function isSuspended(text) {
    return String(text ?? '').trim() === 'suspended';
}

/**
* Prefer an explicit card id, then an active discrete GPU, then any active card.
*
* @param {{id: string, vendor: string, state: string}[]} cards
* @param {string} preferredId
*/
export function selectCard(cards, preferredId) {
    if (preferredId) {
        const chosen = cards.find(card => card.id === preferredId);
        if (chosen)
            return chosen;
    }
    const discrete = cards.find(card =>
        (card.vendor === 'nvidia' || card.vendor === 'amd') && card.state !== 'off');
    if (discrete)
        return discrete;
    return cards.find(card => card.state !== 'off') ?? cards[0] ?? null;
}
