use std::fs::{File, self};
use std::time::{SystemTime, Duration};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::codecs::{DecoderOptions, Decoder};
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo, FormatReader};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::{Hint, ProbeResult};
use symphonia::core::units::{Time, TimeBase};
use symphonia::core::io::MediaSourceStream;

use chrono::{DateTime, Utc, Timelike};

use crate::lib_vsri::{VSRI, self};

// --- Flac Reader
// Remote Reader Spec: ?

/* --- File Structure STRUCTURE
note: t=point in time, chan = channel, samples are the bytes for each channel.
      in this example, each sample is made of 2 bytes (16bit)
+---------------------------+---------------------------+-----
|         Frame 1           |         Frame 2           |  etc
+-------------+-------------+-------------+-------------+-----
| chan 1 @ t1 | chan 2 @ t1 | chan 1 @ t2 | chan 2 @ t2 |  etc
+------+------+------+------+------+------+------+------+-----
| byte | byte | byte | byte | byte | byte | byte | byte |  etc
+------+------+------+------+------+------+------+------+-----
 */ 

/// Structure that holds the samples for a metric that is in a FLaC file
/// It would only hold the number of samples needed for the provided interval
/// if the end of the interval is bigger than what is contained in the file,
///  the whole file content is returned.

 pub struct FlacMetric {
    timeseries_data: Vec<(i64, f64)>, // Sample Data
    file: File,                       // The File where the metric is
    interval_start: i64,              // The start interval in timestamp with miliseconds
    decoder: Option<Box<dyn Decoder>>, // Flac decoder
    format_reader: Option<Box<dyn FormatReader>> // Flac format reader
}

impl FlacMetric {
    pub fn new(file: File, start_ts: i64) -> Self {
        FlacMetric {
                    timeseries_data: Vec::new(),
                    file,
                    interval_start: start_ts,
                    decoder: None,
                    format_reader: None
                 }
    }

    fn datetime_from_ms(real_time: i64) -> String {
        // Time is in ms, convert it to seconds
        let datetime = DateTime::<Utc>::from_utc(
            chrono::NaiveDateTime::from_timestamp_opt(real_time/ 1000, 0).unwrap(),
            Utc,
        );
        // Transform datetime to string with the format YYYY-MM-DD
        let datetime_str = datetime.format("%Y-%m-%d").to_string();
        return datetime_str;
    }

    /// Load sample data into the Flac Object
    fn load_samples(self) -> Vec<(i64, f64)> {
        Vec::new()
    }

    fn get_format_reader(&self) -> Box<dyn FormatReader> {
        let file = &self.file;
        let file = Box::new(file);
        // Create the media source stream using the boxed media source from above.
        let mss = MediaSourceStream::new(*file, Default::default());
        let mut hint_holder = Hint::new();
        let hint = hint_holder.mime_type("FLaC");
        // Use the default options when reading and decoding.
        let format_opts: FormatOptions = Default::default();
        let metadata_opts: MetadataOptions = Default::default();
        // Probe the media source stream for a format.
        let probed = symphonia::default::get_probe().format(&hint, mss, &format_opts, &metadata_opts).unwrap();
        // Get the format reader yielded by the probe operation.
        return probed.format;
    }

    fn get_decoder(&self) ->  Box<dyn Decoder> {
        let decoder_opts: DecoderOptions = Default::default();
        let format = self.get_format_reader();
        // Get the default track.
        let track = format.default_track().unwrap();
        // Create a decoder for the track.
        let decoder = symphonia::default::get_codecs().make(&track.codec_params, &decoder_opts).unwrap();
        return decoder;
    }


