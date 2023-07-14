use std::env::args;

use hound::{WavSpec, WavWriter};
/*
Reads a WAV file, checks the channels and the information contained there. From that
information takes a decision on the best channel, block size and bitrate for the BRRO
encoders.
*/

/* Read a WAV file,  */
fn read_metrics_from_wav(filename: &str) -> Vec<f64> {
    let mut reader = hound::WavReader::open(filename).unwrap();
    let num_channels = reader.spec().channels as usize;

    let mut raw_data: Vec<f64> = Vec::new();
    let mut u64_holder: [u16; 4] = [0,0,0,0]; 
    
    // Iterate over the samples and channels and push each sample to the vector
    let mut current_channel: usize = 0;
    for sample in reader.samples::<i16>() {
        u64_holder[current_channel] = sample.unwrap() as u16;
        current_channel += 1;
        if current_channel == num_channels {
            raw_data.push(join_u16_into_f64(u64_holder));
            current_channel = 0;
        }
    }
    return raw_data;
}

fn generate_wav_header(channels: Option<i32>, bitdepth: u16) -> WavSpec {
    let spec = hound::WavSpec {
        channels: channels.unwrap_or(4) as u16,
        sample_rate: 8000,
        bits_per_sample: bitdepth,
        sample_format: hound::SampleFormat::Int
    };
    return spec;
}

/// Write a WAV file with the outputs of data analysis
fn write_optimal_wav(filename: &str, data: Vec<f64>, bitdepth: i32, channels: i32) {
    let header = generate_wav_header(Some(channels), bitdepth as u16);
    let file_path = format!("opt_{}", filename);
    let file = std::fs::OpenOptions::new().write(true).create(true).read(true).open(&file_path).unwrap();
    let mut wav_writer = WavWriter::new(file, header).unwrap();
    for sample in data {
        let _ = match bitdepth {
            8 =>  wav_writer.write_sample(as_i8(sample)),
            16 => wav_writer.write_sample(as_i16(sample)),
            _ => wav_writer.write_sample(as_i32(sample))
        };
    }
}

fn as_i8(value: f64) -> i8 {
    return split_n(value).0 as i8;
}

fn as_i16(value: f64) -> i16 {
    return split_n(value).0 as i16;
}

fn as_i32(value: f64) -> i32 {
    return split_n(value).0 as i32;
}

// Split a float into an integer
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
    } else {
        if shl < 64 { // x << 1..64
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
}

fn join_u16_into_f64(bits: [u16; 4]) -> f64 {
    let u64_bits = (bits[0] as u64) |
                ((bits[1] as u64) << 16) |
                ((bits[2] as u64) << 32) |
                ((bits[3] as u64) << 48);
    
    let f64_value = f64::from_bits(u64_bits);
    f64_value
}

fn get_max(a: i32, b: i32) -> i32 {
    if a >= b { a } else { b }
}

/// Go through the data, check min and max values, spectral analysis (later)
/// Check if data fits in 8,16,24,32 bits. If so reduce it to a single channel with
/// those bit depths.
fn analyze_data(data: &Vec<f64>) -> (i32, bool) {
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
    let max_int = split_n(max).0;
    let min_int = split_n(min).0;

    // If fractional is it relevant?
    let max_frac = split_n(max).1;
    let min_frac = split_n(min).1;

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

    let recommended_bitdepth = get_max(bitdepth, bitdepth_signed);
    if !fractional {
        print!(" Recommended Bitdepth: {} ", recommended_bitdepth);
    } else {
        print!(" Fractional, Recommended Bitdepth: {}, Fractions max: {} min: {}", recommended_bitdepth, max_frac, min_frac);
    }
    (recommended_bitdepth, fractional)
}

fn main() {

    let arguments: Vec<String> = args().collect();
    // Read file from arg
    print!("\nFile: {},", arguments[1]);
    let wav_data = read_metrics_from_wav(&arguments[1]);
    let (bitdepth, fractional) = analyze_data(&wav_data);
    if bitdepth == 64 || fractional { 
        //println!("No optimization, exiting");
        std::process::exit(0); 
    }
    if arguments.len() > 2 {
        print!("\nWriting optimal file!");
        write_optimal_wav(&arguments[1], wav_data, bitdepth, 1);
    }
}