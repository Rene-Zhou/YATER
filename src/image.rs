use crate::cli::ImageMode;
use ratatui_image::picker::ProtocolType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageModeSupport {
    pub kitty: bool,
    pub iterm2: bool,
    pub sixel: bool,
    pub halfblock: bool,
}

impl ImageModeSupport {
    pub fn terminal_default() -> Self {
        Self {
            kitty: false,
            iterm2: false,
            sixel: false,
            halfblock: true,
        }
    }

    pub fn from_ratatui_capabilities<'a>(
        capabilities: impl IntoIterator<Item = &'a ratatui_image::picker::Capability>,
    ) -> Self {
        let capabilities = capabilities.into_iter().collect::<Vec<_>>();
        Self {
            kitty: capabilities
                .iter()
                .any(|capability| matches!(capability, ratatui_image::picker::Capability::Kitty)),
            iterm2: false,
            sixel: capabilities
                .iter()
                .any(|capability| matches!(capability, ratatui_image::picker::Capability::Sixel)),
            halfblock: true,
        }
    }

    fn from_picker(picker: &ratatui_image::picker::Picker) -> Self {
        let mut support = Self::from_ratatui_capabilities(picker.capabilities());
        match picker.protocol_type() {
            ProtocolType::Kitty => support.kitty = true,
            ProtocolType::Iterm2 => support.iterm2 = true,
            ProtocolType::Sixel => support.sixel = true,
            ProtocolType::Halfblocks => {}
        }
        support
    }

    pub fn detect_terminal() -> Self {
        ratatui_image::picker::Picker::from_query_stdio()
            .map(|picker| Self::from_picker(&picker))
            .unwrap_or_else(|_| Self::terminal_default())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectedImageMode {
    Kitty,
    Iterm2,
    Sixel,
    Halfblock,
    Off,
}

impl SelectedImageMode {
    pub fn protocol_type(self) -> Option<ProtocolType> {
        match self {
            Self::Kitty => Some(ProtocolType::Kitty),
            Self::Iterm2 => Some(ProtocolType::Iterm2),
            Self::Sixel => Some(ProtocolType::Sixel),
            Self::Halfblock | Self::Off => None,
        }
    }
}

pub fn select_image_mode(mode: ImageMode, support: ImageModeSupport) -> SelectedImageMode {
    match mode {
        ImageMode::Sixel => SelectedImageMode::Sixel,
        ImageMode::Halfblock => SelectedImageMode::Halfblock,
        ImageMode::Off => SelectedImageMode::Off,
        ImageMode::Auto if support.sixel => SelectedImageMode::Sixel,
        ImageMode::Auto if support.kitty => SelectedImageMode::Kitty,
        ImageMode::Auto if support.iterm2 => SelectedImageMode::Iterm2,
        ImageMode::Auto if support.halfblock => SelectedImageMode::Halfblock,
        ImageMode::Auto => SelectedImageMode::Off,
    }
}

pub fn resolve_image_mode(
    mode: ImageMode,
    detect_support: impl FnOnce() -> ImageModeSupport,
) -> SelectedImageMode {
    match mode {
        ImageMode::Auto => select_image_mode(mode, detect_support()),
        ImageMode::Sixel | ImageMode::Halfblock | ImageMode::Off => {
            select_image_mode(mode, ImageModeSupport::terminal_default())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use crate::cli::ImageMode;

    use super::{ImageModeSupport, SelectedImageMode, resolve_image_mode, select_image_mode};

    #[test]
    fn auto_prefers_sixel_when_supported() {
        let selected = select_image_mode(
            ImageMode::Auto,
            ImageModeSupport {
                kitty: false,
                iterm2: false,
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
                kitty: false,
                iterm2: false,
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
                kitty: true,
                iterm2: true,
                sixel: true,
                halfblock: true,
            },
        );

        assert_eq!(selected, SelectedImageMode::Off);
    }

    #[test]
    fn explicit_image_mode_does_not_query_terminal_support() {
        let detector_called = Cell::new(false);

        let selected = resolve_image_mode(ImageMode::Off, || {
            detector_called.set(true);
            ImageModeSupport::terminal_default()
        });

        assert_eq!(selected, SelectedImageMode::Off);
        assert!(!detector_called.get());
    }

    #[test]
    fn auto_prefers_sixel_over_kitty_when_both_are_supported() {
        let selected = select_image_mode(
            ImageMode::Auto,
            ImageModeSupport {
                kitty: true,
                iterm2: false,
                sixel: true,
                halfblock: true,
            },
        );

        assert_eq!(selected, SelectedImageMode::Sixel);
    }

    #[test]
    fn auto_falls_back_to_kitty_without_sixel() {
        let selected = select_image_mode(
            ImageMode::Auto,
            ImageModeSupport {
                kitty: true,
                iterm2: false,
                sixel: false,
                halfblock: true,
            },
        );

        assert_eq!(selected, SelectedImageMode::Kitty);
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

    #[test]
    fn ratatui_kitty_capability_enables_kitty_auto_mode() {
        let capabilities = [ratatui_image::picker::Capability::Kitty];
        let support = ImageModeSupport::from_ratatui_capabilities(&capabilities);

        assert_eq!(
            select_image_mode(ImageMode::Auto, support),
            SelectedImageMode::Kitty
        );
    }
}
