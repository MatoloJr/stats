//! Fixed-size history rings for sparklines. Memory never grows past the
//! capacity chosen from `history_seconds`.

use std::collections::VecDeque;

#[derive(Clone, Debug)]
pub struct Ring {
    values: VecDeque<f32>,
    capacity: usize,
}

impl Ring {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.clamp(2, 3600);
        Self { values: VecDeque::with_capacity(capacity), capacity }
    }

    /// Missing samples are stored as NaN so the sparkline shows a gap rather
    /// than a fake zero.
    pub fn push(&mut self, value: Option<f64>) {
        if self.values.len() == self.capacity {
            self.values.pop_front();
        }
        self.values.push_back(value.map(|v| v as f32).unwrap_or(f32::NAN));
    }

    pub fn resize(&mut self, capacity: usize) {
        let capacity = capacity.clamp(2, 3600);
        while self.values.len() > capacity {
            self.values.pop_front();
        }
        self.capacity = capacity;
    }

    pub fn values(&self) -> impl Iterator<Item = f32> + '_ {
        self.values.iter().copied()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn max(&self) -> Option<f32> {
        self.values.iter().copied().filter(|v| v.is_finite()).reduce(f32::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_caps_length() {
        let mut ring = Ring::new(3);
        for i in 0..10 {
            ring.push(Some(i as f64));
        }
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.values().collect::<Vec<_>>(), vec![7.0, 8.0, 9.0]);
        assert_eq!(ring.max(), Some(9.0));
    }

    #[test]
    fn gaps_are_nan_and_ignored_by_max() {
        let mut ring = Ring::new(4);
        ring.push(None);
        ring.push(Some(2.0));
        assert!(ring.values().next().unwrap().is_nan());
        assert_eq!(ring.max(), Some(2.0));
        ring.resize(1);
        assert_eq!(ring.capacity(), 2);
    }
}
