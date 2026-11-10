//! Engine tests: keys typed on a text, compared with Vim's result. `|`
//! marks the cursor in the input and the expected output.

use crate::{
    engine::{Action, Ctx, K, Mode, Vim},
    text::Text,
};

/// Parse "<esc>", "<cr>", "<bs>", "<c-r>" and plain chars.
fn keys(s: &str) -> Vec<K> {
    let mut out = Vec::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '<' {
            let name: String = it.by_ref().take_while(|&c| c != '>').collect();
            out.push(match name.as_str() {
                "esc" => K::Named("escape".into()),
                "cr" => K::Named("enter".into()),
                "bs" => K::Named("backspace".into()),
                "tab" => K::Named("tab".into()),
                "lt" => K::Char('<'),
                n if n.starts_with("c-") => K::Named(format!("ctrl+{}", &n[2..])),
                n => K::Named(n.into()),
            });
        } else {
            out.push(K::Char(c));
        }
    }
    out
}

/// Run `typed` on `before` (cursor at `|`) the way YACE would: keys the
/// engine passes on in Insert mode are typed like the editor would.
fn run_with(vim: &mut Vim, before: &str, typed: &str) -> String {
    let pos = before.find('|').expect("cursor");
    let text: String = before.replacen('|', "", 1);
    let mut t = Text::new(&text);
    let mut buf: Vec<char> = text.chars().collect();
    let mut cursor = before[..pos].chars().count();
    let mut anchor = cursor;
    vim.cursor = cursor;
    let ctx = Ctx::default();
    for k in keys(typed) {
        if vim.insert_passes(&k) {
            // The editor types it.
            match &k {
                K::Char(c) => {
                    buf.insert(cursor, *c);
                    cursor += 1;
                }
                K::Named(n) if n == "enter" => {
                    buf.insert(cursor, '\n');
                    cursor += 1;
                }
                K::Named(n) if n == "backspace" && cursor > 0 => {
                    cursor -= 1;
                    buf.remove(cursor);
                }
                _ => {}
            }
            t = Text::new(&buf.iter().collect::<String>());
            vim.cursor = cursor;
            anchor = cursor;
            continue;
        }
        for a in vim.feed(k, &mut t, cursor, anchor, &ctx) {
            match a {
                Action::Edit { start, end, text } => {
                    buf.splice(start..end, text.chars());
                }
                Action::Cursor(p) => {
                    cursor = p;
                    anchor = p;
                }
                Action::Select { anchor: a, head } => {
                    anchor = a;
                    cursor = head;
                }
                _ => {}
            }
        }
        assert_eq!(
            buf.iter().collect::<String>(),
            t.chars.iter().collect::<String>(),
            "engine and editor agree"
        );
        // The engine's own idea of the cursor, as the editor would report it.
        if matches!(vim.mode, Mode::Normal | Mode::Insert | Mode::Replace) {
            cursor = vim.cursor;
            anchor = cursor;
        }
    }
    let mut out: String = buf.iter().collect();
    let byte = out
        .char_indices()
        .nth(cursor)
        .map(|(i, _)| i)
        .unwrap_or(out.len());
    out.insert(byte, '|');
    out
}

fn run(before: &str, typed: &str) -> String {
    run_with(&mut Vim::new(), before, typed)
}

#[test]
fn motions() {
    assert_eq!(run("|foo bar baz", "w"), "foo |bar baz");
    assert_eq!(run("|foo bar baz", "2w"), "foo bar |baz");
    assert_eq!(run("foo bar |baz", "b"), "foo |bar baz");
    assert_eq!(run("|foo bar", "e"), "fo|o bar");
    assert_eq!(run("|foo bar", "$"), "foo ba|r");
    assert_eq!(run("  foo|", "^"), "  |foo");
    assert_eq!(run("  fo|o", "0"), "|  foo");
    assert_eq!(run("|a\nbb\nccc", "jj"), "a\nbb\n|ccc");
    assert_eq!(
        run("ab|c\nd\nefg", "jj"),
        "abc\nd\nef|g",
        "the column is kept across a short line"
    );
    assert_eq!(run("|a\nb\nc", "G"), "a\nb\n|c");
    assert_eq!(run("a\nb\n|c", "gg"), "|a\nb\nc");
    assert_eq!(run("|a\nb\nc", "2G"), "a\n|b\nc");
    assert_eq!(run("|a(b)c", "f)"), "a(b|)c");
    assert_eq!(run("|a(b)c", "t)"), "a(|b)c");
    assert_eq!(run("|a,b,c", "f,;"), "a,b|,c");
    assert_eq!(run("|(a [b] c)", "%"), "(a [b] c|)");
    assert_eq!(run("|x foo x foo", "/foo<cr>n"), "x foo x |foo");
    assert_eq!(run("|foo x foo", "*"), "foo x |foo");
    assert_eq!(run("|a\n\nb", "}"), "a\n|\nb");
    assert_eq!(run("|abc", "l"), "a|bc");
    assert_eq!(run("ab|c", "l"), "ab|c", "not past the last char");
}

