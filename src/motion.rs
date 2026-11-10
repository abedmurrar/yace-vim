//! Motions and text objects over a [`Text`], as Vim defines them.

use crate::text::{Text, class};

/// How a motion's range is taken by an operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Up to, not including, the target (`w`, `b`, `h`, `l`, `0`).
    Exclusive,
    /// Including the target char (`e`, `$`, `f`, `t`, `%`).
    Inclusive,
    /// Whole lines (`j`, `k`, `gg`, `G`).
    Linewise,
}

/// `w` / `W`: start of the next word.
pub fn word_forward(t: &Text, pos: usize, big: bool) -> usize {
    let n = t.len();
    if pos >= n {
        return n;
    }
    let c0 = class(t.chars[pos], big);
    let mut i = pos;
    // Skip the rest of this word (not blanks).
    if c0 != 0 {
        while i < n && class(t.chars[i], big) == c0 {
            i += 1;
        }
    }
    // Then blanks; an empty line counts as a word.
    while i < n && class(t.chars[i], big) == 0 {
        if t.chars[i] == '\n' && i + 1 < n && t.chars[i + 1] == '\n' {
            return i + 1;
        }
        i += 1;
    }
    i
}

/// `e` / `E`: end of this or the next word.
pub fn word_end(t: &Text, pos: usize, big: bool) -> usize {
    let n = t.len();
    if n == 0 {
        return 0;
    }
    let mut i = (pos + 1).min(n - 1);
    while i < n - 1 && class(t.chars[i], big) == 0 {
        i += 1;
    }
    let c = class(t.chars[i], big);
    while i + 1 < n && class(t.chars[i + 1], big) == c && c != 0 {
        i += 1;
    }
    i
}

/// `b` / `B`: start of this or the previous word.
pub fn word_back(t: &Text, pos: usize, big: bool) -> usize {
    if pos == 0 {
        return 0;
    }
    let mut i = pos - 1;
    while i > 0 && class(t.chars[i], big) == 0 {
        if t.chars[i] == '\n' && t.chars[i - 1] == '\n' {
            return i;
        }
        i -= 1;
    }
    let c = class(t.chars[i], big);
    while i > 0 && class(t.chars[i - 1], big) == c && c != 0 {
        i -= 1;
    }
    i
}

/// `ge` / `gE`: end of the previous word.
pub fn word_end_back(t: &Text, pos: usize, big: bool) -> usize {
    if pos == 0 {
        return 0;
    }
    let mut i = pos;
    let c0 = class(t.chars[i.min(t.len().saturating_sub(1))], big);
    if c0 != 0 {
        while i > 0 && class(t.chars[i], big) == c0 {
            i -= 1;
        }
    }
    while i > 0 && class(t.chars[i], big) == 0 {
        i -= 1;
    }
    i
}

/// `f`/`F`/`t`/`T`: the `count`-th `c` on the line.
pub fn find_char(
    t: &Text,
    pos: usize,
    c: char,
    forward: bool,
    till: bool,
    count: usize,
) -> Option<usize> {
    let line = t.line_of(pos);
    let (s, e) = (t.line_start(line), t.line_end(line));
    let mut i = pos;
    let mut found = 0;
    loop {
        if forward {
            i += 1;
            if i >= e {
                return None;
            }
        } else {
            if i <= s {
                return None;
            }
            i -= 1;
        }
        if t.chars[i] == c {
            found += 1;
            if found == count {
                return Some(match (till, forward) {
                    (false, _) => i,
                    (true, true) => i - 1,
                    (true, false) => i + 1,
                });
            }
        }
    }
}

