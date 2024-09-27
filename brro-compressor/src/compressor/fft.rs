/*
Copyright 2024 NetApp, Inc.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    https://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
*/

use crate::{
    optimizer::utils::DataStats,
    utils::{error::calculate_error, next_size},
};
use bincode::{Decode, Encode};
use rustfft::{num_complex::Complex, FftPlanner};
use std::{cmp::Ordering, collections::BinaryHeap};

use super::{BinConfig, CompressorResult};
use log::{debug, error, info, trace, warn};

const FFT_COMPRESSOR_ID: u8 = 15;
const DECIMAL_PRECISION: u8 = 5;

/// Struct to store frequencies, since bincode can't encode num_complex Complex format, this one is compatible
// This could be a Generic to support f64, integers, etc...
#[derive(Encode, Decode, Debug, Copy, Clone)]
pub struct FrequencyPoint {
    /// Frequency position
    pos: u16, // This is the reason that frame size is limited to 65535, probably enough
    freq_real: f32,
    freq_img: f32,
}

impl FrequencyPoint {
    pub fn new(real: f32, img: f32) -> Self {
        FrequencyPoint {
            pos: 0,
            freq_real: real,
            freq_img: img,
        }
    }

    pub fn with_position(real: f32, img: f32, pos: u16) -> Self {
        FrequencyPoint {
            pos,
            freq_real: real,
            freq_img: img,
        }
    }

    pub fn from_complex(complex: Complex<f32>) -> Self {
        FrequencyPoint {
            pos: 0,
            freq_real: complex.re,
            freq_img: complex.im,
        }
    }

    pub fn from_complex_with_position(complex: Complex<f32>, pos: u16) -> Self {
        FrequencyPoint {
            pos,
            freq_real: complex.re,
            freq_img: complex.im,
        }
    }

    pub fn to_complex(self) -> Complex<f32> {
        Complex {
            re: self.freq_real,
            im: self.freq_img,
        }
    }

    pub fn to_inv_complex(self) -> Complex<f32> {
        Complex {
            re: self.freq_real,
            im: self.freq_img * -1.0,
        }
    }
}

// This is VERY specific for this use case, DO NOT RE-USE! This NORM comparison is false for complex numbers
impl PartialEq for FrequencyPoint {
    fn eq(&self, other: &Self) -> bool {
        let c1 = Complex {
            re: self.freq_real,
            im: self.freq_img,
        };
        let c2 = Complex {
            re: other.freq_real,
            im: other.freq_img,
        };
        c1.norm() == c2.norm()
    }
}

impl Eq for FrequencyPoint {}

impl PartialOrd for FrequencyPoint {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FrequencyPoint {
    fn cmp(&self, other: &Self) -> Ordering {
        let c1 = Complex {
            re: self.freq_real,
            im: self.freq_img,
        };
        let c2 = Complex {
            re: other.freq_real,
            im: other.freq_img,
        };
        if self == other {
            Ordering::Equal
        } else if c1.norm() > c2.norm() {
            return Ordering::Greater;
        } else {
            return Ordering::Less;
        }
    }
}

/// FFT Compressor. Applies FFT to a signal, picks the N best frequencies, discards the rest. Always LOSSY
#[derive(PartialEq, Debug)]
pub struct FFT {
    /// Compressor ID
    pub id: u8,
    /// Stored frequencies
    pub frequencies: Vec<FrequencyPoint>,
    /// The maximum numeric value of the points in the frame
    pub max_value: f32,
    /// The minimum numeric value of the points in the frame
    pub min_value: f32,
    /// Compression error
    pub error: Option<f64>,
}

// Implementing the Encode manually because we don't want to encode the Error field, less bytes used.
impl Encode for FFT {
    fn encode<__E: ::bincode::enc::Encoder>(
        &self,
        encoder: &mut __E,
    ) -> Result<(), ::bincode::error::EncodeError> {
        Encode::encode(&self.id, encoder)?;
        Encode::encode(&self.frequencies, encoder)?;
        Encode::encode(&self.max_value, encoder)?;
        Encode::encode(&self.min_value, encoder)?;
        Ok(())
    }
}

impl Decode for FFT {
    fn decode<__D: ::bincode::de::Decoder>(
        decoder: &mut __D,
    ) -> Result<Self, ::bincode::error::DecodeError> {
        Ok(Self {
            id: Decode::decode(decoder)?,
            frequencies: Decode::decode(decoder)?,
            max_value: Decode::decode(decoder)?,
            min_value: Decode::decode(decoder)?,
            error: None,
        })
    }
}

impl<'__de> ::bincode::BorrowDecode<'__de> for FFT {
    fn borrow_decode<__D: ::bincode::de::BorrowDecoder<'__de>>(
        decoder: &mut __D,
    ) -> Result<Self, ::bincode::error::DecodeError> {
        Ok(Self {
            id: ::bincode::BorrowDecode::borrow_decode(decoder)?,
            frequencies: ::bincode::BorrowDecode::borrow_decode(decoder)?,
            max_value: ::bincode::BorrowDecode::borrow_decode(decoder)?,
            min_value: ::bincode::BorrowDecode::borrow_decode(decoder)?,
            error: None,
        })
    }
}

