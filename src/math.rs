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
}
