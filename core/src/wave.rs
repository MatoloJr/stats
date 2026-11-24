//! The activity line's waveform: a voice-meter style row of bars driven by
//! live system activity. Pure maths, so it is tested here and reused by any
//! platform; the UI only draws `heights()`.
//!
//! Samples arrive about 10 times a second (`push`). Each one enters on the
//! right and scrolls left, after VU-style smoothing: a fast attack so a spike
//! shows at once, a slow decay so it doesn't flicker. Frames between samples
//! ease the drawn heights towards their targets (`advance`), so the motion
//! stays smooth at any frame rate.

/// How busy the machine is right now, 0..1, from CPU percent and disk and
/// network activity already normalised to 0..1. A square-root curve makes
/// light load visible without letting heavy load flatten everything.
pub fn activity(cpu_percent: Option<f64>, disk: f32, net: f32) -> f32 {
    let cpu = (cpu_percent.unwrap_or(0.0) / 100.0) as f32;
    let raw = cpu.max(0.7 * disk).max(0.7 * net).clamp(0.0, 1.0);
    raw.sqrt()
}

/// `value` against its recent peak, with a floor so idle noise (a few KB/s)
/// does not look like a burst.
pub fn normalize(value: Option<f64>, recent_max: Option<f32>, floor: f32) -> f32 {
    let Some(v) = value else { return 0.0 };
    let scale = recent_max.unwrap_or(0.0).max(floor);
    (v as f32 / scale).clamp(0.0, 1.0)
}

pub struct Wave {
    targets: Vec<f32>,
    shown: Vec<f32>,
    envelope: f32,
    seed: u32,
}

const ATTACK: f32 = 0.65;
const DECAY: f32 = 0.18;
/// Drawn heights reach about 63% of the way to a new target in this time.
const EASE_MS: f32 = 70.0;
/// Below this difference nothing visibly moves.
const STILL: f32 = 0.004;

impl Wave {
    pub fn new(columns: usize) -> Self {
        let columns = columns.max(1);
        Wave { targets: vec![0.0; columns], shown: vec![0.0; columns], envelope: 0.0, seed: 0x9E37_79B9 }
    }

    pub fn columns(&self) -> usize {
        self.targets.len()
    }

    /// Small deterministic jitter so bars differ like a voice meter's.
    fn texture(&mut self) -> f32 {
        // xorshift32: cheap, no allocation, no dependency.
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        0.55 + 0.45 * (self.seed % 1000) as f32 / 999.0
    }

    /// A new activity sample (0..1): scrolls everything one column left.
    pub fn push(&mut self, level: f32) {
        let level = if level.is_finite() { level.clamp(0.0, 1.0) } else { 0.0 };
        let rate = if level > self.envelope { ATTACK } else { DECAY };
        self.envelope += (level - self.envelope) * rate;
        let texture = self.texture();
        self.targets.rotate_left(1);
        self.shown.rotate_left(1);
        let last = self.targets.len() - 1;
        self.targets[last] = (self.envelope * texture).clamp(0.0, 1.0);
    }

    /// Move the drawn heights towards their targets for `dt_ms` of time.
    pub fn advance(&mut self, dt_ms: f32) {
        let k = 1.0 - (-dt_ms.max(0.0) / EASE_MS).exp();
        for (shown, target) in self.shown.iter_mut().zip(&self.targets) {
            *shown += (target - *shown) * k;
            if (target - *shown).abs() < STILL {
                *shown = *target;
            }
        }
    }

    /// Heights to draw, 0..1, oldest on the left.
    pub fn heights(&self) -> &[f32] {
        &self.shown
    }

    /// Nothing is moving and nothing is about to: the UI can slow its frame
    /// rate right down.
    pub fn is_settled(&self) -> bool {
        let still = self.shown.iter().zip(&self.targets).all(|(s, t)| (s - t).abs() < STILL);
        let flat = self.targets.iter().all(|&t| t < 0.03);
        still && flat
    }

    /// The smoothed level, 0..1 (for an overall glow or tint).
    pub fn level(&self) -> f32 {
        self.envelope
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_curve() {
        assert_eq!(activity(None, 0.0, 0.0), 0.0);
        assert!((activity(Some(100.0), 0.0, 0.0) - 1.0).abs() < 1e-6);
        // 4% CPU is still clearly visible (0.2 of full height).
        assert!((activity(Some(4.0), 0.0, 0.0) - 0.2).abs() < 1e-3);
        // A disk burst counts even with an idle CPU.
        assert!(activity(Some(1.0), 1.0, 0.0) > 0.8);
        assert!(activity(Some(500.0), 9.0, 9.0) <= 1.0);
    }

    #[test]
    fn normalize_has_a_floor() {
        assert_eq!(normalize(None, Some(10.0), 1.0), 0.0);
        assert_eq!(normalize(Some(5_000.0), Some(0.0), 1_000_000.0), 0.005);
        assert_eq!(normalize(Some(5.0e6), Some(10.0e6), 1.0e6), 0.5);
        assert_eq!(normalize(Some(5.0e7), Some(10.0e6), 1.0e6), 1.0);
    }

    #[test]
    fn spikes_attack_fast_and_decay_slowly() {
        let mut w = Wave::new(8);
        w.push(1.0);
        let after_spike = w.level();
        assert!(after_spike > 0.6, "{after_spike}");
        w.push(0.0);
        let after_drop = w.level();
        assert!(after_drop > 0.5 * after_spike, "decay too fast: {after_drop}");
        for _ in 0..40 {
            w.push(0.0);
        }
        assert!(w.level() < 0.01);
    }

    #[test]
    fn newest_sample_enters_on_the_right_and_heights_ease() {
        let mut w = Wave::new(4);
        w.push(1.0);
        assert_eq!(w.heights(), &[0.0; 4]);
        w.advance(1_000.0);
        let h = w.heights();
        assert!(h[3] > 0.3 && h[..3].iter().all(|&v| v == 0.0), "{h:?}");
        w.push(1.0);
        w.advance(1_000.0);
        assert!(w.heights()[2] > 0.3, "the first sample scrolled left");
        assert!(w.heights().iter().all(|&v| (0.0..=1.0).contains(&v)));
    }

    #[test]
    fn idle_settles_so_the_ui_can_slow_down() {
        let mut w = Wave::new(16);
        assert!(w.is_settled());
        for _ in 0..5 {
            w.push(0.9);
            w.advance(33.0);
        }
        assert!(!w.is_settled());
        for _ in 0..60 {
            w.push(0.0);
            w.advance(100.0);
        }
        assert!(w.is_settled());
    }

    #[test]
    fn bad_input_is_ignored() {
        let mut w = Wave::new(3);
        w.push(f32::NAN);
        w.push(-4.0);
        w.advance(f32::INFINITY);
        assert!(w.heights().iter().all(|v| v.is_finite()));
    }
}