/// `%`: the bracket matching the one under or after the cursor on the line.
pub fn match_pair(t: &Text, pos: usize) -> Option<usize> {
    let line = t.line_of(pos);
    let e = t.line_end(line);
    let start = (pos..e).find(|&i| matches!(t.chars[i], '(' | ')' | '[' | ']' | '{' | '}'))?;
    let c = t.chars[start];
    let (open, close, forward) = match c {
        '(' => ('(', ')', true),
        '[' => ('[', ']', true),
        '{' => ('{', '}', true),
        ')' => ('(', ')', false),
        ']' => ('[', ']', false),
        _ => ('{', '}', false),
    };
    let mut depth = 0i64;
    if forward {
        for i in start..t.len() {
            if t.chars[i] == open {
                depth += 1;
            } else if t.chars[i] == close {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
        }
    } else {
        for i in (0..=start).rev() {
            if t.chars[i] == close {
                depth += 1;
            } else if t.chars[i] == open {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
        }
    }
    None
}

fn blank_line(t: &Text, line: usize) -> bool {
    t.line_start(line) == t.line_end(line)
}

/// `}`: the next blank line after this paragraph (or the end).
pub fn paragraph_forward(t: &Text, pos: usize) -> usize {
    let mut line = t.line_of(pos);
    let last = t.lines() - 1;
    while line < last && blank_line(t, line) {
        line += 1;
    }
    while line < last && !blank_line(t, line) {
        line += 1;
    }
    if line == last && !blank_line(t, line) {
        t.line_end(line)
    } else {
        t.line_start(line)
    }
}

/// `{`: the blank line before this paragraph (or the start).
pub fn paragraph_back(t: &Text, pos: usize) -> usize {
    let mut line = t.line_of(pos);
    while line > 0 && blank_line(t, line) {
        line -= 1;
    }
    while line > 0 && !blank_line(t, line) {
        line -= 1;
    }
    t.line_start(line)
}

/// `iw` / `aw` (and `iW` / `aW`): `[start, end)`.
pub fn word_object(t: &Text, pos: usize, around: bool, big: bool) -> Option<(usize, usize)> {
    if t.is_empty() {
        return None;
    }
    let pos = pos.min(t.len() - 1);
    let line = t.line_of(pos);
    let (ls, le) = (t.line_start(line), t.line_end(line));
    let c = class(t.chars[pos], big);
    let (mut a, mut b) = (pos, pos + 1);
    while a > ls && class(t.chars[a - 1], big) == c {
        a -= 1;
    }
    while b < le && class(t.chars[b], big) == c {
        b += 1;
    }
    if around && c != 0 {
        // Trailing blanks, or leading ones when there are none after.
        let mut e = b;
        while e < le && class(t.chars[e], big) == 0 {
            e += 1;
        }
        if e > b {
            b = e;
        } else {
            while a > ls && class(t.chars[a - 1], big) == 0 {
                a -= 1;
            }
        }
    } else if around && b < le {
        // On blanks: they and the word after.
        let c2 = class(t.chars[b], big);
        while b < le && class(t.chars[b], big) == c2 {
            b += 1;
        }
    }
    Some((a, b))
}

/// `i"` / `a"` (and `'`, `` ` ``) on the cursor's line.
pub fn quote_object(t: &Text, pos: usize, q: char, around: bool) -> Option<(usize, usize)> {
    let line = t.line_of(pos);
    let (ls, le) = (t.line_start(line), t.line_end(line));
    let quotes: Vec<usize> = (ls..le)
        .filter(|&i| t.chars[i] == q && (i == ls || t.chars[i - 1] != '\\'))
        .collect();
    // The pair around the cursor, else the first pair after it.
    let pair = quotes
        .chunks(2)
        .filter(|p| p.len() == 2)
        .map(|p| (p[0], p[1]))
        .find(|&(a, b)| a <= pos && pos <= b)
        .or_else(|| {
            quotes
                .chunks(2)
                .filter(|p| p.len() == 2)
                .map(|p| (p[0], p[1]))
                .find(|&(a, _)| a > pos)
        })?;
    let (a, b) = pair;
    if around {
        let mut e = b + 1;
        while e < le && matches!(t.chars[e], ' ' | '\t') {
            e += 1;
        }
        Some((a, e))
    } else {
        Some((a + 1, b))
    }
}

/// `i(` / `a(` and the other brackets: the innermost pair around `pos`.
pub fn bracket_object(
    t: &Text,
    pos: usize,
    open: char,
    close: char,
    around: bool,
) -> Option<(usize, usize)> {
    let mut depth = 0i64;
    let mut a = None;
    let start = pos.min(t.len().saturating_sub(1));
    // On the closing bracket itself: that pair.
    let mut i = start as i64;
    if t.char(start) == Some(close) {
        i -= 1;
    }
    while i >= 0 {
        let c = t.chars[i as usize];
        if c == close {
            depth += 1;
        } else if c == open {
            if depth == 0 {
                a = Some(i as usize);
                break;
            }
            depth -= 1;
        }
        i -= 1;
    }
    let a = a?;
    let mut depth = 0i64;
    let mut b = None;
    for j in a + 1..t.len() {
        let c = t.chars[j];
        if c == open {
            depth += 1;
        } else if c == close {
            if depth == 0 {
                b = Some(j);
                break;
            }
            depth -= 1;
        }
    }
    let b = b?;
    if around {
        return Some((a, b + 1));
    }
    // Inner: a block's first and last lines stay when they hold only the
    // brackets (`ci{` on a multi-line block keeps them).
    let (mut s, mut e) = (a + 1, b);
    if t.char(s) == Some('\n') && t.line_of(a) != t.line_of(b) {
        s += 1;
        let lb = t.line_of(b);
        if t.first_non_blank(lb) == b {
            e = t.line_start(lb);
        }
    }
    Some((s, e.max(s)))
}

/// `ip` / `ap`: whole lines of the paragraph (and the blank lines after).
pub fn paragraph_object(t: &Text, pos: usize, around: bool) -> (usize, usize) {
    let line = t.line_of(pos);
    let blank = blank_line(t, line);
    let (mut a, mut b) = (line, line);
    while a > 0 && blank_line(t, a - 1) == blank {
        a -= 1;
    }
    while b + 1 < t.lines() && blank_line(t, b + 1) == blank {
        b += 1;
    }
    if around {
        while b + 1 < t.lines() && blank_line(t, b + 1) != blank {
            b += 1;
        }
    }
    (t.line_start(a), t.line_after(b))
}

/// `/`, `?`, `n`, `N`: the next match of `pat` (wrapping around). Lower
/// case patterns match any case (smartcase).
pub fn search(t: &Text, pos: usize, pat: &str, forward: bool) -> Option<usize> {
    let pat: Vec<char> = pat.chars().collect();
    if pat.is_empty() || pat.len() > t.len() {
        return None;
    }
    let fold = pat.iter().all(|c| !c.is_uppercase());
    let eq = |a: char, b: char| {
        if fold {
            a.to_lowercase().eq(b.to_lowercase())
        } else {
            a == b
        }
    };
    let at = |i: usize| (0..pat.len()).all(|k| eq(t.chars[i + k], pat[k]));
    let last = t.len() - pat.len();
    if forward {
        (pos + 1..=last).chain(0..=pos.min(last)).find(|&i| at(i))
    } else {
        (0..pos.min(last + 1))
            .rev()
            .chain((pos.min(last)..=last).rev())
            .find(|&i| at(i))
    }
}

/// The word under (or after) the cursor, for `*` and `#`.
pub fn word_at(t: &Text, pos: usize) -> Option<String> {
    let line = t.line_of(pos);
    let e = t.line_end(line);
    let start = (pos..e).find(|&i| class(t.chars[i], false) == 1)?;
    let (a, b) = word_object(t, start, false, false)?;
    Some(t.slice(a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words() {
        let t = Text::new("foo.bar  baz\nqux");
        assert_eq!(word_forward(&t, 0, false), 3, "w stops at punctuation");
        assert_eq!(word_forward(&t, 0, true), 9, "W skips it");
        assert_eq!(word_forward(&t, 9, false), 13, "across the line break");
        assert_eq!(word_end(&t, 0, false), 2);
        assert_eq!(word_end(&t, 2, false), 3);
        assert_eq!(word_back(&t, 9, false), 4);
        assert_eq!(word_back(&t, 13, false), 9);
        assert_eq!(word_end_back(&t, 9, false), 6);
        let blank = Text::new("a\n\nb");
        assert_eq!(word_forward(&blank, 0, false), 2, "an empty line is a word");
    }

    #[test]
    fn finds_and_pairs() {
        let t = Text::new("f(a, (b), c)\nx");
        assert_eq!(find_char(&t, 0, ',', true, false, 1), Some(3));
        assert_eq!(find_char(&t, 0, ',', true, false, 2), Some(8));
        assert_eq!(find_char(&t, 0, ',', true, true, 1), Some(2));
        assert_eq!(find_char(&t, 8, '(', false, false, 1), Some(5));
        assert_eq!(
            find_char(&t, 0, 'x', true, false, 1),
            None,
            "only on the line"
        );
        assert_eq!(match_pair(&t, 0), Some(11));
        assert_eq!(match_pair(&t, 11), Some(1));
        assert_eq!(match_pair(&t, 5), Some(7));
    }

    #[test]
    fn objects() {
        let t = Text::new("let s = \"a b\";  call(x, (y))\n");
        assert_eq!(word_object(&t, 1, false, false), Some((0, 3)));
        assert_eq!(word_object(&t, 1, true, false), Some((0, 4)));
        assert_eq!(quote_object(&t, 10, '"', false), Some((9, 12)));
        assert_eq!(
            quote_object(&t, 0, '"', true),
            Some((8, 13)),
            "the next pair, with its trailing blanks"
        );
        assert_eq!(bracket_object(&t, 26, '(', ')', false), Some((25, 26)));
        assert_eq!(bracket_object(&t, 22, '(', ')', false), Some((21, 27)));
        assert_eq!(bracket_object(&t, 22, '(', ')', true), Some((20, 28)));
        let block = Text::new("fn x() {\n    a;\n    b;\n}\n");
        assert_eq!(
            bracket_object(&block, 13, '{', '}', false),
            Some((9, 23)),
            "inner lines only"
        );
        let para = Text::new("a\nb\n\nc\n");
        assert_eq!(paragraph_object(&para, 0, false), (0, 4));
        assert_eq!(paragraph_object(&para, 0, true), (0, 5));
    }

    #[test]
    fn searching() {
        let t = Text::new("Foo foo FOO\nfoo");
        assert_eq!(
            search(&t, 0, "foo", true),
            Some(4),
            "smartcase: lower case matches any case"
        );
        assert_eq!(
            search(&t, 4, "Foo", true),
            Some(0),
            "wraps around, case-sensitive"
        );
        assert_eq!(search(&t, 4, "foo", false), Some(0));
        assert_eq!(search(&t, 0, "foo", false), Some(12), "wraps backwards");
        assert_eq!(word_at(&t, 3), Some("foo".into()));
    }

    #[test]
    fn paragraphs() {
        let t = Text::new("a\nb\n\nc\nd");
        assert_eq!(paragraph_forward(&t, 0), 4);
        assert_eq!(paragraph_forward(&t, 4), 8, "the end");
        assert_eq!(paragraph_back(&t, 6), 4);
    }
}
