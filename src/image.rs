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

    pub fn from_ratatui_capabilities<'a>(
        capabilities: impl IntoIterator<Item = &'a ratatui_image::picker::Capability>,
    ) -> Self {
        Self {
            sixel: capabilities
                .into_iter()
                .any(|capability| matches!(capability, ratatui_image::picker::Capability::Sixel)),
            halfblock: true,
        }
    }

    pub fn detect_terminal() -> Self {
        ratatui_image::picker::Picker::from_query_stdio()
            .map(|picker| Self::from_ratatui_capabilities(picker.capabilities()))
            .unwrap_or_else(|_| Self::terminal_default())
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

    #[test]
    fn ratatui_sixel_capability_enables_sixel_auto_mode() {
        let capabilities = [ratatui_image::picker::Capability::Sixel];
        let support = ImageModeSupport::from_ratatui_capabilities(&capabilities);

        assert_eq!(
            select_image_mode(ImageMode::Auto, support),
            SelectedImageMode::Sixel
        );
    }
}
