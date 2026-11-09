//! Calibration blobs for the workspace's tests, so that none of them needs a
//! real person's calibration: [`blob`] lays the format the tracker's blobs
//! have (see [`crate::blob`]) around made-up points and an opaque body of
//! zeros, as long as a tracker's, so that a test moves as many bytes as the
//! real thing. It is no calibration a tracker could use.
//!
//! Behind the `testing` feature, which only dev-dependencies turn on.

use crate::blob::{PointRecord, RECORD_LEN, STIMULUS_POINTS};

/// The length of each blob the tracker has handed out so far, which
/// [`device_like`] matches.
pub const DEVICE_BLOB_LEN: usize = 659_056;

/// Records the tracker's ring holds: two rounds of the 7-point pattern.
const RING: usize = 14;

/// Where the point list's lead-in sits in [`blob`]'s output: where a
/// tracker's blob of [`DEVICE_BLOB_LEN`] bytes has it.
const LIST: usize = DEVICE_BLOB_LEN - 8 - RING * RECORD_LEN;

/// A blob of calibration `id` holding `records`: the tracker's header, a
/// body of zeros up to the point list, then the records. [`DEVICE_BLOB_LEN`]
/// long with 14 records.
///
/// # Panics
///
/// With more records than a blob can hold (a test's mistake).
#[must_use]
pub fn blob(id: u32, records: &[PointRecord]) -> Vec<u8> {
    let word = |v: usize| u32::try_from(v).expect("a blob's offsets fit 32 bits");
    let mut b = vec![0u8; LIST + 8];
    let mut put = |at: usize, bytes: &[u8]| b[at..at + bytes.len()].copy_from_slice(bytes);
    put(0, &word(LIST).to_le_bytes());
    put(4, &110u32.to_le_bytes());
    put(8, &84u32.to_le_bytes());
    put(12, &1u32.to_le_bytes());
    put(20, &id.to_le_bytes());
    put(24, &word(LIST - 88).to_le_bytes());
    put(28, b"human\0\0\0");
    // The 4 opaque bytes at the list stay zero; then the count.
    put(LIST + 4, &word(records.len()).to_le_bytes());
    for r in records {
        for v in [r.target[0], r.target[1], r.a[0], r.a[1]] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&r.a_status.to_le_bytes());
        for v in r.b {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&r.b_status.to_le_bytes());
    }
    b
}

/// Two rounds of the 7-point pattern, in its order, each eye measured a
/// little off its target and every point used: what a session of the
/// default two rounds leaves.
#[must_use]
pub fn two_rounds() -> Vec<PointRecord> {
    (0..RING)
        .zip(0u8..)
        .map(|(i, k)| {
            let target = STIMULUS_POINTS[i % STIMULUS_POINTS.len()];
            let drift = f32::from(k) * 0.001;
            PointRecord {
                target,
                a: [target[0] + 0.01 + drift, target[1] - 0.01],
                a_status: 1,
                b: [target[0] - 0.01, target[1] + 0.01 + drift],
                b_status: 1,
            }
        })
        .collect()
}

/// [`blob`] of [`two_rounds`]: a calibration of `id` shaped as a tracker
/// hands one out.
#[must_use]
pub fn device_like(id: u32) -> Vec<u8> {
    blob(id, &two_rounds())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blob::{BlobInfo, points, validate};

    #[test]
    fn a_made_up_blob_passes_as_a_trackers_does() {
        let b = device_like(7);

        assert_eq!(
            validate(&b),
            Ok(BlobInfo {
                id: 7,
                points: 14,
                len: DEVICE_BLOB_LEN,
                conventional: true,
            })
        );
        assert_eq!(points(&b).expect("points"), two_rounds());
        assert_eq!(validate(&blob(9, &[])).map(|i| i.points), Ok(0));
    }
}