    /// Read samples from a file with an optional start and end point.
    pub fn get_samples(&self, start: Option<i32>, end: Option<i32>) -> std::result::Result<Vec<f64>, SymphoniaError> {
        let mut sample_vec: Vec<f64> = Vec::new();
        let mut format_reader = self.get_format_reader();
        let mut decoder = self.get_decoder();
        let channels = decoder.codec_params().channels.unwrap().count();
        let mut sample_buf = None;
        let mut frame_counter: i32 = 0;
        let start_frame = start.unwrap_or(0);
        let end_frame = end.unwrap_or(lib_vsri::MAX_INDEX_SAMPLES);
        // Loop over all the packets, get all the samples and return them
        loop {
            let packet = match format_reader.next_packet() {
                Ok(packet) => packet,
                Err(err) => break println!("[DEBUG][READ]Reader error: {}", err),
            };
            // How many frames inside the packet
            let dur = packet.dur() as i32;
            // Check if we need to decode this packet or not
            if !(start_frame < frame_counter+dur && end_frame > frame_counter+dur) { 
                continue; 
            }
            // Decode the packet into samples.
            // TODO: This is overly complex, split into its own code
            match decoder.decode(&packet) {
                Ok(decoded) => {
                    // Consume the decoded samples (see below).
                    if sample_buf.is_none() {
                        // Get the audio buffer specification.
                        let spec = *decoded.spec();
                        // Get the capacity of the decoded buffer. Note: This is capacity, not length!
                        let duration = decoded.capacity() as u64;
                        // Create the sample buffer.
                        sample_buf = Some(SampleBuffer::<i16>::new(duration, spec));
                    }
                    // Each frame contains several samples, we need to get the frame not the sample. Since samples = frames * channels
                    if let Some(buf) = &mut sample_buf {
                        buf.copy_interleaved_ref(decoded);
                        let mut i16_samples: [u16; 4] = [0,0,0,0];
                        let mut i = 1; // Starting at 1, channel number is not 0 indexed...
                        for  sample in buf.samples() {
                            if i >= channels {
                                frame_counter += 1;
                                if frame_counter >= start_frame && frame_counter <= end_frame {
                                    sample_vec.push(FlacMetric::join_u16_into_f64(i16_samples));
                                }
                                i = 1;
                            }
                            i16_samples[i-1] = *sample as u16;
                            i += 1;
                        }
                    }
                },
                Err(SymphoniaError::DecodeError(err)) => println!("[DEBUG][READ]Decode error: {}", err),
                Err(err) => break println!("[DEBUG][READ]Unexpeted Decode error: {}", err),
            }
        };
        Ok(sample_vec)
    }

    /// Read all samples from a file
    pub fn get_all_samples(&self) -> std::result::Result<Vec<f64>, SymphoniaError> {
        let mut sample_vec: Vec<f64> = Vec::new();
        let mut format_reader = self.get_format_reader();
        let mut decoder = self.get_decoder();
        let channels = decoder.codec_params().channels.unwrap().count();
        let mut sample_buf = None;
        // Loop over all the packets, get all the samples and return them
        loop {
            let packet = match format_reader.next_packet() {
                Ok(packet) => packet,
                Err(err) => break println!("[DEBUG][READ]Reader error: {}", err),
            };
            // Decode the packet into audio samples.
            match decoder.decode(&packet) {
                Ok(decoded) => {
                    // Consume the decoded audio samples (see below).
                    if sample_buf.is_none() {
                        // Get the audio buffer specification.
                        let spec = *decoded.spec();
                        // Get the capacity of the decoded buffer. Note: This is capacity, not length!
                        let duration = decoded.capacity() as u64;
                        // Create the sample buffer.
                        sample_buf = Some(SampleBuffer::<i16>::new(duration, spec));
                    }
                    if let Some(buf) = &mut sample_buf {
                        buf.copy_interleaved_ref(decoded);
                        let mut i16_samples: [u16; 4] = [0,0,0,0];
                        let mut i = 1; // Starting at 1, channel number is not 0 indexed...
                        for  sample in buf.samples() {
                            if i >= channels {
                                sample_vec.push(FlacMetric::join_u16_into_f64(i16_samples));
                                i = 1;
                            }
                            i16_samples[i-1] = *sample as u16;
                            i += 1;
                        }
                    }
                },
                Err(SymphoniaError::DecodeError(err)) => println!("[DEBUG][READ]Decode error: {}", err),
                Err(err) => break println!("[DEBUG][READ]Unexpeted Decode error: {}", err),
            }
        };
        // Just to make it compile
        Ok(sample_vec)
    }

    /// Recreate a f64
    fn join_u16_into_f64(bits: [u16; 4]) -> f64 {
    let u64_bits = (bits[0] as u64) |
                ((bits[1] as u64) << 16) |
                ((bits[2] as u64) << 32) |
                ((bits[3] as u64) << 48);
    
    let f64_value = f64::from_bits(u64_bits);
    
    f64_value
    }
}