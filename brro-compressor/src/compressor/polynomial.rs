use crate::utils::{DECIMAL_PRECISION, error::calculate_error, round_and_limit_f64, round_f64};

use super::BinConfig;
use bincode::{Decode, Encode};
use inverse_distance_weight::IDW;
use log::{debug, info, trace};
use splines::{Interpolation, Key, Spline};

const POLYNOMIAL_COMPRESSOR_ID: u8 = 0;
const IDW_COMPRESSOR_ID: u8 = 1;

#[derive(Encode, Decode, Default, Debug, Clone, PartialEq)]
pub enum PolynomialType {
    #[default]
    Polynomial = 0,
    Idw = 1
}

#[derive(Encode, Decode, Default, Debug, Clone)]
pub enum Method {
    #[default]
    CatmullRom,
    Idw,
}

#[derive(Encode, Decode, PartialEq, Debug, Clone)]
pub struct Polynomial {
    /// Compressor ID
    pub id: PolynomialType,
    /// Stored Points
    pub data_points: Vec<f64>,
    /// The maximum numeric value of the points in the frame
    pub max_value: f32,
    pub max_position: usize,
    /// The minimum numeric value of the points in the frame
    pub min_value: f32,  
    pub min_position: usize,
    /// What is the base step between points
    pub point_step: u8,
}

impl Polynomial {
    pub fn new(sample_count: usize, min: f64, max: f64, ptype: PolynomialType) -> Self {
        debug!("Polynomial compressor: min:{} max:{}, Type: {:?}", min, max, ptype);
        Polynomial {
            id: ptype,
            data_points: Vec::with_capacity(sample_count),
            /// The maximum numeric value of the points in the frame
            max_value: max as f32,  
            /// The minimum numeric value of the points in the frame
            min_value: min as f32,
            min_position: 0,
            max_position: 0,
            // Minimum step is always 1
            point_step: 1,
            }
    }

    pub fn set_pos(&mut self, pmin: usize, pmax: usize){
        self.min_position = pmin;
        self.max_position = pmax;
    }

    fn locate_in_data_points(&self, point: f64) -> bool {
        self.data_points.iter().any(|&i| i==point)
    }

    fn get_method(&self) -> Method {
        match self.id {
            PolynomialType::Idw => Method::Idw,
            PolynomialType::Polynomial => Method::CatmullRom,
        }
    }

    pub fn compress_bounded(&mut self, data: &[f64], max_err: f64) {
        if self.max_value == self.min_value { 
            debug!("Same max and min, we're done here!");
            return
        }
        // TODO: Big one, read below
        // To reduce error we add more points to the polynomial, but, we also might add residuals
        // each residual is 1/data_lenght * 100% less compression, each jump is 5% less compression. 
        // We can do the math and pick the one which fits better. 
        let method = self.get_method();
        let data_len = data.len();
        let baseline_points = if 3 >= (data_len/100) { 3 } else { data_len/100 };
        // Variables for the error control loop
        let mut current_err = max_err + 1.0;
        let mut jump: usize = 0;
        let mut iterations = 0;
        // Locking max target error precision to 0.1%
        let target_error = round_f64(max_err, 3);
        while target_error < round_f64(current_err, 4) {
            trace!("Method: {:?} Iterations: {} Error: {} Target: {}", method, iterations, current_err, target_error);
            iterations += 1;
            self.compress_hinted(data, baseline_points+jump);
            let out_data = match method {
                Method::CatmullRom => self.polynomial_to_data(data_len),
                Method::Idw => self.idw_to_data(data_len)
            };
            trace!("Calculated Values: {:?}", out_data);
            trace!("Data Values: {:?}", data);
            current_err = calculate_error(data, &out_data);
            trace!("Current Err: {}", current_err);
            // Max iterations is 18 (We start at 10%, we can go to 95% and 1% at a time)
            match iterations {
                // We should always increase by 1 in worst case
                1..=17 => jump += (data_len/10).max(1),
                18..=22 => jump += (data_len/100).max(1),
                // No more jumping, but we landed right in the end
                _ if target_error > round_f64(current_err, 4) => break,
                // We can't hit the target, store everything
                _ => {
                    self.compress_hinted(data, data_len);
                    break;
                }
                
            }
            if self.data_points.len() == data_len {
                // Storing the whole thing anyway...
                break;
            }
        }
        debug!("Final Stored Data Lenght: {} Iterations: {}", self.data_points.len(), iterations);
    } 

