//! Highlight text.
use crate::Color;
use crate::font;

/// A type that describes how to highlight an `Input`
/// with some [`Style`].
pub trait Highlighter<Input, Theme = crate::Theme> {
    /// A unique identifier for the highlighter.
    fn id(&self) -> &str;

    /// Returns the [`Style`] of the given `Input`.
    fn highlight(&self, input: Input, theme: &Theme) -> Style;
}

/// The style of some highlighted text.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Style {
    /// The [`Color`] of the text.
    pub color: Option<Color>,
    /// The [`font::Style`] of the text.
    pub style: Option<font::Style>,
    /// The underline decoration of the text.
    pub underline: Option<Underline>,
    /// The [`Color`] of the underline.
    pub underline_color: Option<Color>,
    /// Whether the text is strikethrough.
    pub strikethrough: bool,
    /// The [`Color`] of the strikethrough.
    pub strikethrough_color: Option<Color>,
    /// Whether the text is overlined.
    pub overline: bool,
    /// The [`Color`] of the overline.
    pub overline_color: Option<Color>,
}

impl<T, Input, Theme> Highlighter<Input, Theme> for T
where
    T: Fn(Input, &Theme) -> Style,
{
    fn id(&self) -> &str {
        std::any::type_name_of_val(self) // Hack: Best effort
    }

    fn highlight(&self, input: Input, theme: &Theme) -> Style {
        (self)(input, theme)
    }
}

impl Style {
    /// Returns whether the [`Format`] needs to be processed.
    pub fn needs_processing(&self) -> bool {
        self.color.is_some()
            || self.style.is_some()
            || self.underline.is_some()
            || self.strikethrough
            || self.overline
    }
}

/// The underline decoration of text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Underline {
    /// A single underline.
    Single,
    /// A double underline.
    Double,
}