impl FFT {
    /// Creates a new instance of the Constant compressor with the size needed to handle the worst case
    pub fn new(sample_count: usize, min: f64, max: f64) -> Self {
        debug!("FFT compressor: min:{} max:{}", min, max);
        FFT {
            id: FFT_COMPRESSOR_ID,
            frequencies: Vec::with_capacity(sample_count),
            max_value: FFT::f64_to_f32(max),
            min_value: FFT::f64_to_f32(min),
            error: None,
        }
    }

    fn f64_to_f32(x: f64) -> f32 {
        let y = x as f32;
        if !(x.is_finite() && y.is_finite()) {
            // PANIC? Error?
            error!("f32 overflow during conversion");
        }
        y
    }

    /// Given an array of size N, it returns the next best FFT size with the
    /// begining and the ended padded to improve Gibbs on the edges of the frame
    fn gibbs_sizing(data: &[f64]) -> Vec<f64> {
        let data_len = data.len();
        let added_len = next_size(data_len) - data_len;
        debug!("Gibbs sizing, padding with {}", added_len);
        let prefix_len = added_len / 2;
        let suffix_len = added_len - prefix_len;
        // Extend the beginning and the end with the first and last value
        let mut prefix = vec![data[0]; prefix_len];
        let suffix = vec![*data.last().unwrap(); suffix_len];
        prefix.extend(data);
        prefix.extend(suffix);
        trace!("Gibbs constructed data: {:?}", prefix);
        prefix
    }

    /// Rounds a number to the specified number of decimal places
    // TODO: Move this into utils? I think this will be helpfull somewhere else.
    fn round(&self, x: f32, decimals: u32) -> f64 {
        let y = 10i32.pow(decimals) as f64;
        let out = (x as f64 * y).round() / y;
        if out > self.max_value as f64 {
            return self.max_value as f64;
        }
        if out < self.min_value as f64 {
            return self.min_value as f64;
        }
        out
    }

    // Converts an f64 vec to an Vec of Complex F32
    fn optimize(data: &[f64]) -> Vec<Complex<f32>> {
        data.iter()
            .map(|x| Complex {
                re: FFT::f64_to_f32(*x),
                im: 0.0f32,
            })
            .collect()
    }

    /// Removes the smallest frequencies from `buffer` until `max_freq` remain
    fn fft_trim(buffer: &mut [Complex<f32>], max_freq: usize) -> Vec<FrequencyPoint> {
        let mut freq_vec = Vec::with_capacity(max_freq);
        if max_freq == 1 {
            freq_vec.push(FrequencyPoint::from_complex_with_position(buffer[0], 0));
            return freq_vec;
        }
        // More than 1 frequency needed, get the biggest frequencies now.
        // Move from the buffer into Frequency Vectors
        let tmp_vec: Vec<FrequencyPoint> = buffer
            .iter()
            .enumerate()
            .map(|(pos, &f)| FrequencyPoint::from_complex_with_position(f, pos as u16))
            .collect();
        // This part, is because Binary heap is very good at "give me the top N elements"
        let mut heap = BinaryHeap::from(tmp_vec);
        // Now that we have it, let's pop the elements we need!
        for _ in 0..max_freq {
            if let Some(item) = heap.pop() {
                // If the frequency is 0, we don't need it or any other
                if item.freq_img == 0.0 && item.freq_real == 0.0 {
                    break;
                }
                freq_vec.push(item)
            }
        }
        freq_vec
    }