    pub fn compress_hinted(&mut self, data: &[f64], points: usize) {
        if self.max_value == self.min_value { 
            debug!("Same max and min, we're done here!");
            return
        }
        // The algorithm is simple, Select 10% of the data points, calculate the Polynomial based on those data points
        // Plus the max and min
        let data_len = data.len();
        // Instead of calculation, we use the provided count
        let point_count = points;
        // Step size
        let step = (data_len/point_count).max(1);
        // I can calculate the positions from here
        let mut points: Vec<f64> = (0..data_len).step_by(step).map(|f| f as f64).collect();
        // Pushing the last value if needed (and if data is not empty)
        if points.last() != Some(&(data_len as f64 -1.)) { points.push(data_len as f64 - 1.); }
        // I need to extract the values for those points
        let mut values: Vec<f64> = points.iter().map(|&f| data[f as usize]).collect();
        
        debug!("Compressed Hinted Points: {:?}", points);
        debug!("Compressed Hinted Values: {:?}", values);

        // I need to insert MIN and MAX only if they don't belong to the values already.
        let mut prev_pos = points[0];
        for (array_position, position_value) in points.iter().enumerate() {
            if self.min_position > (prev_pos.round() as usize) && self.min_position < (position_value.round() as usize) {
                // We have to insert here
                values.insert(array_position, self.min_value as f64);
            }
            if self.max_position > (prev_pos.round() as usize) && self.max_position < (position_value.round() as usize) {
                // We have to insert here
                values.insert(array_position, self.max_value as f64);
                // And we are done
            }
            prev_pos = *position_value;
        }

        self.data_points = values; 
        self.point_step = step as u8;
    }

    // --- MANDATORY METHODS ---
    pub fn compress(&mut self, data: &[f64]) {
        let points = if 3 >= (data.len()/100) { 3 } else { data.len()/100 };
        self.compress_hinted(data, points)
    }

    /// Decompresses data
    pub fn decompress(data: &[u8]) -> Self {
        let config = BinConfig::get();
        let (poly, _) = bincode::decode_from_slice(data, config).unwrap();
        poly
    }

    pub fn to_bytes(self) -> Vec<u8> {
        let config = BinConfig::get();
        bincode::encode_to_vec(self, config).unwrap()
    }

    // --- END OF MANDATORY METHODS ---
    /// Since IDW and Polynomial are the same code everywhere, this function prepares the data
    /// to be used by one of the polynomial decompression methods
    /*
    Trying to explain this:

    1. For calculation a polynomial, we need the data points (Y) and the location of them in the function (X). 
    2. To avoid storing two sets of points (X, Y) we only store one (Y), since the others we can infer (Or data always starts in `0`, and as an increment of `1`).
    3. When we decompress, we look into how much data points we stored (size of Y, values), we also know the frame size.
    4. From the frame size, we do the calculation how many points we should had stored.
    5. if (3) and (4) doesn't match, we know we stored extra points, that means `min_value` and/or `max_value` where stored too.
    6. Ok, then the infer we did in 2, might be wrong, we are missing 1 or 2 points in X.
    7. We call `get_positions` to build X.
    8. `get_positions` walks the X array, and checks if `min_position` and/or `max_position` (those are stored too) fit in between any interval, since we have regular intervals for X. If they fit, we push them there.

    Example:

    Let's say X is `[0, 5, 10, 15, 20]` and Y is `[3, 2, 3, 4, 5, 6, 3]`. `min_value = 2`, `max_value=6`. `min_position=2`, `max_position=17`.

    Decompression starts, Len(x) = 5, Len(Y) = 7. We are missing 2 points in X.
    Walk `X` and check every element if `min_position` is between current point and previous point, if so, insert it there. Continue, do it the same for `max_position`.
     */
    fn get_positions(&self, frame_size: usize) -> Vec<usize> {
        let mut points = Vec::with_capacity(frame_size);
        let mut prev_pos = 0;
        for position_value in (0..frame_size).step_by(self.point_step as usize) {
            // I always need to add the current position
            // if min == current || max == current, push(0), continue
            // if prev < min < current, push(min), push(current)
            // if prev < max < current, push(max), push(current)
            // push(current)
            if self.min_position == position_value || self.max_position == position_value {
                points.push(position_value);
                prev_pos = position_value;
                continue;
            }
            if self.min_position > prev_pos && self.min_position < position_value {
                // Inserting in the middle
                points.push(self.min_position);
            }
            if self.max_position > prev_pos && self.max_position < position_value {
                points.push(self.max_position);
            }
            points.push(position_value);
            prev_pos = position_value;
        }
        // If max position is behind the last step, add it
        if  points.last() < Some(&self.max_position) { points.push(self.max_position); }
        // Always add the last position of the frame, if needed
        if  points.last() != Some(&(frame_size - 1)) { points.push(frame_size-1); }
        trace!("min p {} max p {} step {} data points {}", self.min_position, self.max_position, self.point_step, self.data_points.len());
        trace!("points {:?}", points);
        points
    }

