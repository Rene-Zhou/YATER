use crate::cli::ImageMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageModeSupport {
    pub sixel: bool,
    pub halfblock: bool,
}

impl ImageModeSupport {
    pub fn terminal_default() -> Self {
        Self {
            sixel: false,
            halfblock: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectedImageMode {
    Sixel,
    Halfblock,
    Off,
}

pub fn select_image_mode(mode: ImageMode, support: ImageModeSupport) -> SelectedImageMode {
    match mode {
        ImageMode::Sixel => SelectedImageMode::Sixel,
        ImageMode::Halfblock => SelectedImageMode::Halfblock,
        ImageMode::Off => SelectedImageMode::Off,
        ImageMode::Auto if support.sixel => SelectedImageMode::Sixel,
        ImageMode::Auto if support.halfblock => SelectedImageMode::Halfblock,
        ImageMode::Auto => SelectedImageMode::Off,
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::ImageMode;

    use super::{select_image_mode, ImageModeSupport, SelectedImageMode};

    #[test]
    fn auto_prefers_sixel_when_supported() {
        let selected = select_image_mode(
            ImageMode::Auto,
            ImageModeSupport {
                sixel: true,
                halfblock: true,
            },
        );

        assert_eq!(selected, SelectedImageMode::Sixel);
    }

    #[test]
    fn auto_falls_back_to_halfblock_without_sixel() {
        let selected = select_image_mode(
            ImageMode::Auto,
            ImageModeSupport {
                sixel: false,
                halfblock: true,
            },
        );

        assert_eq!(selected, SelectedImageMode::Halfblock);
    }

    #[test]
    fn explicit_off_disables_images_even_when_supported() {
        let selected = select_image_mode(
            ImageMode::Off,
            ImageModeSupport {
                sixel: true,
                halfblock: true,
            },
        );

        assert_eq!(selected, SelectedImageMode::Off);
    }
}
