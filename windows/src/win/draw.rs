//! GDI drawing: a double-buffered canvas, fonts, icons, text and sparklines.
//! Every GDI object created here is deleted again, so painting never leaks.

use std::mem::zeroed;
use std::ptr::null_mut;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, POINT, RECT, SIZE};
use windows_sys::Win32::Graphics::Gdi::*;

use super::util::wide;
use stats_core::config::Level;

pub const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

// One palette everywhere, matching the website: graphite surfaces, light ink
// and one teal accent. Amber and red are the only other hues, kept because in
// a monitor they carry meaning: warning and critical.
pub const BG: COLORREF = rgb(19, 25, 23);
pub const PANEL: COLORREF = rgb(27, 34, 32);
pub const PANEL_HOT: COLORREF = rgb(34, 43, 40);
pub const LINE: COLORREF = rgb(45, 55, 52);
pub const TEXT: COLORREF = rgb(230, 235, 231);
pub const MUTED: COLORREF = rgb(150, 163, 158);
pub const ACCENT: COLORREF = rgb(76, 194, 166);
pub const ACCENT_DIM: COLORREF = rgb(31, 74, 64);
pub const WARN: COLORREF = rgb(245, 178, 72);
pub const CRIT: COLORREF = rgb(240, 96, 90);
pub const OK: COLORREF = ACCENT;
pub const BUTTON: COLORREF = rgb(38, 48, 45);
pub const BUTTON_HOT: COLORREF = rgb(50, 63, 59);

pub fn level_color(level: Level) -> COLORREF {
    match level {
        Level::Normal => TEXT,
        Level::Warning => WARN,
        Level::Critical => CRIT,
    }
}

/// Accent when normal, amber or red otherwise: for lines, edges and waves.
pub fn signal_color(level: Level) -> COLORREF {
    match level {
        Level::Normal => ACCENT,
        other => level_color(other),
    }
}

/// (r, g, b) of a COLORREF.
pub fn parts(color: COLORREF) -> (u8, u8, u8) {
    ((color & 0xFF) as u8, ((color >> 8) & 0xFF) as u8, ((color >> 16) & 0xFF) as u8)
}

/// Icon glyphs: Segoe Fluent Icons, or the same code points in Segoe MDL2
/// Assets on Windows 10.
pub mod glyph {
    pub const PIN: char = '\u{E718}';
    pub const PINNED: char = '\u{E840}';
    pub const CHEVRON_DOWN: char = '\u{E70D}';
    pub const CHEVRON_RIGHT: char = '\u{E76C}';
    pub const SETTINGS: char = '\u{E713}';
    pub const CLOSE: char = '\u{E711}';
    pub const POWER: char = '\u{E7E8}';
    pub const SEARCH: char = '\u{E721}';
    pub const DOCUMENT: char = '\u{E8A5}';
}

pub struct Fonts {
    pub dpi: u32,
    pub normal: HFONT,
    pub bold: HFONT,
    pub small: HFONT,
    pub title: HFONT,
    /// Readings strip, like the website's bar.
    pub mono: HFONT,
    pub mono_bold: HFONT,
    /// Big numbers on the dashboard tiles.
    pub big: HFONT,
    pub icons: HFONT,
}

unsafe extern "system" fn found_font(_: *const LOGFONTW, _: *const TEXTMETRICW, _: u32, found: LPARAM) -> i32 {
    *(found as *mut bool) = true;
    0
}

fn installed(face: &str) -> bool {
    let mut lf: LOGFONTW = unsafe { zeroed() };
    lf.lfCharSet = DEFAULT_CHARSET;
    for (slot, unit) in lf.lfFaceName.iter_mut().zip(face.encode_utf16().take(31)) {
        *slot = unit;
    }
    let mut found = false;
    unsafe {
        let dc = GetDC(null_mut());
        EnumFontFamiliesExW(dc, &lf, Some(found_font), &mut found as *mut bool as LPARAM, 0);
        ReleaseDC(null_mut(), dc);
    }
    found
}

/// (monospace face, icon face), looked up once.
fn faces() -> (&'static str, &'static str) {
    static FACES: OnceLock<(&str, &str)> = OnceLock::new();
    *FACES.get_or_init(|| {
        let mono = if installed("Cascadia Mono") { "Cascadia Mono" } else { "Consolas" };
        let icons = if installed("Segoe Fluent Icons") { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" };
        (mono, icons)
    })
}

fn make_font(face: &str, points: i32, weight: i32, dpi: u32) -> HFONT {
    let face = wide(face);
    let height = -(points * dpi as i32 / 72);
    unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            (DEFAULT_PITCH | FF_DONTCARE) as u32,
            face.as_ptr(),
        )
    }
}