    pub fn polynomial_to_data(&self, frame_size: usize) -> Vec<f64> {
        if self.max_value == self.min_value { 
            debug!("Same max and min, faster decompression!");
            return vec![self.max_value as f64; frame_size];
         }
        // Create the interpolation
        let points = self.get_positions(frame_size);
        let mut key_vec = Vec::with_capacity(points.len());
        for (current_key, (point, value)) in points.iter().zip(self.data_points.iter()).enumerate() {
            // CatmullRom needs at least 1 key behind and 2 ahead so this check.
            let interpolation = 
                if current_key > 0 && points.len() - current_key > 2 { Interpolation::CatmullRom }
                else { Interpolation::Linear };
            key_vec.push(Key::new(*point as f64, *value, interpolation));
        }
        let spline = Spline::from_vec(key_vec);
        // Build the data
        // There is a problem with the spline calculation, that it might get a value for all positions. In those cases
        // we return the good value calculated. If that doesn't exist, we return the minimum value 
        let mut out_vec = Vec::with_capacity(frame_size);
        let mut prev = self.min_value as f64;
        for value in 0..frame_size {
            let spline_value = spline.clamped_sample(value as f64).unwrap_or(prev);
            prev = spline_value;
            out_vec.push(round_and_limit_f64(spline_value, self.min_value.into(), self.max_value.into(), DECIMAL_PRECISION));
        }
        out_vec
    }

    pub fn idw_to_data(&self, frame_size: usize) -> Vec<f64> {
        // IDW needs f64 for points :(
        let points = self.get_positions(frame_size).iter().map(|&f| f as f64).collect();
        let idw = IDW::new(points, self.data_points.clone());
        // Build the data
        (0..frame_size)
        .map(|f| round_and_limit_f64(idw.evaluate(f as f64), self.min_value.into(), self.max_value.into(), DECIMAL_PRECISION))
        .collect() 
    }

    pub fn to_data(&self, frame_size: usize) -> Vec<f64> {
        match self.id {
            PolynomialType::Idw => self.idw_to_data(frame_size),
            PolynomialType::Polynomial => self.polynomial_to_data(frame_size),
        }
    }

}

pub fn polynomial(data: &[f64], idw: PolynomialType) -> Vec<u8> {
    info!("Initializing Polynomial Compressor");
    let mut min = data[0];
    let mut max = data[0];
    let mut pmin = 0;
    let mut pmax = 0;
    // For these one we need to store where the min and max happens on the data, not only their values
    for (position, value) in data.iter().enumerate(){
        if value > &max { max = *value;  pmax = position;};
        if value < &min { min = *value;  pmin = position; };
    }
    // Initialize the compressor
    let mut c = Polynomial::new(data.len(), min, max, idw);
    c.set_pos(pmin, pmax);
    // Convert the data
    c.compress(data);
    // Convert to bytes
    c.to_bytes()
}

pub fn polynomial_allowed_error(data: &[f64], allowed_error: f64, idw: PolynomialType) -> Vec<u8> {
    info!("Initializing Polynomial Compressor");
    let mut min = data[0];
    let mut max = data[0];
    let mut pmin = 0;
    let mut pmax = 0;
    // For these one we need to store where the min and max happens on the data, not only their values
    for (position, value) in data.iter().enumerate(){
        if value > &max { max = *value;  pmax = position;};
        if value < &min { min = *value;  pmin = position; };
    }
    // Initialize the compressor
    let mut c = Polynomial::new(data.len(), min, max, idw);
    c.set_pos(pmin, pmax);
    // Convert the data
    c.compress_bounded(data, allowed_error);
    // Convert to bytes
    c.to_bytes()
}

