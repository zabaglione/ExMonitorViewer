use crate::cursor::DecodedShape;
use std::mem::size_of;
use windows::core::{Error, Result};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorInfo, GetIconInfo, CURSORINFO, CURSOR_SHOWING, CURSOR_SUPPRESSED, HICON, ICONINFO,
};

const MAX_CURSOR_DIMENSION: i32 = 1024;

#[derive(Clone, Copy, Debug, Default)]
pub struct CursorState {
    pub visible: bool,
    pub handle: isize,
    pub screen_position: [i32; 2],
}

pub struct CursorBitmap {
    pub shape: DecodedShape,
    pub pixels: Vec<u8>,
    pub hotspot: [i32; 2],
}

struct OwnedIconBitmaps {
    mask: HBITMAP,
    color: HBITMAP,
}

impl Drop for OwnedIconBitmaps {
    fn drop(&mut self) {
        unsafe {
            if !self.mask.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(self.mask.0));
            }
            if !self.color.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(self.color.0));
            }
        }
    }
}

pub unsafe fn query_cursor_state() -> Result<CursorState> {
    let mut info = CURSORINFO {
        cbSize: size_of::<CURSORINFO>() as u32,
        ..Default::default()
    };
    GetCursorInfo(&mut info)?;
    Ok(CursorState {
        visible: info.flags.0 & CURSOR_SHOWING.0 != 0 && info.flags.0 & CURSOR_SUPPRESSED.0 == 0,
        handle: info.hCursor.0 as isize,
        screen_position: [info.ptScreenPos.x, info.ptScreenPos.y],
    })
}

pub unsafe fn decode_cursor(handle: isize) -> Result<CursorBitmap> {
    if handle == 0 {
        return Err(Error::from_win32());
    }
    let mut icon = ICONINFO::default();
    GetIconInfo(HICON(handle as *mut _), &mut icon)?;
    let owned = OwnedIconBitmaps {
        mask: icon.hbmMask,
        color: icon.hbmColor,
    };
    let hotspot = [
        icon.xHotspot.min(i32::MAX as u32) as i32,
        icon.yHotspot.min(i32::MAX as u32) as i32,
    ];

    if !owned.color.0.is_null() {
        let (width, height, bgra) = read_bitmap_bgra(owned.color)?;
        let alpha_is_empty = bgra.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0);
        let mask = if alpha_is_empty && !owned.mask.0.is_null() {
            Some(read_bitmap_bgra(owned.mask)?)
        } else {
            None
        };
        let mask_ref = mask.as_ref().map(|(mask_width, mask_height, mask_bgra)| {
            (*mask_width, *mask_height, mask_bgra.as_slice())
        });
        let (shape, rgba) =
            color_bitmap_to_rgba(width, height, &bgra, mask_ref).ok_or_else(Error::from_win32)?;
        return Ok(CursorBitmap {
            shape,
            pixels: rgba,
            hotspot,
        });
    }

    if owned.mask.0.is_null() {
        return Err(Error::from_win32());
    }
    let (packed_width, packed_height, bgra) = read_bitmap_bgra(owned.mask)?;
    let (shape, rgba) = monochrome_mask_to_rgba(packed_width, packed_height, &bgra)
        .ok_or_else(Error::from_win32)?;
    Ok(CursorBitmap {
        shape,
        pixels: rgba,
        hotspot,
    })
}

fn color_bitmap_to_rgba(
    width: u32,
    height: u32,
    bgra: &[u8],
    mask: Option<(u32, u32, &[u8])>,
) -> Option<(DecodedShape, Vec<u8>)> {
    if width == 0
        || height == 0
        || width > MAX_CURSOR_DIMENSION as u32
        || height > MAX_CURSOR_DIMENSION as u32
        || bgra.len() != width as usize * height as usize * 4
    {
        return None;
    }
    let alpha_is_empty = bgra.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0);
    let mask_matches = mask.is_some_and(|(mask_width, mask_height, bytes)| {
        mask_width == width
            && mask_height >= height
            && bytes.len() >= mask_width as usize * mask_height as usize * 4
    });
    let masked_color = alpha_is_empty && mask_matches;
    let mut rgba = vec![0u8; bgra.len()];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let index = (y * width as usize + x) * 4;
            let alpha = if mask_matches && (alpha_is_empty || bgra[index + 3] > 0) {
                let (mask_width, _, mask_bgra) = mask?;
                let and_bit = mask_bgra[(y * mask_width as usize + x) * 4] > 127;
                if masked_color {
                    if and_bit {
                        255
                    } else {
                        0
                    }
                } else if and_bit {
                    0
                } else {
                    bgra[index + 3]
                }
            } else if alpha_is_empty {
                255
            } else {
                bgra[index + 3]
            };
            if masked_color {
                rgba[index] = bgra[index + 2];
                rgba[index + 1] = bgra[index + 1];
                rgba[index + 2] = bgra[index];
            } else {
                rgba[index] = straight_channel_from_premultiplied(bgra[index + 2], alpha);
                rgba[index + 1] = straight_channel_from_premultiplied(bgra[index + 1], alpha);
                rgba[index + 2] = straight_channel_from_premultiplied(bgra[index], alpha);
            }
            rgba[index + 3] = alpha;
        }
    }
    Some((
        DecodedShape {
            width,
            height,
            mode: if masked_color { 2 } else { 0 },
        },
        rgba,
    ))
}

