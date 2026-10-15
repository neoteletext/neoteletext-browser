//! The trusted strip, drawn by the display host itself. In the proof it is a
//! band of colour with a text field behind it: what the user types there,
//! followed by Enter, is strip input.

use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
use smithay::input::keyboard::Keysym;

/// The strip's height in logical pixels.
pub const STRIP_HEIGHT: i32 = 40;
/// Bound on what the strip holds: the longest address the trusted core
/// accepts would be the real bound.
const MAX_TEXT_CHARACTERS: usize = 2048;
/// Colours, red, green, blue and alpha; the tests read them back from the
/// screen as 0, 51, 153 and 0, 102, 230.
const COLOUR: [f32; 4] = [0.0, 0.2, 0.6, 1.0];
const COLOUR_FOCUSED: [f32; 4] = [0.0, 0.4, 0.9, 1.0];

pub struct Strip {
    focused: bool,
    text: String,
    buffer: SolidColorBuffer,
}

impl Strip {
    pub fn new() -> Self {
        Self { focused: false, text: String::new(), buffer: SolidColorBuffer::new((1, STRIP_HEIGHT), COLOUR) }
    }

    pub fn focused(&self) -> bool {
        self.focused
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
        self.buffer.set_color(if focused { COLOUR_FOCUSED } else { COLOUR });
    }

    pub fn set_width(&mut self, width: i32) {
        self.buffer.resize((width.max(1), STRIP_HEIGHT));
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Takes text composed by the user's input method.
    pub fn insert(&mut self, text: &str) {
        let room = MAX_TEXT_CHARACTERS.saturating_sub(self.text.chars().count());
        self.text.extend(text.chars().filter(|character| !character.is_control()).take(room));
    }

    /// Takes one key press; returns the line when Enter completes it.
    pub fn key(&mut self, symbol: Keysym) -> Option<String> {
        match symbol {
            Keysym::Return | Keysym::KP_Enter => Some(std::mem::take(&mut self.text)),
            Keysym::BackSpace => {
                self.text.pop();
                None
            }
            _ => {
                if let Some(character) = symbol.key_char().filter(|character| !character.is_control())
                    && self.text.chars().count() < MAX_TEXT_CHARACTERS
                {
                    self.text.push(character);
                }
                None
            }
        }
    }

    pub fn render_element(&self) -> SolidColorRenderElement {
        SolidColorRenderElement::from_buffer(&self.buffer, (0, 0), 1.0, 1.0, Kind::Unspecified)
    }
}
