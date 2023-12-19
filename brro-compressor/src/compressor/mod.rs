use bincode::config::{self, Configuration};
use bincode::{Decode, Encode};

use self::constant::{constant, constant_to_data};
use self::fft::{fft, fft_to_data, fft_allowed_error};
use self::noop::{noop, noop_to_data};
use self::polynomial::{polynomial, to_data, PolynomialType, polynomial_allowed_error};

pub mod noop;
pub mod constant;
pub mod fft;
pub mod polynomial;

#[derive(Encode, Decode, Default, Debug, Clone)]
pub enum Compressor {
    #[default]
    Noop,
    FFT,
    Idw,
    Constant,
    Polynomial,
    Auto
}

/// Struct to store the results of a compression round. Can be used later to pick the best one
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
pub struct CompressorResult {
    pub compression_size: u64,
    pub error: f64
}

impl CompressorResult {
    pub fn new(compression_size: u64, error: f64) -> Self {
        CompressorResult {compression_size, error}
    }
}

impl Compressor {
    pub fn compress(&self, data: &[f64] ) -> Vec<u8> {
        match self {
            Compressor::Noop => noop(data),
            Compressor::FFT => fft(data),
            Compressor::Constant => constant(data),
            Compressor::Polynomial => polynomial(data, PolynomialType::Polynomial),
            Compressor::Idw => polynomial(data, PolynomialType::Idw),
            _ => todo!(),
        }
    }

    pub fn compress_bounded(&self, data: &[f64], max_error: f64 ) -> Vec<u8> {
        match self {
            Compressor::Noop => noop(data),
            Compressor::FFT => fft_allowed_error(data, max_error),
            Compressor::Constant => constant(data),
            Compressor::Polynomial => polynomial_allowed_error(data, max_error, PolynomialType::Polynomial),
            Compressor::Idw => polynomial_allowed_error(data, max_error, PolynomialType::Idw),
            _ => todo!(),
        }
    }

    pub fn decompress(&self, samples: usize, data: &[u8] ) -> Vec<f64> {
        match self {
            Compressor::Noop => noop_to_data(samples, data),
            Compressor::FFT => fft_to_data(samples, data),
            Compressor::Constant => constant_to_data(samples, data),
            Compressor::Polynomial => to_data(samples, data),
            Compressor::Idw => to_data(samples, data),
            _ => todo!()
        }
    }
}

pub struct BinConfig {
    config: Configuration,
 }

impl BinConfig {
    pub fn get() -> Configuration {
        // Little endian and Variable int encoding
        config::standard()
    }
}