impl Fonts {
    pub fn new(dpi: u32) -> Self {
        let dpi = dpi.max(96);
        let (mono, icons) = faces();
        Fonts {
            dpi,
            normal: make_font("Segoe UI", 9, FW_NORMAL as i32, dpi),
            bold: make_font("Segoe UI", 9, FW_SEMIBOLD as i32, dpi),
            small: make_font("Segoe UI", 8, FW_NORMAL as i32, dpi),
            title: make_font("Segoe UI", 11, FW_SEMIBOLD as i32, dpi),
            mono: make_font(mono, 9, FW_NORMAL as i32, dpi),
            mono_bold: make_font(mono, 9, FW_BOLD as i32, dpi),
            big: make_font("Segoe UI", 17, FW_SEMIBOLD as i32, dpi),
            icons: make_font(icons, 10, FW_NORMAL as i32, dpi),
        }
    }

    /// Scale a 96-DPI pixel length to this DPI.
    pub fn px(&self, value: i32) -> i32 {
        value * self.dpi as i32 / 96
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        unsafe {
            for font in
                [self.normal, self.bold, self.small, self.title, self.mono, self.mono_bold, self.big, self.icons]
            {
                DeleteObject(font);
            }
        }
    }
}

/// Paints into an off-screen bitmap, copied to the window in one step on
/// drop, so nothing flickers.
pub struct Canvas {
    hwnd: HWND,
    ps: PAINTSTRUCT,
    window_dc: HDC,
    pub dc: HDC,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
    old_font: HGDIOBJ,
    pub width: i32,
    pub height: i32,
}

impl Canvas {
    pub fn begin(hwnd: HWND) -> Self {
        unsafe {
            let mut ps: PAINTSTRUCT = zeroed();
            let window_dc = BeginPaint(hwnd, &mut ps);
            let mut rect: RECT = zeroed();
            windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rect);
            let width = (rect.right - rect.left).max(1);
            let height = (rect.bottom - rect.top).max(1);
            let dc = CreateCompatibleDC(window_dc);
            let bitmap = CreateCompatibleBitmap(window_dc, width, height);
            let old_bitmap = SelectObject(dc, bitmap);
            SetBkMode(dc, TRANSPARENT as i32);
            Canvas { hwnd, ps, window_dc, dc, bitmap, old_bitmap, old_font: null_mut(), width, height }
        }
    }

    pub fn font(&mut self, font: HFONT) {
        let previous = unsafe { SelectObject(self.dc, font) };
        if self.old_font.is_null() {
            self.old_font = previous;
        }
    }

    pub fn fill(&self, rect: RECT, color: COLORREF) {
        unsafe {
            let brush = CreateSolidBrush(color);
            FillRect(self.dc, &rect, brush);
            DeleteObject(brush);
        }
    }

    pub fn round_rect(&self, rect: RECT, radius: i32, fill: COLORREF, border: Option<COLORREF>) {
        unsafe {
            let brush = CreateSolidBrush(fill);
            let pen = CreatePen(PS_SOLID, 1, border.unwrap_or(fill));
            let old_brush = SelectObject(self.dc, brush);
            let old_pen = SelectObject(self.dc, pen);
            RoundRect(self.dc, rect.left, rect.top, rect.right, rect.bottom, radius, radius);
            SelectObject(self.dc, old_brush);
            SelectObject(self.dc, old_pen);
            DeleteObject(brush);
            DeleteObject(pen);
        }
    }

    pub fn measure(&self, text: &str) -> i32 {
        measure(self.dc, text)
    }

    /// Draw `text` with its left edge at `x`, vertically centred in
    /// `top..bottom`. Returns the x just after the text.
    pub fn text(&self, text: &str, x: i32, top: i32, bottom: i32, color: COLORREF) -> i32 {
        let utf16: Vec<u16> = text.encode_utf16().collect();
        let mut rect = RECT { left: x, top, right: x + 10_000, bottom };
        unsafe {
            SetTextColor(self.dc, color);
            DrawTextW(
                self.dc,
                utf16.as_ptr(),
                utf16.len() as i32,
                &mut rect,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
        }
        x + self.measure(text)
    }

    /// Text centred in `rect`.
    pub fn text_center(&self, text: &str, rect: RECT, color: COLORREF) {
        let utf16: Vec<u16> = text.encode_utf16().collect();
        let mut r = rect;
        unsafe {
            SetTextColor(self.dc, color);
            DrawTextW(
                self.dc,
                utf16.as_ptr(),
                utf16.len() as i32,
                &mut r,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
        }
    }

    /// One icon glyph centred in `rect`. Leaves the icon font selected.
    pub fn icon(&mut self, fonts: &Fonts, glyph: char, rect: RECT, color: COLORREF) {
        self.font(fonts.icons);
        let mut buf = [0u8; 4];
        self.text_center(glyph.encode_utf8(&mut buf), rect, color);
    }

    /// Right-aligned single line ending at `right`.
    pub fn text_right(&self, text: &str, right: i32, top: i32, bottom: i32, color: COLORREF) {
        let width = self.measure(text);
        self.text(text, right - width, top, bottom, color);
    }

    /// Text cut with an ellipsis to fit `left..right`.
    pub fn text_clipped(&self, text: &str, left: i32, right: i32, top: i32, bottom: i32, color: COLORREF) {
        let utf16: Vec<u16> = text.encode_utf16().collect();
        let mut rect = RECT { left, top, right, bottom };
        unsafe {
            SetTextColor(self.dc, color);
            DrawTextW(
                self.dc,
                utf16.as_ptr(),
                utf16.len() as i32,
                &mut rect,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
            );
        }
    }

    /// Wrapped paragraph inside `rect`; returns the height used.
    pub fn paragraph(&self, text: &str, rect: RECT, color: COLORREF, measure_only: bool) -> i32 {
        paragraph(self.dc, text, rect, color, measure_only)
    }

    /// Line graph of `values` (NaN = gap) scaled to `max` inside `rect`.
    pub fn sparkline(&self, values: &[f32], max: f32, rect: RECT, color: COLORREF) {
        if values.len() < 2 || max <= 0.0 {
            return;
        }
        let width = (rect.right - rect.left) as f32;
        let height = (rect.bottom - rect.top - 2) as f32;
        let step = width / (values.len() - 1) as f32;
        unsafe {
            let pen = CreatePen(PS_SOLID, 1, color);
            let old = SelectObject(self.dc, pen);
            let mut points: Vec<POINT> = Vec::with_capacity(values.len());
            let flush = |points: &mut Vec<POINT>| {
                if points.len() >= 2 {
                    Polyline(self.dc, points.as_ptr(), points.len() as i32);
                }
                points.clear();
            };
            for (i, v) in values.iter().enumerate() {
                if !v.is_finite() {
                    flush(&mut points);
                    continue;
                }
                let y = rect.bottom - 1 - ((v / max).clamp(0.0, 1.0) * height) as i32;
                points.push(POINT { x: rect.left + (i as f32 * step) as i32, y });
            }
            flush(&mut points);
            SelectObject(self.dc, old);
            DeleteObject(pen);
        }
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            BitBlt(self.window_dc, 0, 0, self.width, self.height, self.dc, 0, 0, SRCCOPY);
            if !self.old_font.is_null() {
                SelectObject(self.dc, self.old_font);
            }
            SelectObject(self.dc, self.old_bitmap);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
            EndPaint(self.hwnd, &self.ps);
        }
    }
}