    /// Compress data via FFT.
    /// This picks a set of data, computes the FFT, and uses the hinted number of frequencies to store the N provided
    /// more relevant frequencies
    pub fn compress_hinted(&mut self, data: &[f64], max_freq: usize) {
        if self.max_value == self.min_value {
            debug!("Same max and min, we're done here!");
            return;
        }
        // First thing, always try to get the data len as a power of 2.
        let v = data.len();
        if !v.is_power_of_two() {
            warn!("Slow FFT, data segment is not a power of 2!");
        }
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(v);
        let mut buffer = FFT::optimize(data);
        // The data is processed in place, it gets back to the buffer
        fft.process(&mut buffer);
        // We need half + 1 frequencies at most, due to the mirrored nature of FFT (signal is always real!)
        // and the first one being the dc component
        let size = (buffer.len() / 2) + 1;
        buffer.truncate(size);
        self.frequencies = FFT::fft_trim(&mut buffer, max_freq);
    }

    /// Compress data via FFT - EXPENSIVE
    /// This picks a set of data, computes the FFT, and optimizes the number of frequencies to store to match
    /// the max allowed error.
    /// NOTE: This does not otimize for smallest possible error, just being smaller than the error.
    pub fn compress_bounded(&mut self, data: &[f64], max_err: f64) {
        if self.max_value == self.min_value {
            debug!("Same max and min, we're done here!");
            return;
        }

        if !data.len().is_power_of_two() {
            warn!("Slow FFT, data segment is not a power of 2!");
        }
        // Let's start from the defaults values for frequencies
        let max_freq = if 3 >= (data.len() / 100) {
            3
        } else {
            data.len() / 100
        };

        // Should we apply a Gibbs sizing?
        let g_data: Vec<f64> = if data.len() >= 128 {
            FFT::gibbs_sizing(data)
        } else {
            data.to_vec()
        };

        let len = g_data.len();
        let len_f32 = len as f32;

        // Clean the data
        let mut buffer = FFT::optimize(&g_data);

        // Create the FFT planners
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(len);
        let ifft = planner.plan_fft_inverse(len);

        // FFT calculations
        fft.process(&mut buffer);
        let mut buff_clone = buffer.clone();
        // We need half + 1 frequencies at most, due to the mirrored nature of FFT (signal is always real!)
        // and the first one being the dc component
        let size = (buff_clone.len() / 2) + 1;
        buff_clone.truncate(size);
        // To make sure we run the first cycle
        let mut current_err = max_err + 1.0;
        let mut jump: usize = 0;
        let mut iterations = 0;
        // Aproximation. Faster convergence
        while ((max_err * 1000.0) as i32) < ((current_err * 1000.0) as i32) {
            iterations += 1;
            self.frequencies = FFT::fft_trim(&mut buff_clone, max_freq + jump);
            // Inverse FFT and error check
            let mut idata = self.get_mirrored_freqs(len);
            // run the ifft
            ifft.process(&mut idata);
            let out_data: Vec<f64> = idata
                .iter()
                .map(|&f| self.round(f.re / len_f32, DECIMAL_PRECISION.into()))
                .collect();
            current_err = calculate_error(&g_data, &out_data);
            trace!("Current Err: {}", current_err);
            // Max iterations is 22 (We start at 10%, we can go to 95% and 1% at a time)
            match iterations {
                1..=17 => jump += (max_freq / 2).max(1),
                18..=22 => jump += (max_freq / 10).max(1),
                _ => break,
            }
        }
        self.error = Some(current_err);
        debug!(
            "Iterations to convergence: {}, Freqs P:{} S:{}, Error: {}",
            iterations,
            jump + max_freq,
            self.frequencies.len(),
            current_err
        );
    }

