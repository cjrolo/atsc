use std::cmp;

/// Calculates the mean squared error between two vectors.
///
/// # Arguments
///
/// * `vec1` - The first vector of f64 values.
/// * `vec2` - The second vector of f64 values.
///
/// # Returns
///
/// The mean squared error, or an error message if the vector lengths are different.
pub fn calculate_error(vec1: &[f64], vec2: &Vec<f64>) -> Option<f64> {
    if vec1.len() != vec2.len() {
        return None;
    }

    let min_length = cmp::min(vec1.len(), vec2.len());
    let squared_error: f64 = (0..min_length)
        .map(|i| (vec1[i] - vec2[i]).powi(2))
        .sum();
    Some(squared_error / min_length as f64)
}

/// Computes the Normalized Mean Square Error between 2 signals
pub fn nmsqe(original: &[f64], generated: &[f64]) -> Option<f64> {
    if original.len() != generated.len() {
        return None;
    }

    let squared_error: f64 = original
        .iter()
        .zip(generated.iter())
        .map(|(original, generated)| (generated - original).powi(2))
        .sum();
    let original_square_sum: f64 = original.iter().map(|original| original.powi(2)).sum();
    Some(squared_error / original_square_sum)
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_error() {
        let vector1 = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let vector2 = vec![2.5, 4.0, 6.0, 8.0, 10.0];
        let vector3 = vec![1.5, 2.5, 2.8, 3.7];

        assert_eq!(calculate_error(&vector1, &vector1), Some(0.0));
        assert_eq!(calculate_error(&vector1, &vector2), Some(11.25));
        assert_eq!(calculate_error(&vector1, &vector3), None);
    }

    #[test]
    fn test_calculate_nmsqe() {
        let vector1 = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let vector2 = vec![2.5, 4.0, 6.0, 8.0, 10.0];
        let vector3 = vec![1.5, 2.5, 2.8, 3.7];

        assert_eq!(nmsqe(&vector1, &vector1), Some(0.0));
        assert_eq!(nmsqe(&vector1, &vector2), Some(1.0227272727272727));
        assert_eq!(nmsqe(&vector1, &vector3), None);
    }
}
