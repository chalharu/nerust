//! Column-major 4x4 matrices for `glUniformMatrix4fv` with
//! `transpose = FALSE` (required: GLES forbids `TRUE`).
//!
//! `_data` stores columns: `_data[col][row]`, so `as_ptr` points at
//! the exact memory layout GL reads. Symmetric matrices (identity,
//! axis scale) look identical either way, which is why the
//! row-vs-column bug hides until the first non-symmetric matrix
//! (translation, rotation); the test below locks a translation.

#[derive(Debug, Copy, Clone)]
pub(crate) struct Mat4 {
    _data: [[f32; 4]; 4],
}

impl Mat4 {
    pub(crate) fn identity() -> Self {
        Self {
            _data: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// Test-only symmetric reference (production fit goes through
    /// `layout::fit_viewport`, which returns the matrix directly).
    #[cfg(test)]
    pub(crate) fn scale(x: f32, y: f32, z: f32) -> Self {
        Self {
            _data: [
                [x, 0.0, 0.0, 0.0],
                [0.0, y, 0.0, 0.0],
                [0.0, 0.0, z, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// Non-symmetric translation: translation lives in the fourth
    /// *column* (`_data[3][0..3]`). Column-major memory is
    /// `[1,0,0,0, 0,1,0,0, 0,0,1,0, tx,ty,tz,1]`.
    /// Test-only until a camera/translate path needs it in production.
    #[cfg(test)]
    pub(crate) fn translate(tx: f32, ty: f32, tz: f32) -> Self {
        Self {
            _data: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [tx, ty, tz, 1.0],
            ],
        }
    }

    pub(crate) fn as_ptr(&self) -> *const f32 {
        &self._data as *const _ as *const f32
    }

    pub(crate) fn as_array(&self) -> [f32; 16] {
        // Column-major copy for the matrix-change cache.
        let mut out = [0.0f32; 16];
        for (i, col) in self._data.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(col);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::Mat4;

    #[test]
    fn translation_memory_is_column_major() {
        // The bug this locks: storing rows but uploading with
        // `transpose = FALSE` transposes translation into the last
        // *row*, shifting `w` instead of `xyz`. Symmetric matrices
        // (identity, scale) pass either way, so only a non-symmetric
        // matrix proves the layout.
        let m = Mat4::translate(2.0, 3.0, 4.0);
        let words = unsafe { std::slice::from_raw_parts(m.as_ptr(), 16) };
        assert_eq!(
            words,
            &[
                1.0, 0.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, 0.0, //
                0.0, 0.0, 1.0, 0.0, //
                2.0, 3.0, 4.0, 1.0,
            ]
        );
        assert_eq!(m.as_array(), words);
    }

    #[test]
    fn scale_stays_symmetric() {
        let m = Mat4::scale(0.5, 1.0, 1.0);
        assert_eq!(
            m.as_array(),
            [
                0.5, 0.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, 0.0, //
                0.0, 0.0, 1.0, 0.0, //
                0.0, 0.0, 0.0, 1.0,
            ]
        );
    }
}