    /// Compresses data via FFT
    /// The set of frequencies to store is 1/100 of the data lenght OR 3, which is bigger.
    pub fn compress(&mut self, data: &[f64]) {
        if self.max_value == self.min_value {
            debug!("Same max and min, we're done here!");
            return;
        }
        // First thing, always try to get the data len as a power of 2.
        let v = data.len();
        let max_freq = if 3 >= (v / 100) { 3 } else { v / 100 };
        debug!("Setting max_freq count to: {}", max_freq);
        if !v.is_power_of_two() {
            warn!("Slow FFT, data segment is not a power of 2!");
        }
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(v);
        let mut buffer = FFT::optimize(data);
        // The data is processed in place, it gets back to the buffer
        fft.process(&mut buffer);
        // We need half + 1 frequencies at most, due to the mirrored nature of FFT (signal is always real!)
        // and the first one being the dc component
        let size = (buffer.len() / 2) + 1;
        buffer.truncate(size);
        self.frequencies = FFT::fft_trim(&mut buffer, max_freq);
    }

    /// Decompresses data
    pub fn decompress(data: &[u8]) -> Self {
        let config = BinConfig::get();
        let (fft, _) = bincode::decode_from_slice(data, config).unwrap();
        fft
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let config = BinConfig::get();
        bincode::encode_to_vec(self, config).unwrap()
    }

    /// Gets the full sized array with the frequencies mirrored
    fn get_mirrored_freqs(&self, len: usize) -> Vec<Complex<f32>> {
        // Because we are dealing with Real inputs, we only store half the frequencies, but
        // we need all for the ifft
        let mut data = vec![
            Complex {
                re: 0.0f32,
                im: 0.0f32
            };
            len
        ];
        for f in &self.frequencies {
            let pos = f.pos as usize;
            data[pos] = f.to_complex();
            // fo doesn't mirror
            if pos == 0 {
                continue;
            }
            // Mirror and invert the imaginary part
            data[len - pos] = f.to_inv_complex()
        }
        data
    }

    /// Returns an array of data
    /// Runs the ifft, and push residuals into place and/or adjusts max and mins accordingly
    pub fn to_data(&self, frame_size: usize) -> Vec<f64> {
        if self.max_value == self.min_value {
            debug!("Same max and min, faster decompression!");
            return vec![self.max_value as f64; frame_size];
        }
        // Was this processed to reduce the Gibbs phenomeon?
        let trim_sizes = if frame_size >= 128 {
            let added_len = next_size(frame_size) - frame_size;
            let prefix_len = added_len / 2;
            let suffix_len = added_len - prefix_len;
            debug!(
                "Gibbs sizing detected, removing padding with {} len",
                added_len
            );
            (prefix_len, suffix_len)
        } else {
            (0, 0)
        };
        let gibbs_frame_size = frame_size + trim_sizes.0 + trim_sizes.1;
        // Vec to process the ifft
        let mut data = self.get_mirrored_freqs(gibbs_frame_size);
        // Plan the ifft
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_inverse(gibbs_frame_size);
        // run the ifft
        fft.process(&mut data);
        // We need this for normalization
        let len = gibbs_frame_size as f32;
        // We only need the real part
        let out_data: Vec<f64> = data
            .iter()
            .map(|&f| self.round(f.re / len, DECIMAL_PRECISION.into()))
            .collect();
        // Trim the excess data
        let trimmed_data = out_data[trim_sizes.0..out_data.len() - trim_sizes.1].to_vec();
        trimmed_data
    }
}

/// Compresses a data segment via FFT.
pub fn fft(data: &[f64]) -> Vec<u8> {
    info!("Initializing FFT Compressor");
    let mut min = data[0];
    let mut max = data[0];
    for e in data.iter() {
        if e > &max {
            max = *e
        };
        if e < &min {
            min = *e
        };
    }
    // Initialize the compressor
    let mut c = FFT::new(data.len(), min, max);
    // Convert the data
    c.compress(data);
    // Convert to bytes
    c.to_bytes()
}

/// Uncompress a FFT data
pub fn fft_to_data(sample_number: usize, compressed_data: &[u8]) -> Vec<f64> {
    let c = FFT::decompress(compressed_data);
    c.to_data(sample_number)
}

/// Compress targeting a specific max error allowed. This is very computational intensive,
/// as the FFT will be calculated over and over until the specific error threshold is achived.
pub fn fft_allowed_error(data: &[f64], allowed_error: f64) -> CompressorResult {
    info!("Initializing FFT Compressor. Max error: {}", allowed_error);
    let mut min = data[0];
    let mut max = data[0];
    for e in data.iter() {
        if e > &max {
            max = *e
        };
        if e < &min {
            min = *e
        };
    }
    // Initialize the compressor
    let mut c = FFT::new(data.len(), min, max);
    // Convert the data
    c.compress_bounded(data, allowed_error);
    // Convert to bytes
    CompressorResult::new(c.to_bytes(), c.error.unwrap_or(0.0))
}