#[test]
fn operators() {
    assert_eq!(run("|foo bar", "dw"), "|bar");
    assert_eq!(run("|foo bar", "d2w"), "|");
    assert_eq!(
        run("foo |bar\nbaz", "dw"),
        "foo| \nbaz",
        "dw on the last word stops at the line end"
    );
    assert_eq!(run("|foo bar", "cwxy<esc>"), "x|y bar");
    assert_eq!(run("|foo bar", "de"), "| bar");
    assert_eq!(run("|foo bar", "d$"), "|");
    assert_eq!(run("foo b|ar", "D"), "foo |b");
    assert_eq!(run("a\n|b\nc", "dd"), "a\n|c");
    assert_eq!(
        run("a\nb\n|c", "dd"),
        "a\n|b",
        "the last line takes the newline before it"
    );
    assert_eq!(run("|a\nb\nc", "2dd"), "|c");
    assert_eq!(run("|a\nb\nc", "dj"), "|c");
    assert_eq!(
        run("  |foo\nbar", "ccx<esc>"),
        "  |x\nbar",
        "cc keeps the indentation"
    );
    assert_eq!(run("|abc", "x"), "|bc");
    assert_eq!(run("|abc", "2x"), "|c");
    assert_eq!(run("ab|c", "X"), "a|c");
    assert_eq!(run("|abc", "rx"), "|xbc");
    assert_eq!(run("|abc", "~~"), "AB|c");
    assert_eq!(run("|a\n  b", "J"), "a| b");
    assert_eq!(run("|foo", ">>"), "    |foo");
    assert_eq!(run("    |foo", "<lt><lt>"), "|foo");
    assert_eq!(run("|foo bar", "gUiw"), "|FOO bar");
}

#[test]
fn text_objects() {
    assert_eq!(run("say \"he|llo\" now", "ci\"x<esc>"), "say \"|x\" now");
    assert_eq!(run("say \"he|llo\" now", "da\""), "say |now");
    assert_eq!(run("f(a, |b)", "di("), "f(|)");
    assert_eq!(run("f(a, |b)", "da("), "|f");
    assert_eq!(run("foo b|ar baz", "diw"), "foo | baz");
    assert_eq!(run("foo b|ar baz", "daw"), "foo |baz");
    assert_eq!(run("{\n  |a;\n}", "ci{x<esc>"), "{\n|x}");
}

#[test]
fn yank_put_registers() {
    assert_eq!(run("|foo bar", "yiwP"), "fo|ofoo bar");
    assert_eq!(run("|a\nb", "yyp"), "a\n|a\nb");
    assert_eq!(run("|a\nb", "ddp"), "b\n|a");
    assert_eq!(run("|ab", "xp"), "b|a");
    assert_eq!(run("|one two", "\"ayiwwdiw\"aP"), "oneon|e ");
    assert_eq!(run("|one", "\"_dd"), "|", "the black hole register");
    assert_eq!(run("|a\nb", "yy\"_ddp"), "b\n|a");
    assert_eq!(run("|x", "yl3p"), "xxx|x");
}

#[test]
fn insert_mode() {
    assert_eq!(run("|bc", "ia<esc>"), "|abc");
    assert_eq!(run("|ab", "ax<esc>"), "a|xb");
    assert_eq!(run("a|b", "Ax<esc>"), "ab|x");
    assert_eq!(run("  a|b", "Ix<esc>"), "  |xab");
    assert_eq!(
        run("  |a\nb", "ox<esc>"),
        "  a\n  |x\nb",
        "o keeps the indentation"
    );
    assert_eq!(run("a\n|b", "Ox<esc>"), "a\n|x\nb");
    assert_eq!(run("|abc", "Rxy<esc>"), "x|yc");
    assert_eq!(
        run("foo b|ar", "a<c-w><esc>"),
        "foo| r",
        "Ctrl+W deletes the word; Esc steps back"
    );
}

#[test]
fn visual_mode() {
    assert_eq!(run("|foo bar", "vld"), "|o bar");
    assert_eq!(run("|foo bar", "vey$p"), "foo barfo|o");
    assert_eq!(run("a\n|b\nc\nd", "Vjd"), "a\n|d");
    assert_eq!(run("|foo bar", "viwU"), "|FOO bar");
    assert_eq!(run("a|b", "v<esc>x"), "|a");
    assert_eq!(run("|a\nb", "VJ"), "a| b");
}

#[test]
fn repeat_and_macros() {
    assert_eq!(run("|a b c d", "dw."), "|c d");
    assert_eq!(run("|a b c d", "dw2."), "|d");
    assert_eq!(run("|x\ny", "Ahi<esc>j."), "xhi\nyh|i");
    assert_eq!(run("|1\n2\n3", "qaA!<esc>jq@a@@"), "1!\n2!\n3|!");
    assert_eq!(
        run("|abc", "xu").replace('|', ""),
        "bc",
        "undo is the editor's (a command)"
    );
}

#[test]
fn ex_commands() {
    let mut vim = Vim::new();
    assert_eq!(run_with(&mut vim, "|a\nb\nc", ":3<cr>"), "a\nb\n|c");
    assert_eq!(run("|foo foo\nfoo", ":%s/foo/bar/g<cr>"), "bar bar\n|bar");
    assert_eq!(run("|foo foo\nfoo", ":s/foo/bar/<cr>"), "|bar foo\nfoo");
    let mut vim = Vim::new();
    run_with(&mut vim, "|a", ":nope<cr>");
    assert!(vim.status().contains("E492"), "{}", vim.status());
}

#[test]
fn editor_keys_pass_through() {
    let mut vim = Vim::new();
    let mut t = Text::new("abc");
    let acts = vim.feed(K::Named("ctrl+p".into()), &mut t, 0, 0, &Ctx::default());
    assert_eq!(
        acts,
        vec![Action::Pass],
        "Quick Open still works in Normal mode"
    );
    let acts = vim.feed(K::Char('Q'), &mut t, 0, 0, &Ctx::default());
    assert!(
        !acts.contains(&Action::Pass),
        "unknown letters never type in Normal mode"
    );
    assert_eq!(K::from_editor("ctrl+[", None), K::Named("escape".into()));
    assert_eq!(K::from_editor("shift+a", Some("A")), K::Char('A'));
}
