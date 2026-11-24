//! Where the activity line, the readings strip and the dashboard go on a
//! screen. Plain rectangles in screen pixels, no OS calls.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Rect { left, top, right, bottom }
    }

    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

/// Which side of the top edge the line sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Anchor {
    Left,
    #[default]
    Center,
    Right,
}

impl Anchor {
    pub const ALL: [Anchor; 3] = [Anchor::Left, Anchor::Center, Anchor::Right];

    pub fn key(self) -> &'static str {
        match self {
            Anchor::Left => "left",
            Anchor::Center => "center",
            Anchor::Right => "right",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Anchor::Left => "Left",
            Anchor::Center => "Center",
            Anchor::Right => "Right",
        }
    }

    pub fn from_key(text: &str) -> Option<Anchor> {
        match text.trim().to_ascii_lowercase().as_str() {
            "left" | "l" => Some(Anchor::Left),
            "center" | "centre" | "middle" | "c" => Some(Anchor::Center),
            "right" | "r" => Some(Anchor::Right),
            _ => None,
        }
    }
}

/// Left edge of a box `width` wide along `area`, by anchor.
fn align(area: Rect, width: i32, anchor: Anchor, margin: i32) -> i32 {
    let x = match anchor {
        Anchor::Left => area.left + margin,
        Anchor::Center => area.left + (area.width() - width) / 2,
        Anchor::Right => area.right - margin - width,
    };
    x.clamp(area.left, (area.right - width).max(area.left))
}

/// The line: at the top of the work area (so below a top taskbar).
pub fn line_rect(work: Rect, width: i32, height: i32, anchor: Anchor, margin: i32) -> Rect {
    let width = width.min(work.width());
    let x = align(work, width, anchor, margin);
    let y = work.top + margin / 4;
    Rect::new(x, y, x + width, y + height)
}

/// The strip drops down under the line, on the same side, kept on screen.
pub fn strip_rect(line: Rect, width: i32, height: i32, work: Rect, anchor: Anchor, gap: i32) -> Rect {
    let width = width.min(work.width());
    let x = match anchor {
        Anchor::Left => line.left,
        Anchor::Center => line.left + (line.width() - width) / 2,
        Anchor::Right => line.right - width,
    }
    .clamp(work.left, (work.right - width).max(work.left));
    let y = (line.bottom + gap).min((work.bottom - height).max(work.top));
    Rect::new(x, y, x + width, y + height)
}

/// A box centred in `work`, its height capped at `max_fraction` of it.
pub fn centered(work: Rect, width: i32, height: i32, max_fraction: f32) -> Rect {
    let width = width.min(work.width());
    let height = height.min((work.height() as f32 * max_fraction) as i32);
    let x = work.left + (work.width() - width) / 2;
    let y = work.top + (work.height() - height) / 2;
    Rect::new(x, y, x + width, y + height)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: Rect = Rect::new(0, 0, 1920, 1040);

    #[test]
    fn line_positions() {
        assert_eq!(line_rect(WORK, 132, 14, Anchor::Center, 16), Rect::new(894, 4, 1026, 18));
        assert_eq!(line_rect(WORK, 132, 14, Anchor::Left, 16).left, 16);
        assert_eq!(line_rect(WORK, 132, 14, Anchor::Right, 16).right, 1904);
        // A taskbar at the top moves the work area down; the line follows.
        let below_taskbar = Rect::new(0, 48, 1920, 1080);
        assert_eq!(line_rect(below_taskbar, 132, 14, Anchor::Center, 16).top, 52);
        // A second monitor to the left has negative coordinates.
        let left_screen = Rect::new(-1280, 0, 0, 1024);
        assert_eq!(line_rect(left_screen, 132, 14, Anchor::Left, 16).left, -1264);
    }

    #[test]
    fn strip_follows_the_line_and_stays_on_screen() {
        let line = line_rect(WORK, 132, 14, Anchor::Right, 16);
        let strip = strip_rect(line, 700, 30, WORK, Anchor::Right, 6);
        assert_eq!(strip.right, line.right);
        assert_eq!(strip.top, line.bottom + 6);
        let left = strip_rect(line_rect(WORK, 132, 14, Anchor::Left, 16), 700, 30, WORK, Anchor::Left, 6);
        assert_eq!(left.left, 16);
        // Wider than the line: centred under it.
        let center = strip_rect(line_rect(WORK, 132, 14, Anchor::Center, 16), 700, 30, WORK, Anchor::Center, 6);
        assert_eq!(center.left, (1920 - 700) / 2);
        // Wider than the screen: clamped to it.
        let tiny = Rect::new(0, 0, 500, 400);
        let s = strip_rect(line_rect(tiny, 132, 14, Anchor::Right, 16), 700, 30, tiny, Anchor::Right, 6);
        assert_eq!((s.left, s.right), (0, 500));
    }

    #[test]
    fn centred_and_capped() {
        let r = centered(WORK, 720, 2000, 0.8);
        assert_eq!(r.height(), 832);
        assert_eq!(r.left, 600);
        assert_eq!(r.top, (1040 - 832) / 2);
        let small = centered(Rect::new(100, 50, 600, 450), 720, 300, 0.8);
        assert_eq!((small.left, small.right), (100, 600));
    }

    #[test]
    fn anchor_keys_round_trip() {
        for a in Anchor::ALL {
            assert_eq!(Anchor::from_key(a.key()), Some(a));
        }
        assert_eq!(Anchor::from_key("Centre"), Some(Anchor::Center));
        assert_eq!(Anchor::from_key("top"), None);
    }
}