fn monochrome_mask_to_rgba(
    packed_width: u32,
    packed_height: u32,
    bgra: &[u8],
) -> Option<(DecodedShape, Vec<u8>)> {
    if packed_width == 0
        || packed_width > MAX_CURSOR_DIMENSION as u32
        || packed_height == 0
        || !packed_height.is_multiple_of(2)
        || packed_height > (MAX_CURSOR_DIMENSION * 2) as u32
        || bgra.len() != packed_width as usize * packed_height as usize * 4
    {
        return None;
    }
    let width = packed_width;
    let height = packed_height / 2;
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let and_pixel = bgra[(y * width as usize + x) * 4];
            let xor_pixel = bgra[((y + height as usize) * width as usize + x) * 4];
            let and_value = if and_pixel > 127 { 255 } else { 0 };
            let xor_value = if xor_pixel > 127 { 255 } else { 0 };
            let index = (y * width as usize + x) * 4;
            rgba[index..index + 3].fill(and_value);
            rgba[index + 3] = xor_value;
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

unsafe fn read_bitmap_bgra(bitmap: HBITMAP) -> Result<(u32, u32, Vec<u8>)> {
    let mut object = BITMAP::default();
    if GetObjectW(
        HGDIOBJ(bitmap.0),
        size_of::<BITMAP>() as i32,
        Some(&mut object as *mut BITMAP as *mut _),
    ) == 0
        || object.bmWidth <= 0
        || object.bmHeight <= 0
        || object.bmWidth > MAX_CURSOR_DIMENSION
        || object.bmHeight > MAX_CURSOR_DIMENSION * 2
    {
        return Err(Error::from_win32());
    }
    let width = object.bmWidth as u32;
    let height = object.bmHeight as u32;
    let byte_len = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(Error::from_win32)? as usize;
    let mut pixels = vec![0u8; byte_len];
    let hdc = GetDC(None);
    if hdc.0.is_null() {
        return Err(Error::from_win32());
    }
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: object.bmWidth,
            biHeight: -object.bmHeight,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: byte_len as u32,
            ..Default::default()
        },
        ..Default::default()
    };
    let rows = GetDIBits(
        hdc,
        bitmap,
        0,
        height,
        Some(pixels.as_mut_ptr() as *mut _),
        &mut info,
        DIB_RGB_COLORS,
    );
    let _ = ReleaseDC(None, hdc);
    if rows != height as i32 {
        return Err(Error::from_win32());
    }
    Ok((width, height, pixels))
}

pub fn cursor_bounds(position: [i32; 2], hotspot: [i32; 2], shape: DecodedShape) -> RECT {
    let left = position[0].saturating_sub(hotspot[0]);
    let top = position[1].saturating_sub(hotspot[1]);
    RECT {
        left,
        top,
        right: left.saturating_add(shape.width.min(i32::MAX as u32) as i32),
        bottom: top.saturating_add(shape.height.min(i32::MAX as u32) as i32),
    }
}

fn straight_channel_from_premultiplied(channel: u8, alpha: u8) -> u8 {
    if alpha == 0 {
        0
    } else {
        ((u16::from(channel) * 255 + u16::from(alpha) / 2) / u16::from(alpha)).min(255) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_bounds_use_hotspot_and_saturate() {
        let rect = cursor_bounds(
            [100, 200],
            [3, 4],
            DecodedShape {
                width: 10,
                height: 20,
                mode: 0,
            },
        );
        assert_eq!(
            (rect.left, rect.top, rect.right, rect.bottom),
            (97, 196, 107, 216)
        );
    }

    #[test]
    fn unpremultiplies_color_bitmap_channels_before_gpu_composition() {
        assert_eq!(straight_channel_from_premultiplied(64, 128), 128);
        assert_eq!(straight_channel_from_premultiplied(255, 0), 0);
        assert_eq!(straight_channel_from_premultiplied(200, 128), 255);
    }

    #[test]
    fn color_cursor_unpremultiplies_and_respects_legacy_and_mask() {
        let color = [30, 20, 10, 128, 60, 40, 20, 0];
        let mask = [0, 0, 0, 255, 255, 255, 255, 255];
        let (shape, rgba) = color_bitmap_to_rgba(2, 1, &color, Some((2, 1, &mask))).unwrap();
        assert_eq!(shape.mode, 0);
        assert_eq!(&rgba[..4], &[20, 40, 60, 128]);
        assert_eq!(&rgba[4..], &[0, 0, 0, 0]);
    }

    #[test]
    fn legacy_masked_color_maps_and_bits_to_xor_mode() {
        let color = [30, 20, 10, 0, 60, 40, 20, 0];
        let mask = [255, 255, 255, 255, 0, 0, 0, 255];
        let (shape, rgba) = color_bitmap_to_rgba(2, 1, &color, Some((2, 1, &mask))).unwrap();
        assert_eq!(shape.mode, 2);
        assert_eq!(&rgba[..4], &[10, 20, 30, 255]);
        assert_eq!(&rgba[4..], &[20, 40, 60, 0]);
    }

    #[test]
    fn monochrome_double_height_mask_decodes_and_xor_planes() {
        let packed = [
            255, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 255,
        ];
        let (shape, rgba) = monochrome_mask_to_rgba(2, 2, &packed).unwrap();
        assert_eq!((shape.width, shape.height, shape.mode), (2, 1, 1));
        assert_eq!(rgba, [255, 255, 255, 0, 0, 0, 0, 255]);
    }
}
