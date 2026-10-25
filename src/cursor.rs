//! Decodes the small pointer shapes returned by DXGI Desktop Duplication.

pub const SHAPE_COLOR: u32 = 2;
pub const SHAPE_MASKED_COLOR: u32 = 4;
pub const SHAPE_MONOCHROME: u32 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecodedShape {
    pub width: u32,
    pub height: u32,
    /// 0 = straight-alpha color, 1 = monochrome AND/XOR, 2 = masked color.
    pub mode: u32,
}

pub fn decode_shape(
    shape_type: u32,
    width: u32,
    packed_height: u32,
    pitch: u32,
    bytes: &[u8],
) -> Option<(DecodedShape, Vec<u8>)> {
    let height = if shape_type == SHAPE_MONOCHROME {
        if !packed_height.is_multiple_of(2) {
            return None;
        }
        packed_height / 2
    } else {
        packed_height
    };
    if width == 0 || height == 0 || width > 1024 || height > 1024 {
        return None;
    }
    let mut rgba = vec![0; width as usize * height as usize * 4];
    match shape_type {
        SHAPE_COLOR | SHAPE_MASKED_COLOR => {
            let row_bytes = width.checked_mul(4)?;
            if pitch < row_bytes || bytes.len() < pitch as usize * height as usize {
                return None;
            }
            for y in 0..height as usize {
                for x in 0..width as usize {
                    let source = y * pitch as usize + x * 4;
                    let target = (y * width as usize + x) * 4;
                    // DXGI supplies 32-bit BGRA pixels; our shader texture is RGBA.
                    rgba[target] = bytes[source + 2];
                    rgba[target + 1] = bytes[source + 1];
                    rgba[target + 2] = bytes[source];
                    rgba[target + 3] = bytes[source + 3];
                }
            }
            let mode = if shape_type == SHAPE_COLOR { 0 } else { 2 };
            Some((
                DecodedShape {
                    width,
                    height,
                    mode,
                },
                rgba,
            ))
        }
        SHAPE_MONOCHROME => {
            let row_bytes = width.checked_add(7)? / 8;
            if pitch < row_bytes || bytes.len() < pitch as usize * packed_height as usize {
                return None;
            }
            for y in 0..height as usize {
                for x in 0..width as usize {
                    let mask = 0x80 >> (x & 7);
                    let and_bit = bytes[y * pitch as usize + x / 8] & mask != 0;
                    let xor_bit = bytes[(y + height as usize) * pitch as usize + x / 8] & mask != 0;
                    let target = (y * width as usize + x) * 4;
                    let and_byte = if and_bit { 255 } else { 0 };
                    rgba[target..target + 3].fill(and_byte);
                    rgba[target + 3] = if xor_bit { 255 } else { 0 };
                }
            }
            Some((
                DecodedShape {
                    width,
                    height,
                    mode: 1,
                },
                rgba,
            ))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_color_bgra_and_respects_row_pitch() {
        let bytes = [3, 2, 1, 4, 9, 9, 9, 9, 7, 6, 5, 8, 9, 9, 9, 9];
        let (shape, pixels) = decode_shape(SHAPE_COLOR, 1, 2, 8, &bytes).unwrap();
        assert_eq!((shape.width, shape.height, shape.mode), (1, 2, 0));
        assert_eq!(pixels, [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn decodes_monochrome_and_xor_planes_msb_first() {
        let bytes = [0b1010_0000, 0b0110_0000];
        let (shape, pixels) = decode_shape(SHAPE_MONOCHROME, 3, 2, 1, &bytes).unwrap();
        assert_eq!((shape.width, shape.height, shape.mode), (3, 1, 1));
        assert_eq!(pixels, [255, 255, 255, 0, 0, 0, 0, 255, 255, 255, 255, 255]);
    }

    #[test]
    fn rejects_truncated_rows_and_unknown_shapes() {
        assert!(decode_shape(SHAPE_COLOR, 2, 1, 8, &[0; 7]).is_none());
        assert!(decode_shape(99, 1, 1, 4, &[0; 4]).is_none());
    }
}
