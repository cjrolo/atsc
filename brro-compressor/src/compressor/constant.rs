/// This compressor is for static values, it allows some level of non-static (1%)
use std::collections::HashMap;
use log::{info, debug};

const CONSTANT_COMPRESSOR_ID: u8 = 0;

/// This is a temporary implementation, other implementations (FFT, Polynomial) might provide the same result
/// as going through the data anyway.
pub struct Constant {
    pub id: u8,
    pub constant: i64,
    // Lets make the error, the number of residuals/frame lenght
    pub error: f32,
    pub residuals: Vec<(i32, i64)>,
}

impl Constant {
    /// Creates a new instance of the Constant compressor with the size needed to handle the worst case
    pub fn new(frame_size: usize) -> Self {
        debug!("Constant compressor");
        Constant {
            id: CONSTANT_COMPRESSOR_ID,
            constant: 0,
            error: 0.0,
            residuals: Vec::with_capacity(frame_size),
        }
    }

    /// This compressor is about having a single constant for the whole segment
    pub fn set_constant(&mut self, constant_value: i64) {
        self.constant = constant_value;
    }

    /// Adds a residual
    pub fn add_residual(&mut self, error_sample: i32, error_value: i64) {
        self.residuals.push((error_sample, error_value));
    }

    /// Sets the error value
    fn set_error(&mut self, data_lenght: usize){
        self.error = (self.residuals.len()/data_lenght) as f32;
    }

    /// Currently the data is provided in f64, this compressor needs i64. So the optimizer needs
    /// to get this out for the compressor
    pub fn optimize(data: &[f64]) -> Vec<i64> {
        let out_vec = Vec::with_capacity(data.len());
        out_vec
    }

    /// Compresses the data. Walks the data array and sets one value as the constant.
    /// TODO: VERY INEFFICIENT, we walk the data, and then the resulting map 2x. Find opportunities to improve
    /// TODO2: Fix residuals positions
    pub fn compress(&mut self, data: &[i64]) {
        // Count occurrences of each value in the data
        let mut seen_values = HashMap::new();
        for &val in data {
            *seen_values.entry(val).or_insert(0) += 1;
        }

        // Find the constant and residuals in a single iteration
        let mut constant = 0;
        let mut hit_count = 0;
        let mut residuals = Vec::new();

        for (&val, &count) in &seen_values {
            if count > hit_count {
                constant = val;
                hit_count = count;
            } else {
                // Push residuals as (sample_number, value) pairs
                for _ in 0..count {
                    residuals.push((0, val)); // Sample number is set to 0 for now
                }
            }
        }

        self.set_constant(constant);
        self.residuals = residuals;
        self.set_error(data.len());
    }

    /// This function transforms the structure into a Binary stream to be appended to the frame
    pub fn to_bytes(self) -> Vec<u8> {
        // Use Bincode and flate2-rs? Do this at the Stream Level?
        Vec::new()
    }
}

pub fn constant(data: &[f64]) -> Vec<u8> {
    info!("[Compressor] Initializing Constant Compressor");
    // Initialize the compressor
    let mut c = Constant::new(data.len());
    // Convert the data via the optimizer
    c.compress(&Constant::optimize(data));
    // Convert to bytes
    c.to_bytes()
}
// TODO: WRITE A TESTTTTTTTTTT!!!!!!!!!!!!!!