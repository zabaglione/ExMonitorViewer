#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FitRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

pub fn fit_rect(
    source_width: f32,
    source_height: f32,
    viewport_width: f32,
    viewport_height: f32,
) -> FitRect {
    if source_width <= 0.0
        || source_height <= 0.0
        || viewport_width <= 0.0
        || viewport_height <= 0.0
    {
        return FitRect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        };
    }

    let scale = (viewport_width / source_width).min(viewport_height / source_height);
    let width = source_width * scale;
    let height = source_height * scale;
    FitRect {
        x: (viewport_width - width) * 0.5,
        y: (viewport_height - height) * 0.5,
        width,
        height,
    }
}

pub fn fit_client_size(
    source_width: u32,
    source_height: u32,
    available_width: u32,
    available_height: u32,
) -> (u32, u32) {
    if source_width == 0 || source_height == 0 || available_width == 0 || available_height == 0 {
        return (0, 0);
    }
    let scale = (available_width as f64 / source_width as f64)
        .min(available_height as f64 / source_height as f64);
    (
        ((source_width as f64 * scale).round() as u32).max(1),
        ((source_height as f64 * scale).round() as u32).max(1),
    )
}

pub fn scale_client_size(source_width: u32, source_height: u32, percent: u32) -> (u32, u32) {
    if source_width == 0 || source_height == 0 || percent == 0 {
        return (0, 0);
    }
    (
        ((source_width as u64 * percent as u64 + 50) / 100).max(1) as u32,
        ((source_height as u64 * percent as u64 + 50) / 100).max(1) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_letterbox_without_cropping() {
        assert_eq!(
            fit_rect(1920.0, 1080.0, 900.0, 900.0),
            FitRect {
                x: 0.0,
                y: 196.875,
                width: 900.0,
                height: 506.25,
            }
        );
    }

    #[test]
    fn fits_portrait_without_cropping() {
        assert_eq!(
            fit_rect(1080.0, 1920.0, 960.0, 540.0),
            FitRect {
                x: 328.125,
                y: 0.0,
                width: 303.75,
                height: 540.0,
            }
        );
    }

    #[test]
    fn returns_empty_for_zero_sized_viewports() {
        assert_eq!(
            fit_rect(1920.0, 1080.0, 0.0, 900.0),
            FitRect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            }
        );
    }

    #[test]
    fn fit_client_size_preserves_aspect_ratio_and_fills_available_bounds() {
        assert_eq!(fit_client_size(1920, 1080, 1000, 700), (1000, 563));
        assert_eq!(fit_client_size(1080, 1920, 900, 1000), (563, 1000));
        assert_eq!(fit_client_size(1920, 1080, 0, 1000), (0, 0));
    }

    #[test]
    fn scale_client_size_uses_source_pixels_as_one_hundred_percent() {
        assert_eq!(scale_client_size(1920, 1080, 100), (1920, 1080));
        assert_eq!(scale_client_size(1920, 1080, 25), (480, 270));
        assert_eq!(scale_client_size(640, 480, 25), (160, 120));
    }
}
