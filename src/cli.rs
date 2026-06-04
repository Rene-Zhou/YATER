use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageMode {
    Auto,
    Kitty,
    Iterm2,
    Sixel,
    Halfblock,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ImageModeOverride {
    Kitty,
    Iterm2,
    Sixel,
    Halfblock,
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cli {
    pub file: PathBuf,
    pub image_mode: ImageMode,
}

#[derive(Debug, Clone, Parser)]
#[command(name = "yater", version, about = "Yet Another Terminal Epub Reader")]
struct CliArgs {
    file: PathBuf,

    #[arg(long, value_enum)]
    image_mode: Option<ImageModeOverride>,
}

pub fn parse_from<I, T>(args: I) -> Result<Cli, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    CliArgs::try_parse_from(args).map(|args| Cli {
        file: args.file,
        image_mode: args.image_mode.map_or(ImageMode::Auto, ImageMode::from),
    })
}

impl From<ImageModeOverride> for ImageMode {
    fn from(mode: ImageModeOverride) -> Self {
        match mode {
            ImageModeOverride::Kitty => Self::Kitty,
            ImageModeOverride::Iterm2 => Self::Iterm2,
            ImageModeOverride::Sixel => Self::Sixel,
            ImageModeOverride::Halfblock => Self::Halfblock,
            ImageModeOverride::Off => Self::Off,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{parse_from, ImageMode};

    #[test]
    fn parses_epub_path_and_image_mode_override() {
        let cli = parse_from(["yater", "book.epub", "--image-mode=halfblock"]).expect("parse CLI");

        assert_eq!(cli.file, PathBuf::from("book.epub"));
        assert_eq!(cli.image_mode, ImageMode::Halfblock);
    }

    #[test]
    fn defaults_to_auto_image_detection_when_mode_is_omitted() {
        let cli = parse_from(["yater", "book.epub"]).expect("parse CLI");

        assert_eq!(cli.image_mode, ImageMode::Auto);
    }

    #[test]
    fn parses_all_documented_image_modes() {
        let cases = [
            ("kitty", ImageMode::Kitty),
            ("iterm2", ImageMode::Iterm2),
            ("sixel", ImageMode::Sixel),
            ("halfblock", ImageMode::Halfblock),
            ("off", ImageMode::Off),
        ];

        for (raw_mode, expected_mode) in cases {
            let cli =
                parse_from(["yater", "book.epub", "--image-mode", raw_mode]).expect("parse CLI");

            assert_eq!(cli.image_mode, expected_mode);
        }
    }

    #[test]
    fn rejects_unknown_image_mode() {
        let error = parse_from(["yater", "book.epub", "--image-mode=bitmap"])
            .expect_err("invalid image mode");

        assert_eq!(error.kind(), clap::error::ErrorKind::InvalidValue);
    }
}