pub fn measure(dc: HDC, text: &str) -> i32 {
    let utf16: Vec<u16> = text.encode_utf16().collect();
    let mut size: SIZE = unsafe { zeroed() };
    unsafe { GetTextExtentPoint32W(dc, utf16.as_ptr(), utf16.len() as i32, &mut size) };
    size.cx
}

pub fn paragraph(dc: HDC, text: &str, rect: RECT, color: COLORREF, measure_only: bool) -> i32 {
    let utf16: Vec<u16> = text.encode_utf16().collect();
    let mut r = rect;
    unsafe {
        SetTextColor(dc, color);
        let mut flags = DT_LEFT | DT_WORDBREAK | DT_NOPREFIX;
        if measure_only {
            flags |= DT_CALCRECT;
        }
        DrawTextW(dc, utf16.as_ptr(), utf16.len() as i32, &mut r, flags);
    }
    r.bottom - r.top
}

/// A screen DC with `font` selected, for measuring outside WM_PAINT.
pub struct MeasureDc {
    dc: HDC,
    old: HGDIOBJ,
}

impl MeasureDc {
    pub fn new(font: HFONT) -> Self {
        unsafe {
            let dc = GetDC(null_mut());
            let old = SelectObject(dc, font);
            MeasureDc { dc, old }
        }
    }

    pub fn width(&self, text: &str) -> i32 {
        measure(self.dc, text)
    }

    pub fn select(&self, font: HFONT) {
        unsafe { SelectObject(self.dc, font) };
    }

    pub fn paragraph_height(&self, text: &str, width: i32) -> i32 {
        paragraph(self.dc, text, RECT { left: 0, top: 0, right: width, bottom: 10_000 }, 0, true)
    }
}

impl Drop for MeasureDc {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            ReleaseDC(null_mut(), self.dc);
        }
    }
}
