// Download site behaviour: OS detection, tabs, copy buttons, latest release,
// and the floating nav. Every part works without the others; with JS off the
// Linux instructions still show and every link still works.
'use strict';

const REPO = 'MatoloJr/stats';

function detectOs() {
    const platform = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || '';
    const ua = navigator.userAgent || '';
    const text = `${platform} ${ua}`.toLowerCase();
    if (text.includes('win'))
        return 'windows';
    if (text.includes('mac') || /iphone|ipad/.test(text))
        return 'macos';
    return 'linux';
}

// ---------- Tabs (WAI-ARIA tabs pattern) ----------
const tabs = [...document.querySelectorAll('[role="tab"]')];

function selectTab(tab, focus) {
    for (const t of tabs) {
        const on = t === tab;
        t.setAttribute('aria-selected', String(on));
        t.tabIndex = on ? 0 : -1;
        document.getElementById(t.getAttribute('aria-controls')).hidden = !on;
    }
    if (focus)
        tab.focus();
}

for (const tab of tabs) {
    tab.addEventListener('click', () => selectTab(tab, false));
    tab.addEventListener('keydown', e => {
        const i = tabs.indexOf(tab);
        const next = { ArrowRight: i + 1, ArrowLeft: i - 1, Home: 0, End: tabs.length - 1 }[e.key];
        if (next === undefined)
            return;
        e.preventDefault();
        selectTab(tabs[(next + tabs.length) % tabs.length], true);
    });
}

const os = detectOs();
const detected = tabs.find(t => t.dataset.os === os);
if (detected) {
    selectTab(detected, false);
    detected.title = 'Detected from your browser';
    const hero = document.getElementById('hero-download');
    hero.textContent = os === 'macos' ? 'macOS is coming: see Linux & Windows' : `Download for ${detected.textContent}`;
}

// ---------- Copy buttons ----------
async function copyText(text) {
    if (navigator.clipboard && window.isSecureContext) {
        await navigator.clipboard.writeText(text);
        return;
    }
    const area = document.createElement('textarea');
    area.value = text;
    area.setAttribute('readonly', '');
    area.style.position = 'fixed';
    area.style.opacity = '0';
    document.body.append(area);
    area.select();
    const ok = document.execCommand('copy');
    area.remove();
    if (!ok)
        throw new Error('copy failed');
}

for (const button of document.querySelectorAll('.copy')) {
    button.addEventListener('click', async () => {
        const code = document.getElementById(button.dataset.copy);
        try {
            await copyText(code.textContent.trim());
            button.textContent = 'Copied';
            button.classList.add('is-done');
        } catch {
            // Fall back to selecting the text so Ctrl+C works.
            const range = document.createRange();
            range.selectNodeContents(code);
            const sel = window.getSelection();
            sel.removeAllRanges();
            sel.addRange(range);
            button.textContent = 'Press Ctrl+C';
        }
        clearTimeout(button._t);
        button._t = setTimeout(() => {
            button.textContent = 'Copy';
            button.classList.remove('is-done');
        }, 2000);
    });
}

// ---------- Latest release (optional: the links work without it) ----------
(async () => {
    const out = document.getElementById('release');
    try {
        const res = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, { headers: { Accept: 'application/vnd.github+json' } });
        if (!res.ok)
            return;
        const rel = await res.json();
        const date = new Date(rel.published_at).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
        out.textContent = `Latest ${rel.tag_name} · ${date}`;
    } catch {
        // Offline or rate-limited: say nothing rather than something wrong.
    }
})();

// ---------- Floating nav: active section indicator, hide on scroll down ----------
const nav = document.querySelector('.nav');
const indicator = nav.querySelector('.nav__indicator');
const links = [...nav.querySelectorAll('.nav__link')];

function moveIndicator(link) {
    for (const l of links)
        l.classList.toggle('is-active', l === link);
    if (!link) {
        indicator.style.opacity = '0';
        return;
    }
    indicator.style.opacity = '1';
    indicator.style.width = `${link.offsetWidth}px`;
    indicator.style.transform = `translateX(${link.offsetLeft}px)`;
}

// In-page links only ("#id"); links to other pages have no section here.
const sections = links.map(l => {
    const href = l.getAttribute('href');
    return href.startsWith('#') ? document.querySelector(href) : null;
});
const visible = new Map();
const observer = new IntersectionObserver(entries => {
    for (const e of entries)
        visible.set(e.target, e.intersectionRatio);
    let best = null, bestRatio = 0;
    for (const [i, s] of sections.entries()) {
        const r = visible.get(s) || 0;
        if (r > bestRatio) {
            best = links[i];
            bestRatio = r;
        }
    }
    moveIndicator(best);
}, { threshold: [0, .15, .35, .6], rootMargin: '-20% 0px -40% 0px' });
sections.forEach(s => s && observer.observe(s));
window.addEventListener('resize', () => moveIndicator(links.find(l => l.classList.contains('is-active'))));

let lastY = window.scrollY;
let ticking = false;
window.addEventListener('scroll', () => {
    if (ticking)
        return;
    ticking = true;
    requestAnimationFrame(() => {
        const y = window.scrollY;
        const down = y > lastY + 6;
        const up = y < lastY - 6;
        if (down && y > 200 && !nav.contains(document.activeElement))
            nav.classList.add('is-hidden');
        else if (up || y < 200)
            nav.classList.remove('is-hidden');
        if (down || up)
            lastY = y;
        ticking = false;
    });
}, { passive: true });
nav.addEventListener('focusin', () => nav.classList.remove('is-hidden'));
