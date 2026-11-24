//! Where the activity line, the readings strip and the dashboard go on a
//! screen. Plain rectangles in screen pixels, no OS calls.
//!
//! The line lives on one edge: the top (centred), or the left or right side
//! (vertically centred, drawn vertically). Its window is always island-sized;
//! at rest only a hairline along the screen edge is drawn and the window lets
//! clicks through, so it never blocks what is underneath.

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

/// Which screen edge the line sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Anchor {
    #[default]
    Top,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

impl Anchor {
    pub const ALL: [Anchor; 3] = [Anchor::Top, Anchor::Left, Anchor::Right];

    pub fn key(self) -> &'static str {
        match self {
            Anchor::Top => "top",
            Anchor::Left => "left",
            Anchor::Right => "right",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Anchor::Top => "Top",
            Anchor::Left => "Left side",
            Anchor::Right => "Right side",
        }
    }

    /// Also accepts the 0.1 names: `center` was the top-centre line.
    pub fn from_key(text: &str) -> Option<Anchor> {
        match text.trim().to_ascii_lowercase().as_str() {
            "top" | "center" | "centre" | "middle" | "t" | "c" => Some(Anchor::Top),
            "left" | "l" => Some(Anchor::Left),
            "right" | "r" => Some(Anchor::Right),
            _ => None,
        }
    }

    pub fn orientation(self) -> Orientation {
        match self {
            Anchor::Top => Orientation::Horizontal,
            Anchor::Left | Anchor::Right => Orientation::Vertical,
        }
    }
}

/// The line's window. `length` runs along the edge, `depth` away from it.
pub fn line_rect(work: Rect, length: i32, depth: i32, anchor: Anchor) -> Rect {
    match anchor {
        Anchor::Top => {
            let length = length.min(work.width());
            let x = work.left + (work.width() - length) / 2;
            Rect::new(x, work.top, x + length, work.top + depth)
        }
        Anchor::Left | Anchor::Right => {
            let length = length.min(work.height());
            let y = work.top + (work.height() - length) / 2;
            let x = if anchor == Anchor::Left { work.left } else { work.right - depth };
            Rect::new(x, y, x + depth, y + length)
        }
    }
}

/// The strip of `edge` pixels of `line` that touches the screen edge: where
/// the hairline is drawn and where the pointer must rest to wake it.
pub fn edge_band(line: Rect, anchor: Anchor, edge: i32) -> Rect {
    match anchor {
        Anchor::Top => Rect::new(line.left, line.top, line.right, line.top + edge),
        Anchor::Left => Rect::new(line.left, line.top, line.left + edge, line.bottom),
        Anchor::Right => Rect::new(line.right - edge, line.top, line.right, line.bottom),
    }
}

/// The strip: under a top line, beside a side line; always on screen.
pub fn strip_rect(line: Rect, width: i32, height: i32, work: Rect, anchor: Anchor, gap: i32) -> Rect {
    let width = width.min(work.width());
    let height = height.min(work.height());
    let (x, y) = match anchor {
        Anchor::Top => (line.left + (line.width() - width) / 2, line.bottom + gap),
        Anchor::Left => (line.right + gap, line.top + (line.height() - height) / 2),
        Anchor::Right => (line.left - gap - width, line.top + (line.height() - height) / 2),
    };
    let x = x.clamp(work.left, (work.right - width).max(work.left));
    let y = y.clamp(work.top, (work.bottom - height).max(work.top));
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
    fn top_is_centred_and_flush() {
        let r = line_rect(WORK, 260, 34, Anchor::Top);
        assert_eq!(r, Rect::new(830, 0, 1090, 34));
        // A taskbar at the top moves the work area down; the line follows.
        let below_taskbar = Rect::new(0, 48, 1920, 1080);
        assert_eq!(line_rect(below_taskbar, 260, 34, Anchor::Top).top, 48);
    }

    #[test]
    fn sides_are_vertical_and_flush() {
        let l = line_rect(WORK, 260, 34, Anchor::Left);
        assert_eq!(l, Rect::new(0, 390, 34, 650));
        assert!(l.height() > l.width());
        let r = line_rect(WORK, 260, 34, Anchor::Right);
        assert_eq!((r.left, r.right), (1886, 1920));
        // Taskbar on the left: the work area starts further right.
        let taskbar_left = Rect::new(62, 0, 1920, 1080);
        assert_eq!(line_rect(taskbar_left, 260, 34, Anchor::Left).left, 62);
        // A second monitor to the left has negative coordinates.
        let left_screen = Rect::new(-1280, 0, 0, 1024);
        assert_eq!(line_rect(left_screen, 260, 34, Anchor::Right).right, 0);
        assert_eq!(Anchor::Left.orientation(), Orientation::Vertical);
        assert_eq!(Anchor::Top.orientation(), Orientation::Horizontal);
    }

    #[test]
    fn edge_band_hugs_the_screen_edge() {
        let top = line_rect(WORK, 260, 34, Anchor::Top);
        assert_eq!(edge_band(top, Anchor::Top, 6), Rect::new(830, 0, 1090, 6));
        let right = line_rect(WORK, 260, 34, Anchor::Right);
        assert_eq!(edge_band(right, Anchor::Right, 6), Rect::new(1914, 390, 1920, 650));
    }

    #[test]
    fn strip_goes_under_or_beside_the_line() {
        let top = line_rect(WORK, 260, 34, Anchor::Top);
        let s = strip_rect(top, 700, 34, WORK, Anchor::Top, 4);
        assert_eq!((s.left, s.top), ((1920 - 700) / 2, 38));
        let left = line_rect(WORK, 260, 34, Anchor::Left);
        let s = strip_rect(left, 700, 34, WORK, Anchor::Left, 4);
        assert_eq!(s.left, 38);
        assert_eq!(s.top + s.height() / 2, left.top + left.height() / 2);
        let right = line_rect(WORK, 260, 34, Anchor::Right);
        let s = strip_rect(right, 700, 34, WORK, Anchor::Right, 4);
        assert_eq!(s.right, right.left - 4);
        // Wider than the screen: clamped to it.
        let tiny = Rect::new(0, 0, 500, 400);
        let s = strip_rect(line_rect(tiny, 260, 34, Anchor::Top), 700, 34, tiny, Anchor::Top, 4);
        assert_eq!((s.left, s.right), (0, 500));
    }

    #[test]
    fn centred_and_capped() {
        let r = centered(WORK, 720, 2000, 0.8);
        assert_eq!(r.height(), 832);
        assert_eq!(r.left, 600);
        let small = centered(Rect::new(100, 50, 600, 450), 720, 300, 0.8);
        assert_eq!((small.left, small.right), (100, 600));
    }

    #[test]
    fn anchor_keys_round_trip_and_migrate() {
        for a in Anchor::ALL {
            assert_eq!(Anchor::from_key(a.key()), Some(a));
        }
        assert_eq!(Anchor::from_key("center"), Some(Anchor::Top));
        assert_eq!(Anchor::from_key("Right"), Some(Anchor::Right));
        assert_eq!(Anchor::from_key("bottom"), None);
    }
}
