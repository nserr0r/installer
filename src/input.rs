/// Однострочное поле ввода с курсором; позиция курсора считается в символах.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Input {
    text: String,
    cursor: usize,
}

impl Input {
    pub fn with(text: &str) -> Self {
        Self {
            text: text.to_string(),
            cursor: text.chars().count(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn clear(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }

    pub fn insert(&mut self, symbol: char) {
        let at = self.byte(self.cursor);

        self.text.insert(at, symbol);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let at = self.byte(self.cursor);

            self.text.remove(at);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.len() {
            let at = self.byte(self.cursor);

            self.text.remove(at);
        }
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.len());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.len();
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn byte(&self, position: usize) -> usize {
        self.text
            .char_indices()
            .nth(position)
            .map_or(self.text.len(), |(index, _)| index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_inserts_at_the_cursor_and_editing_works_with_multibyte_text() {
        let mut input = Input::with("привет");

        input.left();
        input.left();
        input.insert('-');
        assert_eq!((input.text(), input.cursor()), ("прив-ет", 5));

        input.backspace();
        input.delete();
        assert_eq!((input.text(), input.cursor()), ("привт", 4));

        input.home();
        input.delete();
        assert_eq!(input.text(), "ривт");

        input.end();
        input.insert('!');
        assert_eq!(input.text(), "ривт!");
    }

    #[test]
    fn the_cursor_stays_inside_the_text() {
        let mut input = Input::with("ab");

        input.right();
        input.right();
        assert_eq!(input.cursor(), 2);

        input.home();
        input.left();
        input.backspace();
        assert_eq!((input.text(), input.cursor()), ("ab", 0));
        assert_eq!(input.clear(), "ab");
        assert!(input.is_empty());
    }
}
