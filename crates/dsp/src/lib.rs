pub struct Fir {
    pub tap: Vec<f32>,
    pub buf: Vec<f32>,
}

impl Fir {
    pub fn new(tap: Vec<f32>) -> Self {
        let buf = vec![0.0; tap.len()];
        Fir { tap, buf }
    }

    /// 移動平均
    pub fn new_moving(n: usize) -> Self {
        // Create a moving average filter with n taps
        let tap = vec![1.0 / n as f32; n];
        let buf = vec![0.0; n];
        Fir { tap, buf }
    }

    pub fn filter(&mut self, input: f32) -> f32 {
        // Shift buffer
        for i in (1..self.buf.len()).rev() {
            self.buf[i] = self.buf[i - 1];
        }
        self.buf[0] = input;

        // Apply FIR filter
        let mut output = 0.0;
        for i in 0..self.tap.len() {
            output += self.tap[i] * self.buf[i];
        }
        output
    }

    pub fn filter_vec(&mut self, input: &[f32]) -> Vec<f32> {
        input.iter().map(|&x| self.filter(x)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fir_filter() {
        let mut fir = Fir::new(vec![0.2, 0.5, 0.3]);
        let input = vec![1.0, 2.0, 3.0, 4.0];
        let output = fir.filter_vec(&input);
        approx::assert_abs_diff_eq!(output.as_slice(), vec![0.2, 0.9, 1.9, 2.9].as_slice());
    }

    #[test]
    fn test_moving_average() {
        let mut fir = Fir::new_moving(3);
        let input = vec![1.0, 1.0, 1.0, 1.0];
        let output = fir.filter_vec(&input);
        assert_eq!(output, vec![1.0 / 3.0, 2.0 / 3.0, 1.0, 1.0]);
    }
}