/// Compress targeting a specific max error allowed. This is very computational intensive,
/// as the FFT will be calculated over and over until the specific error threshold is achived.
pub fn fft_compressor(data: &[f64], allowed_error: f64, stats: DataStats) -> CompressorResult {
    debug!("Initializing FFT Compressor. Error and Stats provided");
    // Initialize the compressor
    let mut c = FFT::new(data.len(), stats.min, stats.max);
    // Convert the data
    c.compress_bounded(data, allowed_error);
    // Convert to bytes
    CompressorResult::new(c.to_bytes(), c.error.unwrap_or(0.0))
}

pub fn fft_set(data: &[f64], freqs: usize) -> Vec<u8> {
    info!("Initializing FFT Compressor");
    let mut min = data[0];
    let mut max = data[0];
    for e in data.iter() {
        if e > &max {
            max = *e
        };
        if e < &min {
            min = *e
        };
    }
    // Initialize the compressor
    let mut c = FFT::new(data.len(), min, max);
    // Convert the data
    c.compress_hinted(data, freqs);
    // Convert to bytes
    c.to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fft() {
        let vector1 = vec![1.0, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 3.0, 1.0, 1.0, 5.0];
        assert_eq!(
            fft_set(&vector1, 2),
            [
                15, 2, 0, 0, 0, 152, 65, 0, 0, 0, 0, 4, 0, 0, 96, 192, 102, 144, 138, 64, 0, 0,
                160, 64, 0, 0, 128, 63
            ]
        );
    }

    #[test]
    fn test_to_lossless_data() {
        let vector1 = vec![1.0, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 3.0, 1.0, 1.0, 5.0];
        let compressed_data = fft_set(&vector1, 12);
        let out = fft_to_data(vector1.len(), &compressed_data);
        assert_eq!(vector1, out);
    }

    #[test]
    fn test_to_lossy_data() {
        let vector1 = vec![1.0, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 3.0, 1.0, 1.0, 5.0];
        let lossy_vec = vec![
            1.0, 1.87201, 2.25, 1.0, 1.82735, 1.689, 1.82735, 1.0, 2.75, 1.189, 1.0, 3.311,
        ];
        let compressed_data = fft(&vector1);
        let out = fft_to_data(vector1.len(), &compressed_data);
        assert_eq!(lossy_vec, out);
    }

    #[test]
    fn test_to_allowed_error() {
        let vector1 = vec![1.0, 1.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 3.0, 1.0, 1.0, 5.0];
        let frame_size = vector1.len();
        let compressed_result = fft_allowed_error(&vector1, 0.01);
        let out = FFT::decompress(&compressed_result.compressed_data).to_data(frame_size);
        let e = calculate_error(&vector1, &out);
        assert!(e <= 0.01);
    }

    #[test]
    fn test_gibbs_sizing() {
        let mut vector1 = vec![2.0; 2048];
        vector1[0] = 1.0;
        vector1[2047] = 3.0;
        let vector1_sized = FFT::gibbs_sizing(&vector1);
        assert!(vector1_sized.len() == 2187);
        assert!(vector1_sized[2] == 1.0);
        assert!(vector1_sized[2185] == 3.0);
    }

    #[test]
    fn test_static_and_trim() {
        // This vector should lead to 11 frequencies
        let vector1 = vec![1.0; 1024];
        let frame_size = vector1.len();
        let mut min = vector1[0];
        let mut max = vector1[0];
        for e in vector1.iter() {
            if e > &max {
                max = *e
            };
            if e < &min {
                min = *e
            };
        }
        // Initialize the compressor
        let mut c = FFT::new(frame_size, min, max);
        // Convert the data
        c.compress(&vector1);
        let frequencies_total = c.frequencies.len();
        let compressed_data = c.to_bytes();
        let out = FFT::decompress(&compressed_data).to_data(frame_size);
        assert_eq!(vector1, out);
        assert_eq!(frequencies_total, 0);
    }
}
