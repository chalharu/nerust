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

    pub(crate) fn as_ptr(&self) -> *const f32 {
        &self._data as *const _ as *const f32
    }

    pub(crate) fn as_array(&self) -> [f32; 16] {
        // Row-major copy for the matrix-change cache.
        let mut out = [0.0f32; 16];
        for (i, row) in self._data.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(row);
        }
        out
    }
}
