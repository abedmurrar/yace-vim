//! The document as chars, with line starts, and the character classes
//! Vim's word motions use.

/// A snapshot of a document's text.
#[derive(Debug, Clone, Default)]
pub struct Text {
    pub chars: Vec<char>,
    /// Char offset where each line starts.
    starts: Vec<usize>,
}

impl Text {
    pub fn new(s: &str) -> Self {
        let chars: Vec<char> = s.chars().collect();
        let mut starts = vec![0];
        for (i, c) in chars.iter().enumerate() {
            if *c == '\n' {
                starts.push(i + 1);
            }
        }
        Self { chars, starts }
    }

    pub fn len(&self) -> usize {
        self.chars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    pub fn char(&self, i: usize) -> Option<char> {
        self.chars.get(i).copied()
    }

    /// Number of lines (a final newline doesn't start a new one, as in Vim).
    pub fn lines(&self) -> usize {
        let n = self.starts.len();
        if n > 1 && self.starts[n - 1] == self.chars.len() {
            n - 1
        } else {
            n
        }
    }

    pub fn line_of(&self, pos: usize) -> usize {
        match self.starts.binary_search(&pos) {
            Ok(l) => l.min(self.lines() - 1),
            Err(l) => (l - 1).min(self.lines() - 1),
        }
    }

    pub fn line_start(&self, line: usize) -> usize {
        self.starts[line.min(self.lines() - 1)]
    }

    /// End of the line's text: before its `\n` (and a `\r` before it).
    pub fn line_end(&self, line: usize) -> usize {
        let line = line.min(self.lines() - 1);
        let mut end = self
            .starts
            .get(line + 1)
            .map(|s| s - 1)
            .unwrap_or(self.chars.len());
        if end > self.starts[line] && self.chars.get(end - 1) == Some(&'\r') {
            end -= 1;
        }
        end
    }

    /// Start of the next line (or the end of the text).
    pub fn line_after(&self, line: usize) -> usize {
        self.starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.chars.len())
    }

    /// Last position the Normal-mode cursor may be on in `line`.
    pub fn last_col_pos(&self, line: usize) -> usize {
        let (s, e) = (self.line_start(line), self.line_end(line));
        if e > s { e - 1 } else { s }
    }

    /// First non-blank char of `line` (or its end).
    pub fn first_non_blank(&self, line: usize) -> usize {
        let (s, e) = (self.line_start(line), self.line_end(line));
        (s..e)
            .find(|&i| !matches!(self.chars[i], ' ' | '\t'))
            .unwrap_or(e)
    }

    pub fn col(&self, pos: usize) -> usize {
        pos - self.line_start(self.line_of(pos))
    }

    /// `pos` clamped for Normal mode: on a char, not after the line's end.
    pub fn clamp_normal(&self, pos: usize) -> usize {
        let line = self.line_of(pos.min(self.len()));
        pos.clamp(self.line_start(line), self.last_col_pos(line))
    }

    /// The line's leading whitespace.
    pub fn indent(&self, line: usize) -> String {
        let s = self.line_start(line);
        self.chars[s..self.first_non_blank(line)].iter().collect()
    }

    pub fn slice(&self, a: usize, b: usize) -> String {
        self.chars[a.min(self.len())..b.min(self.len())]
            .iter()
            .collect()
    }
}

/// Character classes for `w`/`b`/`e`: blanks, word chars, punctuation.
/// With `big` (`W`/`B`/`E`) everything that isn't blank is one class.
pub fn class(c: char, big: bool) -> u8 {
    if c.is_whitespace() {
        0
    } else if big || c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_ends() {
        let t = Text::new("ab\n\ncd\r\nef");
        assert_eq!(t.lines(), 4);
        assert_eq!(
            (t.line_start(2), t.line_end(2)),
            (4, 6),
            "the \\r is not text"
        );
        assert_eq!(t.line_of(5), 2);
        assert_eq!(t.last_col_pos(1), 3, "empty line: its start");
        assert_eq!(Text::new("a\n").lines(), 1, "a final newline is not a line");
        assert_eq!(Text::new("").lines(), 1);
        assert_eq!(Text::new("  x").first_non_blank(0), 2);
    }
}
