//! This index estimates scrollbar geometry. Reading position belongs to source anchors, not here.
#[derive(Debug, Clone)]
pub struct HeightIndex {
    values: Vec<f64>,
    tree: Vec<f64>,
}
impl HeightIndex {
    pub fn new(count: usize, estimate: f32) -> Self {
        let value = if estimate.is_finite() && estimate > 0.0 {
            estimate as f64
        } else {
            20.0
        };
        let mut h = Self {
            values: vec![0.0; count],
            tree: vec![0.0; count + 1],
        };
        for i in 0..count {
            h.set(i, value)
        }
        h
    }
    fn set(&mut self, index: usize, value: f64) {
        let delta = value - self.values[index];
        self.values[index] = value;
        let mut i = index + 1;
        while i < self.tree.len() {
            self.tree[i] += delta;
            i += i & (!i + 1);
        }
    }
    pub fn update(&mut self, index: usize, height: f32) -> Result<(), &'static str> {
        if index >= self.values.len() || !height.is_finite() || height <= 0.0 {
            return Err("invalid row height");
        }
        self.set(index, height as f64);
        Ok(())
    }
    pub fn prefix(&self, count: usize) -> f32 {
        let mut i = count.min(self.values.len());
        let mut sum = 0.0;
        while i > 0 {
            sum += self.tree[i];
            i -= i & (!i + 1);
        }
        sum as f32
    }
    pub fn total(&self) -> f32 {
        self.prefix(self.values.len())
    }
    pub fn locate(&self, y: f32) -> (usize, f32) {
        if self.values.is_empty() {
            return (0, 0.0);
        }
        let y = if y.is_finite() {
            y.max(0.0).min(self.total()) as f64
        } else {
            0.0
        };
        let mut index = 0;
        let mut sum = 0.0;
        let mut bit = 1usize;
        while bit < self.tree.len() {
            bit <<= 1;
        }
        while bit > 0 {
            let next = index + bit;
            if next < self.tree.len() && sum + self.tree[next] <= y {
                index = next;
                sum += self.tree[next];
            }
            bit >>= 1;
        }
        let i = index.min(self.values.len() - 1);
        (
            i,
            (y - self.prefix(i) as f64).max(0.0).min(self.values[i]) as f32,
        )
    }
}
