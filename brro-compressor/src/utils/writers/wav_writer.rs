use std::io::{self};
use std::path::{Path};
use hound::{WavSpec, WavWriter};
use log::info;

// Function to create a streaming writer for a file
pub fn initialize_directory(base_dir: &Path) -> io::Result<()> {
    if !base_dir.exists() {
        std::fs::create_dir_all(base_dir)?;
    }
    Ok(())
}
pub fn write_optimal_wav(filename: &str, data: Vec<f64>, channels: i32) {
    let (bitdepth, dc, _fractional) = analyze_data(&data);
    // Make DC a float for operations
    let fdc = dc as f64;
    let header: WavSpec = generate_wav_header(Some(channels), bitdepth as u16, 8000);
    let mut file_path = filename.to_string();
    file_path.truncate(file_path.len() - 4);
    file_path = format!("{}.wav", file_path);
    let file = std::fs::OpenOptions::new().write(true).create(true).read(true).open(file_path).unwrap();
    let mut wav_writer = WavWriter::new(file, header).unwrap();
    for sample in data {
        let _ = match bitdepth {
            8 =>  wav_writer.write_sample(as_i8(sample-fdc)),
            16 => wav_writer.write_sample(as_i16(sample-fdc)),
            _ => wav_writer.write_sample(as_i32(sample-fdc))
        };
    }
    let _ = wav_writer.finalize();
}
fn as_i8(value: f64) -> i8 {
    split_n(value).0 as i8
}

fn as_i16(value: f64) -> i16 {
    split_n(value).0 as i16
}

fn as_i32(value: f64) -> i32 {
    split_n(value).0 as i32
}

fn split_n(x: f64) -> (i64, f64) {
    const FRACT_SCALE: f64 = 1.0 / (65536.0 * 65536.0 * 65536.0 * 65536.0); // 1_f64.exp(-64)
    const STORED_MANTISSA_DIGITS: u32 = f64::MANTISSA_DIGITS - 1;
    const STORED_MANTISSA_MASK: u64 = (1 << STORED_MANTISSA_DIGITS) - 1;
    const MANTISSA_MSB: u64 = 1 << STORED_MANTISSA_DIGITS;

    const EXPONENT_BITS: u32 = 64 - 1 - STORED_MANTISSA_DIGITS;
    const EXPONENT_MASK: u32 = (1 << EXPONENT_BITS) - 1;
    const EXPONENT_BIAS: i32 = (1 << (EXPONENT_BITS - 1)) - 1;

    let bits = x.to_bits();
    let is_negative = (bits as i64) < 0;
    let exponent = ((bits >> STORED_MANTISSA_DIGITS) as u32 & EXPONENT_MASK) as i32;

    let mantissa = (bits & STORED_MANTISSA_MASK) | MANTISSA_MSB;
    let mantissa = if is_negative { -(mantissa as i64) } else { mantissa as i64 };

    let shl = exponent + (64 - f64::MANTISSA_DIGITS as i32 - EXPONENT_BIAS + 1);
    if shl <= 0 {
        let shr = -shl;
        if shr < 64 { // x >> 0..64
            let fraction = ((mantissa as u64) >> shr) as f64 * FRACT_SCALE;
            (0, fraction)
        } else { // x >> 64..
            (0, 0.0)
        }
    }
    else if shl < 64 { // x << 1..64
        let int = mantissa >> (64 - shl);
        let fraction = ((mantissa as u64) << shl) as f64 * FRACT_SCALE;
        (int, fraction)
    } else if shl < 128 { // x << 64..128
        let int = mantissa << (shl - 64);
        (int, 0.0)
    } else { // x << 128..
        (0, 0.0)
    }
}

fn join_u16_into_f64(bits: [u16; 4]) -> f64 {
    let u64_bits = (bits[0] as u64) |
        ((bits[1] as u64) << 16) |
        ((bits[2] as u64) << 32) |
        ((bits[3] as u64) << 48);


    f64::from_bits(u64_bits)
}
fn generate_wav_header(channels: Option<i32>, bitdepth: u16, samplerate: u32) -> WavSpec {

    hound::WavSpec {
        channels: channels.unwrap_or(4) as u16,
        // TODO: Sample rate adaptations
        sample_rate: samplerate,
        bits_per_sample: bitdepth,
        sample_format: hound::SampleFormat::Int
    }
}
fn analyze_data(data: &Vec<f64>) -> (i32, i64, bool) {
    let mut min: f64 = 0.0;
    let mut max: f64 = 0.0;
    let mut fractional = false;
    for value in data {
        let t_value = *value;
        if split_n(t_value).1 != 0.0 { fractional = true;}
        if t_value > max { max = t_value};
        if t_value < min { min = t_value};
    }
    // Check max size of values
    // For very large numbers (i32 and i64), it might be ideal to detect the dc component
    // of the signal. And then remove it later
    let max_int = split_n(max).0; // This is the DC component
    let min_int = split_n(min).0;

    // If fractional is it relevant?
    let max_frac = split_n(max).1;

    // Finding the bitdepth without the DC component
    let recommended_bitdepth = find_bitdepth(max_int-min_int, min_int);
    if !fractional {
        info!(" Recommended Bitdepth: {} ", recommended_bitdepth);
    } else {
        info!(" Fractional, Recommended Bitdepth: {}, Fractions max: {}", recommended_bitdepth, max_frac);
    }
    (recommended_bitdepth, min_int, fractional)
}
fn find_bitdepth(max_int: i64, min_int: i64) -> i32 {
    // Check where those ints fall into
    let bitdepth = match max_int {
        _ if max_int <= u8::MAX.into() => 8,
        _ if max_int <= i16::MAX.into() => 16,
        _ if max_int <= i32::MAX.into() => 32,
        _ => 64
    };

    let bitdepth_signed = match min_int {
        _ if min_int == 0 => 8,
        _ if min_int >= i16::MIN.into() => 16,
        _ if min_int >= i32::MIN.into() => 32,
        _ => 64
    };


    get_max(bitdepth, bitdepth_signed)
}
fn get_max(a: i32, b: i32) -> i32 {
    a.max(b)
}