/// Uncompress 
pub fn to_data(sample_number: usize, compressed_data: &[u8]) -> Vec<f64> {
    let c = Polynomial::decompress(compressed_data);
    c.to_data(sample_number)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_polynomial() {
        let vector1 = vec![1.0, 0.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 3.0, 1.0, 1.0, 5.0];
        assert_eq!(polynomial(&vector1, PolynomialType::Polynomial), [0, 5, 0, 0, 0, 0, 0, 0, 240, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 64, 0, 0, 0, 0, 0, 0, 8, 64, 0, 0, 0, 0, 0, 0, 20, 64, 0, 0, 160, 64, 11, 0, 0, 0, 0, 1, 4]);
    }

    #[test]
    fn test_polynomial_compression() {
        let vector1 = vec![1.0, 1.0, 1.0, 1.0, 2.0, 3.0, 5.0, 1.0, 2.0, 7.0, 1.0, 1.0, 1.0, 3.0, 1.0, 1.0, 5.0];
        let frame_size = vector1.len();
        let idw_data = polynomial(&vector1, PolynomialType::Polynomial);
        let out = Polynomial::decompress(&idw_data).to_data(frame_size);
        assert_eq!(out, [1.0, 1.4, 1.8, 2.2, 2.6, 3.0, 4.075, 5.53333, 6.725, 7.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 5.0]);
    }

    #[test]
    fn test_polynomial_linear_compression() {
        let vector1 = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0];
        let frame_size = vector1.len();
        let idw_data = polynomial(&vector1, PolynomialType::Polynomial);
        let out = Polynomial::decompress(&idw_data).to_data(frame_size);
        assert_eq!(out, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0]);
    }

    #[test]
    fn test_to_allowed_error() {
        let vector1 = vec![1.0, 1.0, 1.0, 1.0, 2.0, 3.0, 5.0, 1.0, 2.0, 7.0, 1.0, 1.0, 1.0, 3.0, 1.0, 1.0, 5.0];
        let frame_size = vector1.len();
        let compressed_data = polynomial_allowed_error(&vector1, 0.5, PolynomialType::Polynomial);
        let out = Polynomial::decompress(&compressed_data).to_data(frame_size);
        let e = calculate_error(&vector1, &out);
        assert!(e <= 0.5);
    }

    #[test]
    fn test_idw() {
        let vector1 = vec![1.0, 0.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 3.0, 1.0, 1.0, 5.0];
        assert_eq!(polynomial(&vector1, PolynomialType::Idw), [1, 5, 0, 0, 0, 0, 0, 0, 240, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 64, 0, 0, 0, 0, 0, 0, 8, 64, 0, 0, 0, 0, 0, 0, 20, 64, 0, 0, 160, 64, 11, 0, 0, 0, 0, 1, 4]);
    }

    #[test]
    fn test_idw_compression() {
        let vector1 = vec![1.0, 1.0, 1.0, 1.0, 2.0, 3.0, 5.0, 1.0, 2.0, 7.0, 1.0, 1.0, 1.0, 3.0, 1.0, 1.0, 5.0];
        let frame_size = vector1.len();
        let idw_data = polynomial(&vector1, PolynomialType::Idw);
        let out = Polynomial::decompress(&idw_data).to_data(frame_size);
        assert_eq!(out, [1.0, 1.21502, 1.89444, 2.63525, 2.97975, 3.0, 3.21181, 4.10753, 5.44851, 7.0, 1.0, 2.23551, 2.70348, 2.5293, 1.92317, 1.0, 5.0]);
    }

    #[test]
    fn test_idw_linear_compression() {
        let vector1 = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0];
        let frame_size = vector1.len();
        let idw_data = polynomial(&vector1, PolynomialType::Idw);
        let out = Polynomial::decompress(&idw_data).to_data(frame_size);
        assert_eq!(out, [1.0, 1.62873, 3.51429, 4.84995, 5.0, 5.40622, 7.05871, 8.64807, 9.0, 9.37719, 11.18119, 12.0]);
    }

    #[test]
    fn test_idw_to_allowed_error() {
        let vector1 = vec![1.0, 1.0, 1.0, 1.0, 2.0, 3.0, 5.0, 1.0, 2.0, 7.0, 1.0, 1.0, 1.0, 3.0, 1.0, 1.0, 5.0];
        let frame_size = vector1.len();
        let compressed_data = polynomial_allowed_error(&vector1, 0.02, PolynomialType::Idw);
        let out = Polynomial::decompress(&compressed_data).to_data(frame_size);
        let e = calculate_error(&vector1, &out);
        assert!(e <= 0.5);
    }

    #[test]
    fn test_line_polynomial() {
        let vector1 = vec![1.0, 1.0, 1.0, 1.0];
        assert_eq!(polynomial(&vector1, PolynomialType::Polynomial), [0, 0, 0, 0, 128, 63, 0, 0, 0, 128, 63, 0, 1]);
    }

    #[test]
    fn test_line_idw() {
        let vector1 = vec![1.0, 1.0, 1.0, 1.0];
        assert_eq!(polynomial(&vector1, PolynomialType::Idw), [1, 0, 0, 0, 128, 63, 0, 0, 0, 128, 63, 0, 1]);
    }

}