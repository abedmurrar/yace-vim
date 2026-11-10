//! Vim's modes and commands over a [`Text`], independent of YACE: keys go
//! in, [`Action`]s come out (edits, cursor moves, editor commands). The
//! engine edits its own copy of the text as it goes, so later positions in
//! the same batch are right.

use std::collections::HashMap;

use crate::{
    motion::{self, Kind},
    text::Text,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Replace,
    Visual,
    VisualLine,
}

/// A key: a char it types, or a named key (`escape`, `enter`, `ctrl+r`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum K {
    Char(char),
    Named(String),
}

impl K {
    /// From YACE's `{"key", "text"}`.
    pub fn from_editor(key: &str, text: Option<&str>) -> K {
        let plain = !key.contains("ctrl+") && !key.contains("alt+") && !key.contains("meta+");
        if plain && let Some(t) = text {
            let mut it = t.chars();
            if let (Some(c), None) = (it.next(), it.next()) {
                return K::Char(c);
            }
        }
        match key {
            "ctrl+[" | "ctrl+c" => K::Named("escape".into()),
            "ctrl+h" => K::Named("backspace".into()),
            "space" => K::Char(' '),
            k => K::Named(k.to_string()),
        }
    }

    fn is(&self, name: &str) -> bool {
        matches!(self, K::Named(n) if n == name)
    }
}

/// What the editor should do, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Replace chars `start..end` (against the text as it is at that point).
    Edit {
        start: usize,
        end: usize,
        text: String,
    },
    /// Put the cursor (and an empty selection) here.
    Cursor(usize),
    /// A selection from `anchor` to `head`.
    Select { anchor: usize, head: usize },
    /// Run an editor command (after the key).
    Command(String),
    /// Open a file (`:e path`).
    Open(String),
    /// Not a Vim key: let the editor handle it.
    Pass,
}

/// Editor facts the engine needs for some commands.
#[derive(Debug, Clone, Copy)]
pub struct Ctx {
    /// First and last line on screen (`H`, `M`, `L`, `Ctrl+D`).
    pub visible: (usize, usize),
    pub tab_size: usize,
    pub insert_spaces: bool,
    /// Completion, hover or parameter hints are open.
    pub popup: bool,
}

impl Default for Ctx {
    fn default() -> Self {
        Self {
            visible: (0, 40),
            tab_size: 4,
            insert_spaces: true,
            popup: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reg {
    pub text: String,
    pub linewise: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Delete,
    Change,
    Yank,
    Indent,
    Outdent,
    Lower,
    Upper,
    Toggle,
}

/// The parsed part of a Normal-mode command.
enum Parse {
    More,
    Bad,
    Done,
}

pub struct Vim {
    pub mode: Mode,
    pub cursor: usize,
    /// Visual mode's other end.
    anchor: usize,
    want_col: Option<usize>,
    /// Keys of the Normal-mode command being typed.
    pending: Vec<K>,
    /// `:` or `/` / `?` and what was typed after it.
    pub cmdline: Option<(char, String)>,
    regs: HashMap<char, Reg>,
    last_find: Option<(char, bool, bool)>,
    last_search: Option<(String, bool)>,
    /// The last change, as keys, for `.`.
    dot: Vec<K>,
    /// The change being recorded (until back in Normal mode).
    recording: Option<Vec<K>>,
    macro_rec: Option<(char, Vec<K>)>,
    macros: HashMap<char, Vec<K>>,
    last_macro: Option<char>,
    /// Replaying `.` or a macro: Insert-mode text is typed by the engine.
    replaying: bool,
    last_visual: Option<(usize, usize, Mode)>,
    /// A message for the status bar (`E492: Not an editor command`).
    pub message: Option<String>,
    /// The selection we last gave the editor (anchor, head): a different
    /// one means the user moved the cursor (mouse, arrow keys).
    shown: Option<(usize, usize)>,
}

impl Default for Vim {
    fn default() -> Self {
        Self::new()
    }
}

fn digit(k: &K) -> Option<usize> {
    match k {
        K::Char(c) if c.is_ascii_digit() => c.to_digit(10).map(|d| d as usize),
        _ => None,
    }
}

impl Vim {
    pub fn new() -> Self {
        Self {
            mode: Mode::Normal,
            cursor: 0,
            anchor: 0,
            want_col: None,
            pending: Vec::new(),
            cmdline: None,
            regs: HashMap::new(),
            last_find: None,
            last_search: None,
            dot: Vec::new(),
            recording: None,
            macro_rec: None,
            macros: HashMap::new(),
            last_macro: None,
            replaying: false,
            last_visual: None,
            message: None,
            shown: None,
        }
    }

    /// The status bar text: mode, then what's being typed.
    pub fn status(&self) -> String {
        if let Some((p, s)) = &self.cmdline {
            return format!("{p}{s}");
        }
        let mut out = match self.mode {
            Mode::Normal => "NORMAL",
            Mode::Insert => "-- INSERT --",
            Mode::Replace => "-- REPLACE --",
            Mode::Visual => "-- VISUAL --",
            Mode::VisualLine => "-- VISUAL LINE --",
        }
        .to_string();
        if !self.pending.is_empty() {
            out.push(' ');
            for k in &self.pending {
                match k {
                    K::Char(c) => out.push(*c),
                    K::Named(n) => out.push_str(&format!("<{n}>")),
                }
            }
        }
        if let Some((r, _)) = &self.macro_rec {
            out.push_str(&format!("  recording @{r}"));
        }
        if let Some(m) = &self.message {
            out.push_str("  ");
            out.push_str(m);
        }
        out
    }

    /// Whether `key` in Insert mode can be left to the editor without
    /// looking at the text (the fast path while typing).
    pub fn insert_passes(&mut self, k: &K) -> bool {
        if self.mode != Mode::Insert || self.replaying {
            return false;
        }
        let pass = match k {
            K::Char(_) => true,
            K::Named(n) => !matches!(n.as_str(), "escape" | "ctrl+w" | "ctrl+u"),
        };
        if pass && let Some((_, keys)) = &mut self.macro_rec {
            keys.push(k.clone());
        }
        if pass && let Some(rec) = &mut self.recording {
            match k {
                K::Char(_) => rec.push(k.clone()),
                K::Named(n) if matches!(n.as_str(), "backspace" | "enter" | "tab" | "delete") => {
                    rec.push(k.clone())
                }
                // Moving around in Insert mode starts a new change.
                _ => {}
            }
        }
        pass
    }

    /// Handle one key. `head` and `anchor` are the editor's selection now
    /// (the cursor may have moved by mouse or arrow keys).
    pub fn feed(
        &mut self,
        k: K,
        t: &mut Text,
        head: usize,
        anchor: usize,
        ctx: &Ctx,
    ) -> Vec<Action> {
        self.message = None;
        let mut out = Vec::new();
        if let Some((_, keys)) = &mut self.macro_rec
            && !(self.mode == Mode::Normal
                && self.pending.is_empty()
                && self.cmdline.is_none()
                && k == K::Char('q'))
        {
            keys.push(k.clone());
        }
        // Follow the editor's cursor when it moved without us.
        if self.shown != Some((anchor, head)) && (self.shown.is_some() || head != self.cursor) {
            self.cursor = head;
            self.want_col = None;
            if matches!(self.mode, Mode::Visual | Mode::VisualLine) {
                if head == anchor {
                    self.mode = Mode::Normal;
                } else {
                    // A selection made with the mouse: keep it as Vim's.
                    self.anchor = anchor;
                    self.cursor = if head > anchor { head - 1 } else { head };
                }
            }
        }
        if self.cmdline.is_some() {
            self.cmdline_key(k, t, ctx, &mut out);
        } else {
            match self.mode {
                Mode::Insert | Mode::Replace => self.insert_key(k, t, ctx, &mut out),
                _ => {
                    self.pending.push(k);
                    match self.normal(t, ctx, &mut out) {
                        Parse::More => {}
                        Parse::Bad | Parse::Done => self.pending.clear(),
                    }
                }
            }
        }
        if let Some(last) = out.iter().rev().find_map(|a| match a {
            Action::Cursor(p) => Some((*p, *p)),
            Action::Select { anchor, head } => Some((*anchor, *head)),
            _ => None,
        }) {
            self.shown = Some(last);
        } else if out
            .iter()
            .any(|a| matches!(a, Action::Edit { .. } | Action::Pass))
        {
            self.shown = None;
        }
        out
    }

    // ------------------------------------------------------------------
    // Insert and Replace modes

    fn insert_key(&mut self, k: K, t: &mut Text, ctx: &Ctx, out: &mut Vec<Action>) {
        if k.is("escape") {
            if ctx.popup {
                for c in [
                    "hideSuggestWidget",
                    "editor.action.hideHover",
                    "closeParameterHints",
                ] {
                    out.push(Action::Command(c.into()));
                }
            }
            if let Some(mut rec) = self.recording.take() {
                rec.push(K::Named("escape".into()));
                if !self.replaying {
                    self.dot = rec;
                }
            }
            self.mode = Mode::Normal;
            let line = t.line_of(self.cursor);
            if self.cursor > t.line_start(line) {
                self.cursor -= 1;
            }
            self.cursor = t.clamp_normal(self.cursor);
            out.push(Action::Cursor(self.cursor));
            return;
        }
        if let Some(rec) = &mut self.recording {
            rec.push(k.clone());
        }
        let replace = self.mode == Mode::Replace;
        if !self.replaying
            && !replace
            && !matches!(&k, K::Named(n) if n == "ctrl+w" || n == "ctrl+u")
        {
            out.push(Action::Pass);
            return;
        }
        // Typed by the engine: replays, Replace mode, Ctrl+W / Ctrl+U.
        let c = self.cursor;
        match &k {
            K::Char(ch) => {
                let end = if replace && c < t.line_end(t.line_of(c)) {
                    c + 1
                } else {
                    c
                };
                self.edit(t, c, end, &ch.to_string(), out);
                self.cursor = c + 1;
            }
            K::Named(n) if n == "enter" => {
                let indent = t.indent(t.line_of(c));
                self.edit(t, c, c, &format!("\n{indent}"), out);
                self.cursor = c + 1 + indent.chars().count();
            }
            K::Named(n) if n == "tab" => {
                let s = if ctx.insert_spaces {
                    " ".repeat(ctx.tab_size)
                } else {
                    "\t".into()
                };
                self.edit(t, c, c, &s, out);
                self.cursor = c + s.chars().count();
            }
            K::Named(n) if n == "backspace" => {
                if c > 0 {
                    if !replace {
                        self.edit(t, c - 1, c, "", out);
                    }
                    self.cursor = c - 1;
                }
            }
            K::Named(n) if n == "delete" => {
                if c < t.len() {
                    self.edit(t, c, c + 1, "", out);
                }
            }
            K::Named(n) if n == "ctrl+w" => {
                let a = motion::word_back(t, c, false).max(t.line_start(t.line_of(c)));
                self.edit(t, a, c, "", out);
                self.cursor = a;
            }
            K::Named(n) if n == "ctrl+u" => {
                let a = t.first_non_blank(t.line_of(c)).min(c);
                let a = if a == c {
                    t.line_start(t.line_of(c))
                } else {
                    a
                };
                self.edit(t, a, c, "", out);
                self.cursor = a;
            }
            _ => {
                out.push(Action::Pass);
                return;
            }
        }
        out.push(Action::Cursor(self.cursor));
    }

    // ------------------------------------------------------------------
    // Command line (`:`, `/`, `?`)

    fn cmdline_key(&mut self, k: K, t: &mut Text, ctx: &Ctx, out: &mut Vec<Action>) {
        let Some((p, mut s)) = self.cmdline.take() else {
            return;
        };
        match k {
            K::Char(c) => {
                s.push(c);
                self.cmdline = Some((p, s));
            }
            K::Named(n) if n == "backspace" => {
                if s.pop().is_some() {
                    self.cmdline = Some((p, s));
                }
            }
            K::Named(n) if n == "enter" => {
                if p == ':' {
                    self.ex(&s, t, ctx, out);
                } else if !s.is_empty() || self.last_search.is_some() {
                    let pat = if s.is_empty() {
                        self.last_search.clone().map(|x| x.0).unwrap_or_default()
                    } else {
                        s
                    };
                    self.last_search = Some((pat, p == '/'));
                    self.search_next(t, false, out);
                }
                if matches!(self.mode, Mode::Visual | Mode::VisualLine) {
                    self.show_visual(t, out);
                }
            }
            K::Named(n) if n == "escape" => {}
            _ => self.cmdline = Some((p, s)),
        }
    }

    fn search_next(&mut self, t: &Text, reverse: bool, out: &mut Vec<Action>) {
        let Some((pat, fwd)) = self.last_search.clone() else {
            self.message = Some("E35: No previous regular expression".into());
            return;
        };
        match motion::search(t, self.cursor, &pat, fwd != reverse) {
            Some(p) => {
                self.cursor = p;
                if self.mode == Mode::Normal {
                    out.push(Action::Cursor(p));
                }
            }
            None => self.message = Some(format!("E486: Pattern not found: {pat}")),
        }
    }

    fn ex(&mut self, cmd: &str, t: &mut Text, ctx: &Ctx, out: &mut Vec<Action>) {
        let cmd = cmd.trim();
        let c = |s: &str| Action::Command(s.into());
        match cmd {
            "" => {}
            "w" => out.push(c("workbench.action.files.save")),
            "wa" | "wall" => out.push(c("workbench.action.files.saveAll")),
            "q" | "quit" | "clo" | "close" => out.push(c("workbench.action.closeActiveEditor")),
            "q!" | "quit!" => {
                out.push(c("workbench.action.files.revert"));
                out.push(c("workbench.action.closeActiveEditor"));
            }
            "wq" | "x" | "xit" => {
                out.push(c("workbench.action.files.save"));
                out.push(c("workbench.action.closeActiveEditor"));
            }
            "qa" | "qall" | "qa!" | "qall!" => out.push(c("workbench.action.quit")),
            "wqa" | "xa" | "wqall" | "xall" => {
                out.push(c("workbench.action.files.saveAll"));
                out.push(c("workbench.action.quit"));
            }
            "e!" | "edit!" => out.push(c("workbench.action.files.revert")),
            "sp" | "split" | "vs" | "vsplit" => out.push(c("workbench.action.splitEditor")),
            "bn" | "bnext" => out.push(c("workbench.action.nextEditor")),
            "bp" | "bprevious" | "bN" => out.push(c("workbench.action.previousEditor")),
            "noh" | "nohlsearch" => {}
            _ => {
                if let Ok(n) = cmd.parse::<usize>() {
                    let line = n.saturating_sub(1).min(t.lines() - 1);
                    self.cursor = t.first_non_blank(line);
                    out.push(Action::Cursor(self.cursor));
                } else if let Some(path) =
                    cmd.strip_prefix("e ").or_else(|| cmd.strip_prefix("edit "))
                {
                    out.push(Action::Open(path.trim().to_string()));
                } else if let Some(rest) = cmd.strip_prefix("%s").or_else(|| cmd.strip_prefix("s"))
                {
                    self.substitute(rest, cmd.starts_with('%'), t, ctx, out);
                } else {
                    self.message = Some(format!("E492: Not an editor command: {cmd}"));
                }
            }
        }
    }

    /// `:s/old/new/[g]` and `:%s/…`: literal text, not a regular expression.
    fn substitute(
        &mut self,
        rest: &str,
        all_lines: bool,
        t: &mut Text,
        _ctx: &Ctx,
        out: &mut Vec<Action>,
    ) {
        let mut it = rest.chars();
        let Some(sep) = it
            .next()
            .filter(|c| !c.is_alphanumeric() && !c.is_whitespace())
        else {
            self.message = Some("E492: Not an editor command".into());
            return;
        };
        let parts: Vec<&str> = it.as_str().split(sep).collect();
        let (old, new) = match parts.as_slice() {
            [o, n, ..] => (o.to_string(), n.to_string()),
            [o] => (o.to_string(), String::new()),
            _ => return,
        };
        let global = parts.get(2).is_some_and(|f| f.contains('g'));
        if old.is_empty() {
            return;
        }
        let lines: Vec<usize> = if all_lines {
            (0..t.lines()).collect()
        } else {
            vec![t.line_of(self.cursor)]
        };
        let mut count = 0;
        let mut last_line = None;
        // Last line first, so earlier offsets stay valid.
        for &l in lines.iter().rev() {
            let line = t.slice(t.line_start(l), t.line_end(l));
            let replaced = if global {
                line.replace(&old, &new)
            } else {
                line.replacen(&old, &new, 1)
            };
            if replaced != line {
                count += 1;
                last_line.get_or_insert(l);
                let (s, e) = (t.line_start(l), t.line_end(l));
                self.edit(t, s, e, &replaced, out);
            }
        }
        if count == 0 {
            self.message = Some(format!("E486: Pattern not found: {old}"));
        } else if let Some(l) = last_line {
            self.cursor = t.first_non_blank(l);
            out.push(Action::Cursor(self.cursor));
        }
    }

    // ------------------------------------------------------------------
    // Normal and Visual modes

    fn edit(&mut self, t: &mut Text, start: usize, end: usize, text: &str, out: &mut Vec<Action>) {
        let mut chars = std::mem::take(&mut t.chars);
        let (start, end) = (start.min(chars.len()), end.min(chars.len()));
        chars.splice(start..end, text.chars());
        *t = Text::new(&chars.iter().collect::<String>());
        out.push(Action::Edit {
            start,
            end,
            text: text.to_string(),
        });
    }

    fn set_reg(&mut self, name: Option<char>, text: String, linewise: bool, yank: bool) {
        let reg = Reg { text, linewise };
        match name {
            Some('_') => return,
            Some(c) if c.is_ascii_uppercase() => {
                let e = self.regs.entry(c.to_ascii_lowercase()).or_default();
                e.text.push_str(&reg.text);
                e.linewise |= reg.linewise;
                let r = e.clone();
                self.regs.insert('"', r);
                return;
            }
            Some(c) if c != '"' => {
                self.regs.insert(c, reg.clone());
            }
            _ => {}
        }
        if yank {
            self.regs.insert('0', reg.clone());
        }
        self.regs.insert('"', reg);
    }

    fn get_reg(&self, name: Option<char>) -> Option<Reg> {
        self.regs
            .get(&name.unwrap_or('"').to_ascii_lowercase())
            .cloned()
    }

    fn show_visual(&mut self, t: &Text, out: &mut Vec<Action>) {
        let (a, c) = (self.anchor, self.cursor);
        match self.mode {
            Mode::VisualLine => {
                let (la, lc) = (t.line_of(a), t.line_of(c));
                if lc >= la {
                    out.push(Action::Select {
                        anchor: t.line_start(la),
                        head: t.line_end(lc),
                    });
                } else {
                    out.push(Action::Select {
                        anchor: t.line_end(la),
                        head: t.line_start(lc),
                    });
                }
            }
            _ => {
                // Vim's selection includes the char under the cursor.
                if c >= a {
                    out.push(Action::Select {
                        anchor: a,
                        head: (c + 1).min(t.len()),
                    });
                } else {
                    out.push(Action::Select {
                        anchor: (a + 1).min(t.len()),
                        head: c,
                    });
                }
            }
        }
    }

    fn visual_range(&self, t: &Text) -> (usize, usize, bool) {
        let (a, b) = (self.anchor.min(self.cursor), self.anchor.max(self.cursor));
        if self.mode == Mode::VisualLine {
            (t.line_start(t.line_of(a)), t.line_after(t.line_of(b)), true)
        } else {
            (a, (b + 1).min(t.len()), false)
        }
    }

    fn enter_insert(&mut self, at: usize, out: &mut Vec<Action>) {
        self.mode = Mode::Insert;
        self.cursor = at;
        out.push(Action::Cursor(at));
    }

    /// Parse and run what's in `pending`.
    fn normal(&mut self, t: &mut Text, ctx: &Ctx, out: &mut Vec<Action>) -> Parse {
        let keys = self.pending.clone();
        let mut i = 0;
        // "x register
        let mut reg = None;
        if keys.first() == Some(&K::Char('"')) {
            match keys.get(1) {
                None => return Parse::More,
                Some(K::Char(c)) => {
                    reg = Some(*c);
                    i = 2;
                }
                _ => return Parse::Bad,
            }
        }
        let (count, j) = parse_count(&keys, i);
        i = j;
        let Some(k) = keys.get(i).cloned() else {
            return Parse::More;
        };
        let visual = matches!(self.mode, Mode::Visual | Mode::VisualLine);
        if self.mode == Mode::Normal {
            self.cursor = t.clamp_normal(self.cursor);
        }
        let n = count.unwrap_or(1);

        // Operators: d c y > < g~ gu gU (and their doubled forms).
        let op = match &k {
            K::Char('d') => Some(Op::Delete),
            K::Char('c') => Some(Op::Change),
            K::Char('y') => Some(Op::Yank),
            K::Char('>') => Some(Op::Indent),
            K::Char('<') => Some(Op::Outdent),
            K::Char('g') => match keys.get(i + 1) {
                None => return Parse::More,
                Some(K::Char('~')) => Some(Op::Toggle),
                Some(K::Char('u')) => Some(Op::Lower),
                Some(K::Char('U')) => Some(Op::Upper),
                _ => None,
            },
            _ => None,
        };
        if let Some(op) = op {
            let oplen = if matches!(k, K::Char('g')) { 2 } else { 1 };
            if visual {
                let (a, b, lw) = self.visual_range(t);
                self.last_visual = Some((self.anchor, self.cursor, self.mode));
                self.mode = Mode::Normal;
                self.begin_change(&keys);
                self.operate(op, a, b, lw, reg, t, ctx, out);
                self.end_change_unless_insert();
                return Parse::Done;
            }
            let rest = &keys[i + oplen..];
            let (count2, k2) = parse_count(rest, 0);
            let total = match (count, count2) {
                (None, None) => None,
                (a, b) => Some(a.unwrap_or(1) * b.unwrap_or(1)),
            };
            let n = total.unwrap_or(1);
            let Some(mk) = rest.get(k2) else {
                return Parse::More;
            };
            // dd, cc, yy, >>, <<, g~~, guu, gUU
            let doubled = match (op, mk) {
                (Op::Toggle, K::Char('~'))
                | (Op::Lower, K::Char('u'))
                | (Op::Upper, K::Char('U')) => true,
                (_, K::Char(c)) => oplen == 1 && K::Char(*c) == k,
                _ => false,
            };
            if doubled {
                let l1 = t.line_of(self.cursor);
                let l2 = (l1 + n - 1).min(t.lines() - 1);
                self.begin_change(&keys);
                self.operate(
                    op,
                    t.line_start(l1),
                    t.line_after(l2),
                    true,
                    reg,
                    t,
                    ctx,
                    out,
                );
                self.end_change_unless_insert();
                return Parse::Done;
            }
            // Text objects.
            if let K::Char(ia @ ('i' | 'a')) = mk {
                let Some(K::Char(obj)) = rest.get(k2 + 1) else {
                    return if rest.get(k2 + 1).is_none() {
                        Parse::More
                    } else {
                        Parse::Bad
                    };
                };
                let Some((a, b, lw)) = text_object(t, self.cursor, *ia == 'a', *obj) else {
                    return Parse::Bad;
                };
                self.begin_change(&keys);
                self.operate(op, a, b, lw, reg, t, ctx, out);
                self.end_change_unless_insert();
                return Parse::Done;
            }
            let motion_keys = &rest[k2..];
            let mut m = match self.motion(motion_keys, t, total, ctx, true) {
                MotionResult::More => return Parse::More,
                MotionResult::Bad => return Parse::Bad,
                MotionResult::Done(p, kind) => (p, kind),
            };
            // `cw` on a word is `ce`.
            if op == Op::Change
                && matches!(motion_keys.first(), Some(K::Char('w' | 'W')))
                && t.char(self.cursor).is_some_and(|c| !c.is_whitespace())
            {
                let big = motion_keys.first() == Some(&K::Char('W'));
                let mut e = self.cursor;
                for _ in 0..n {
                    e = motion::word_end(
                        t,
                        if e == self.cursor {
                            e.saturating_sub(1).max(self.cursor)
                        } else {
                            e
                        },
                        big,
                    );
                }
                // On the last char of a word, `cw` changes just that char.
                let at_end = t.char(self.cursor + 1).is_none_or(|c| {
                    crate::text::class(c, big) != crate::text::class(t.chars[self.cursor], big)
                });
                if at_end && n == 1 {
                    e = self.cursor;
                }
                m = (e, Kind::Inclusive);
            }
            let (target, kind) = m;
            let (mut a, mut b) = if target >= self.cursor {
                (self.cursor, target)
            } else {
                (target, self.cursor)
            };
            let lw = kind == Kind::Linewise;
            if lw {
                a = t.line_start(t.line_of(a));
                b = t.line_after(t.line_of(b));
            } else if kind == Kind::Inclusive {
                b = (b + 1).min(t.len());
            } else if t.line_of(b) > t.line_of(a) && b == t.line_start(t.line_of(b)) && b > a {
                // An exclusive motion ending at a line start stops at the
                // end of the line before (`dw` on the last word).
                b = t.line_end(t.line_of(b) - 1).max(a);
            }
            self.begin_change(&keys);
            self.operate(op, a, b, lw, reg, t, ctx, out);
            self.end_change_unless_insert();
            return Parse::Done;
        }

        // Text objects in Visual mode.
        if visual && let K::Char(ia @ ('i' | 'a')) = k {
            let Some(K::Char(obj)) = keys.get(i + 1) else {
                return if keys.get(i + 1).is_none() {
                    Parse::More
                } else {
                    Parse::Bad
                };
            };
            if let Some((a, b, lw)) = text_object(t, self.cursor, ia == 'a', *obj) {
                if lw {
                    self.mode = Mode::VisualLine;
                }
                self.anchor = a;
                self.cursor = b.saturating_sub(1).max(a);
                self.show_visual(t, out);
            }
            return Parse::Done;
        }

        // Motions.
        match self.motion(&keys[i..], t, count, ctx, false) {
            MotionResult::More => return Parse::More,
            MotionResult::Done(p, _) => {
                self.cursor = if visual {
                    p.min(t.len().saturating_sub(1))
                } else {
                    t.clamp_normal(p)
                };
                if visual {
                    self.show_visual(t, out);
                } else {
                    out.push(Action::Cursor(self.cursor));
                }
                return Parse::Done;
            }
            MotionResult::Bad => {}
        }

        // Everything else.
        let c = self.cursor;
        let line = t.line_of(c);
        let next = keys.get(i + 1).cloned();
        if visual {
            return self.visual_command(&k, next, reg, n, t, ctx, out, &keys);
        }
        match k {
            K::Char('i') => {
                self.begin_change(&keys);
                self.enter_insert(c, out);
            }
            K::Char('a') => {
                self.begin_change(&keys);
                let at = if c < t.line_end(line) { c + 1 } else { c };
                self.enter_insert(at, out);
            }
            K::Char('I') => {
                self.begin_change(&keys);
                self.enter_insert(t.first_non_blank(line), out);
            }
            K::Char('A') => {
                self.begin_change(&keys);
                self.enter_insert(t.line_end(line), out);
            }
            K::Char('o') | K::Char('O') => {
                self.begin_change(&keys);
                let indent = t.indent(line);
                if k == K::Char('o') {
                    let e = t.line_end(line);
                    self.edit(t, e, e, &format!("\n{indent}"), out);
                    self.enter_insert(e + 1 + indent.chars().count(), out);
                } else {
                    let s = t.line_start(line);
                    self.edit(t, s, s, &format!("{indent}\n"), out);
                    self.enter_insert(s + indent.chars().count(), out);
                }
            }
            K::Char('v') => {
                self.mode = Mode::Visual;
                self.anchor = c;
                self.show_visual(t, out);
            }
            K::Char('V') => {
                self.mode = Mode::VisualLine;
                self.anchor = c;
                self.show_visual(t, out);
            }
            K::Char('R') => {
                self.begin_change(&keys);
                self.mode = Mode::Replace;
            }
            K::Char('x')
            | K::Char('X')
            | K::Char('s')
            | K::Char('D')
            | K::Char('C')
            | K::Char('S') => {
                self.begin_change(&keys);
                let (a, b, lw) = match k {
                    K::Char('x') | K::Char('s') => (c, (c + n).min(t.line_end(line)), false),
                    K::Char('X') => (c.saturating_sub(n).max(t.line_start(line)), c, false),
                    K::Char('S') => {
                        let l2 = (line + n - 1).min(t.lines() - 1);
                        (t.line_start(line), t.line_after(l2), true)
                    }
                    _ => {
                        let l2 = (line + n - 1).min(t.lines() - 1);
                        (c, t.line_end(l2), false)
                    }
                };
                let op = if matches!(k, K::Char('s') | K::Char('C') | K::Char('S')) {
                    Op::Change
                } else {
                    Op::Delete
                };
                if a < b || op == Op::Change {
                    self.operate(op, a, b, lw, reg, t, ctx, out);
                }
                self.end_change_unless_insert();
            }
            K::Char('Y') => {
                let l2 = (line + n - 1).min(t.lines() - 1);
                self.operate(
                    Op::Yank,
                    t.line_start(line),
                    t.line_after(l2),
                    true,
                    reg,
                    t,
                    ctx,
                    out,
                );
            }
            K::Char('p') | K::Char('P') => {
                self.begin_change(&keys);
                self.put(reg, n, k == K::Char('p'), t, out);
                self.end_change_unless_insert();
            }
            K::Char('r') => {
                let Some(r) = next else { return Parse::More };
                let ch = match r {
                    K::Char(ch) => ch,
                    K::Named(ref nm) if nm == "enter" => '\n',
                    _ => return Parse::Bad,
                };
                let e = t.line_end(line);
                if c + n <= e {
                    self.begin_change(&keys);
                    let s = if ch == '\n' {
                        "\n".to_string()
                    } else {
                        ch.to_string().repeat(n)
                    };
                    self.edit(t, c, c + n, &s, out);
                    self.cursor = if ch == '\n' { c + 1 } else { c + n - 1 };
                    out.push(Action::Cursor(self.cursor));
                    self.end_change_unless_insert();
                }
            }
            K::Char('~') => {
                self.begin_change(&keys);
                let e = (c + n).min(t.line_end(line));
                let s: String = t.chars[c..e].iter().map(|&ch| toggle_case(ch)).collect();
                self.edit(t, c, e, &s, out);
                self.cursor = t.clamp_normal(e);
                out.push(Action::Cursor(self.cursor));
                self.end_change_unless_insert();
            }
            K::Char('J') => {
                self.begin_change(&keys);
                self.join(line, n.max(2) - 1, t, out);
                self.end_change_unless_insert();
            }
            K::Char('u') => {
                for _ in 0..n {
                    out.push(Action::Command("undo".into()));
                }
            }
            K::Named(ref nm) if nm == "ctrl+r" => {
                for _ in 0..n {
                    out.push(Action::Command("redo".into()));
                }
            }
            K::Char('.') => {
                let mut keys = self.dot.clone();
                if let Some(cnt) = count {
                    // A count replaces the change's own count.
                    let start = if keys.first() == Some(&K::Char('"')) {
                        2
                    } else {
                        0
                    };
                    let (_, end) = parse_count(&keys, start);
                    let digits: Vec<K> = cnt.to_string().chars().map(K::Char).collect();
                    keys.splice(start..end, digits);
                }
                self.replay(&keys, t, ctx, out);
            }
            K::Char(':') | K::Char('/') | K::Char('?') => {
                let K::Char(p) = k else { unreachable!() };
                self.cmdline = Some((p, String::new()));
            }
            K::Char('q') => {
                if let Some((r, keys)) = self.macro_rec.take() {
                    self.macros.insert(r, keys);
                    return Parse::Done;
                }
                let Some(K::Char(r)) = next else {
                    return if next.is_none() {
                        Parse::More
                    } else {
                        Parse::Bad
                    };
                };
                if r.is_ascii_alphanumeric() {
                    self.macro_rec = Some((r.to_ascii_lowercase(), Vec::new()));
                }
            }
            K::Char('@') => {
                let Some(K::Char(r)) = next else {
                    return if next.is_none() {
                        Parse::More
                    } else {
                        Parse::Bad
                    };
                };
                let r = if r == '@' {
                    self.last_macro.unwrap_or('@')
                } else {
                    r.to_ascii_lowercase()
                };
                if let Some(keys) = self.macros.get(&r).cloned() {
                    self.last_macro = Some(r);
                    for _ in 0..n {
                        self.replay(&keys, t, ctx, out);
                    }
                }
            }
            K::Char('g') => match next {
                None => return Parse::More,
                Some(K::Char('v')) => {
                    if let Some((a, c, m)) = self.last_visual {
                        self.anchor = a.min(t.len().saturating_sub(1));
                        self.cursor = c.min(t.len().saturating_sub(1));
                        self.mode = m;
                        self.show_visual(t, out);
                    }
                }
                Some(K::Char('J')) => {
                    self.begin_change(&keys);
                    self.join_raw(line, n.max(2) - 1, t, out);
                    self.end_change_unless_insert();
                }
                Some(K::Char('i')) => {
                    self.begin_change(&keys);
                    self.enter_insert(c, out);
                }
                _ => return Parse::Bad,
            },
            K::Char('z') => {
                // zz, zt, zb: scrolling the view isn't available to plugins yet.
                if next.is_none() {
                    return Parse::More;
                }
            }
            K::Char(ch) if ch.is_ascii_digit() => return Parse::More,
            K::Char(_) => return Parse::Bad,
            // Keys Vim doesn't use here: the editor's (Ctrl+P, F1, …).
            K::Named(_) => out.push(Action::Pass),
        }
        Parse::Done
    }

    #[allow(clippy::too_many_arguments)]
    fn visual_command(
        &mut self,
        k: &K,
        next: Option<K>,
        reg: Option<char>,
        n: usize,
        t: &mut Text,
        ctx: &Ctx,
        out: &mut Vec<Action>,
        keys: &[K],
    ) -> Parse {
        let (a, b, lw) = self.visual_range(t);
        let leave = |me: &mut Self| {
            me.last_visual = Some((me.anchor, me.cursor, me.mode));
            me.mode = Mode::Normal;
        };
        match k {
            K::Named(nm) if nm == "escape" => {
                leave(self);
                self.cursor = t.clamp_normal(self.cursor);
                out.push(Action::Cursor(self.cursor));
            }
            K::Char('v') | K::Char('V') => {
                let want = if *k == K::Char('v') {
                    Mode::Visual
                } else {
                    Mode::VisualLine
                };
                if self.mode == want {
                    leave(self);
                    self.cursor = t.clamp_normal(self.cursor);
                    out.push(Action::Cursor(self.cursor));
                } else {
                    self.mode = want;
                    self.show_visual(t, out);
                }
            }
            K::Char('o') => {
                std::mem::swap(&mut self.anchor, &mut self.cursor);
                self.show_visual(t, out);
            }
            K::Char('x')
            | K::Char('X')
            | K::Char('D')
            | K::Char('s')
            | K::Char('S')
            | K::Char('C')
            | K::Char('R') => {
                leave(self);
                self.begin_change(keys);
                let full = matches!(
                    k,
                    K::Char('X') | K::Char('D') | K::Char('S') | K::Char('C') | K::Char('R')
                );
                let (a, b, lw) = if full {
                    (
                        t.line_start(t.line_of(a)),
                        t.line_after(t.line_of(b.saturating_sub(1).max(a))),
                        true,
                    )
                } else {
                    (a, b, lw)
                };
                let op = if matches!(k, K::Char('x') | K::Char('X') | K::Char('D')) {
                    Op::Delete
                } else {
                    Op::Change
                };
                self.operate(op, a, b, lw, reg, t, ctx, out);
                self.end_change_unless_insert();
            }
            K::Char('~') | K::Char('u') | K::Char('U') => {
                leave(self);
                self.begin_change(keys);
                let op = match k {
                    K::Char('~') => Op::Toggle,
                    K::Char('u') => Op::Lower,
                    _ => Op::Upper,
                };
                self.operate(op, a, b, lw, reg, t, ctx, out);
                self.end_change_unless_insert();
            }
            K::Char('J') => {
                leave(self);
                self.begin_change(keys);
                let (l1, l2) = (t.line_of(a), t.line_of(b.saturating_sub(1).max(a)));
                self.join(l1, (l2 - l1).max(1), t, out);
                self.end_change_unless_insert();
            }
            K::Char('p') | K::Char('P') => {
                leave(self);
                self.begin_change(keys);
                let Some(r) = self.get_reg(reg) else {
                    return Parse::Done;
                };
                let removed = t.slice(a, b);
                self.edit(t, a, b, "", out);
                self.cursor = a.min(t.len());
                if lw && !r.linewise {
                    self.edit(t, a, a, "\n", out);
                }
                let at = self.cursor;
                let text = if r.linewise && !lw {
                    format!("\n{}", r.text.trim_end_matches('\n'))
                } else {
                    r.text.clone()
                };
                self.edit(t, at, at, &text.repeat(n), out);
                self.cursor = t.clamp_normal(if r.linewise {
                    at + usize::from(!lw)
                } else {
                    at + text.chars().count() * n - 1
                });
                out.push(Action::Cursor(self.cursor));
                if k == &K::Char('p') {
                    self.set_reg(None, removed, lw, false);
                }
                self.end_change_unless_insert();
            }
            K::Char('I') | K::Char('A') => {
                leave(self);
                self.begin_change(keys);
                let at = if *k == K::Char('I') { a } else { b };
                self.enter_insert(at.min(t.len()), out);
            }
            K::Char(':') => self.cmdline = Some((':', String::new())),
            K::Char('g') if next.is_none() => return Parse::More,
            K::Char(ch) if ch.is_ascii_digit() => return Parse::More,
            K::Char(_) => return Parse::Bad,
            K::Named(_) => out.push(Action::Pass),
        }
        Parse::Done
    }

    /// Run `keys` as if typed (`.` and macros); Insert-mode text is typed by
    /// the engine itself.
    fn replay(&mut self, keys: &[K], t: &mut Text, ctx: &Ctx, out: &mut Vec<Action>) {
        if self.replaying {
            return;
        }
        let saved = std::mem::take(&mut self.dot);
        self.replaying = true;
        self.pending.clear();
        for k in keys {
            let cur = self.cursor;
            let acts = self.feed(
                k.clone(),
                t,
                cur,
                if matches!(self.mode, Mode::Visual | Mode::VisualLine) {
                    self.anchor
                } else {
                    cur
                },
                ctx,
            );
            out.extend(acts.into_iter().filter(|a| *a != Action::Pass));
        }
        self.replaying = false;
        // Replaying doesn't change what `.` repeats.
        self.dot = saved;
        self.recording = None;
    }

    fn begin_change(&mut self, keys: &[K]) {
        if !self.replaying {
            self.recording = Some(keys.to_vec());
        }
    }

    fn end_change_unless_insert(&mut self) {
        if matches!(self.mode, Mode::Insert | Mode::Replace) {
            return;
        }
        if let Some(rec) = self.recording.take()
            && !self.replaying
        {
            self.dot = rec;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn operate(
        &mut self,
        op: Op,
        a: usize,
        b: usize,
        linewise: bool,
        reg: Option<char>,
        t: &mut Text,
        ctx: &Ctx,
        out: &mut Vec<Action>,
    ) {
        let (a, b) = (a.min(t.len()), b.min(t.len()));
        match op {
            Op::Yank => {
                let mut text = t.slice(a, b);
                if linewise && !text.ends_with('\n') {
                    text.push('\n');
                }
                self.set_reg(reg, text, linewise, true);
                let l = t.line_of(a);
                self.cursor = if linewise {
                    if t.line_of(self.cursor) == l {
                        self.cursor
                    } else {
                        t.first_non_blank(l)
                    }
                } else {
                    a
                };
                self.cursor = t.clamp_normal(self.cursor);
                out.push(Action::Cursor(self.cursor));
            }
            Op::Delete => {
                let mut text = t.slice(a, b);
                let mut a2 = a;
                if linewise && !text.ends_with('\n') {
                    text.push('\n');
                    // The last line: take the newline before it.
                    if a > 0 {
                        a2 = a - 1;
                    }
                }
                self.set_reg(reg, text, linewise, false);
                self.edit(t, a2, b, "", out);
                self.cursor = if linewise {
                    t.first_non_blank(t.line_of(a2.min(t.len())))
                } else {
                    t.clamp_normal(a)
                };
                out.push(Action::Cursor(self.cursor));
            }
            Op::Change => {
                if linewise {
                    // Keep the first line's indentation, and the last newline.
                    let indent = t.indent(t.line_of(a));
                    let end = if b > a && t.char(b - 1) == Some('\n') {
                        b - 1
                    } else {
                        b
                    };
                    let mut text = t.slice(a, b);
                    if !text.ends_with('\n') {
                        text.push('\n');
                    }
                    self.set_reg(reg, text, true, false);
                    self.edit(t, a, end, &indent, out);
                    self.enter_insert(a + indent.chars().count(), out);
                } else {
                    self.set_reg(reg, t.slice(a, b), false, false);
                    self.edit(t, a, b, "", out);
                    self.enter_insert(a, out);
                }
            }
            Op::Indent | Op::Outdent => {
                let (l1, l2) = (t.line_of(a), t.line_of(b.saturating_sub(1).max(a)));
                let unit = if ctx.insert_spaces {
                    " ".repeat(ctx.tab_size)
                } else {
                    "\t".into()
                };
                for l in (l1..=l2).rev() {
                    let s = t.line_start(l);
                    if op == Op::Indent {
                        if t.line_end(l) > s {
                            self.edit(t, s, s, &unit, out);
                        }
                    } else {
                        let fnb = t.first_non_blank(l);
                        let mut cut = 0;
                        let mut width = 0;
                        for &ch in &t.chars[s..fnb] {
                            if width >= ctx.tab_size {
                                break;
                            }
                            width += if ch == '\t' { ctx.tab_size } else { 1 };
                            cut += 1;
                        }
                        if cut > 0 {
                            self.edit(t, s, s + cut, "", out);
                        }
                    }
                }
                self.cursor = t.first_non_blank(l1);
                out.push(Action::Cursor(self.cursor));
            }
            Op::Lower | Op::Upper | Op::Toggle => {
                let s: String = t.chars[a..b]
                    .iter()
                    .map(|&ch| match op {
                        Op::Lower => ch.to_lowercase().next().unwrap_or(ch),
                        Op::Upper => ch.to_uppercase().next().unwrap_or(ch),
                        _ => toggle_case(ch),
                    })
                    .collect();
                self.edit(t, a, b, &s, out);
                self.cursor = t.clamp_normal(a);
                out.push(Action::Cursor(self.cursor));
            }
        }
    }

    fn put(
        &mut self,
        reg: Option<char>,
        n: usize,
        after: bool,
        t: &mut Text,
        out: &mut Vec<Action>,
    ) {
        let Some(r) = self.get_reg(reg) else { return };
        let c = self.cursor;
        let line = t.line_of(c);
        if r.linewise {
            let mut text = r.text.repeat(n);
            let at = if after {
                let at = t.line_after(line);
                if at == t.len() && !t.chars.last().is_some_and(|&ch| ch == '\n') {
                    // After the last line, which has no newline.
                    text = format!("\n{}", text.trim_end_matches('\n'));
                }
                at
            } else {
                t.line_start(line)
            };
            self.edit(t, at, at, &text, out);
            let first = if text.starts_with('\n') { at + 1 } else { at };
            self.cursor = t.first_non_blank(t.line_of(first));
        } else {
            let at = if after && c < t.line_end(line) {
                c + 1
            } else {
                c
            };
            let text = r.text.repeat(n);
            self.edit(t, at, at, &text, out);
            self.cursor = (at + text.chars().count()).saturating_sub(1).max(at);
            self.cursor = t.clamp_normal(self.cursor);
        }
        out.push(Action::Cursor(self.cursor));
    }

    /// `J`: join `times` lines after `line`, with one space between.
    fn join(&mut self, line: usize, times: usize, t: &mut Text, out: &mut Vec<Action>) {
        for _ in 0..times {
            if line + 1 >= t.lines() {
                break;
            }
            let e = t.line_end(line);
            let next = line + 1;
            let fnb = t.first_non_blank(next);
            let next_empty = fnb == t.line_end(next);
            let sep = if e == t.line_start(line) || next_empty || t.char(fnb) == Some(')') {
                ""
            } else {
                " "
            };
            // Trailing blanks of this line go too.
            let mut s = e;
            while s > t.line_start(line) && matches!(t.chars[s - 1], ' ' | '\t') {
                s -= 1;
            }
            self.edit(t, s, fnb, sep, out);
            self.cursor = s;
        }
        out.push(Action::Cursor(self.cursor));
    }

    /// `gJ`: join without adding or removing spaces.
    fn join_raw(&mut self, line: usize, times: usize, t: &mut Text, out: &mut Vec<Action>) {
        for _ in 0..times {
            if line + 1 >= t.lines() {
                break;
            }
            let e = t.line_end(line);
            self.edit(t, e, t.line_start(line + 1), "", out);
            self.cursor = e;
        }
        out.push(Action::Cursor(self.cursor));
    }

    /// A motion from the keys at the start of `keys`.
    fn motion(
        &mut self,
        keys: &[K],
        t: &Text,
        count: Option<usize>,
        ctx: &Ctx,
        for_op: bool,
    ) -> MotionResult {
        use MotionResult::*;
        let n = count.unwrap_or(1);
        let Some(k) = keys.first() else { return More };
        let c = self.cursor;
        let line = t.line_of(c);
        let col = t.col(c);
        let vertical = |me: &mut Self, target_line: usize| {
            let want = *me.want_col.get_or_insert(col);
            let s = t.line_start(target_line);
            let last = if for_op || matches!(me.mode, Mode::Normal) {
                t.last_col_pos(target_line)
            } else {
                t.line_end(target_line)
            };
            let p = if want == usize::MAX {
                last
            } else {
                (s + want).min(last)
            };
            Done(p, Kind::Linewise)
        };
        let horizontal = |me: &mut Self, p: usize, kind: Kind| {
            me.want_col = None;
            Done(p, kind)
        };
        match k {
            K::Char('h') | K::Named(_)
                if k == &K::Char('h') || k.is("left") || k.is("backspace") =>
            {
                horizontal(
                    self,
                    c.saturating_sub(n).max(t.line_start(line)),
                    Kind::Exclusive,
                )
            }
            K::Char('l') | K::Char(' ') => {
                let max = if for_op {
                    t.line_end(line)
                } else {
                    t.last_col_pos(line)
                };
                horizontal(self, (c + n).min(max), Kind::Exclusive)
            }
            K::Named(nm) if nm == "right" => {
                horizontal(self, (c + n).min(t.last_col_pos(line)), Kind::Exclusive)
            }
            K::Char('j') => vertical(self, (line + n).min(t.lines() - 1)),
            K::Named(nm) if nm == "down" => vertical(self, (line + n).min(t.lines() - 1)),
            K::Char('k') => vertical(self, line.saturating_sub(n)),
            K::Named(nm) if nm == "up" => vertical(self, line.saturating_sub(n)),
            K::Char('+') | K::Named(_) if k == &K::Char('+') || k.is("enter") => {
                let l = (line + n).min(t.lines() - 1);
                self.want_col = None;
                Done(t.first_non_blank(l), Kind::Linewise)
            }
            K::Char('-') => {
                self.want_col = None;
                Done(t.first_non_blank(line.saturating_sub(n)), Kind::Linewise)
            }
            K::Char('_') => {
                self.want_col = None;
                Done(
                    t.first_non_blank((line + n - 1).min(t.lines() - 1)),
                    Kind::Linewise,
                )
            }
            K::Char('0') | K::Named(_) if k == &K::Char('0') || k.is("home") => {
                horizontal(self, t.line_start(line), Kind::Exclusive)
            }
            K::Char('^') => horizontal(self, t.first_non_blank(line), Kind::Exclusive),
            K::Char('$') | K::Named(_) if k == &K::Char('$') || k.is("end") => {
                let l = (line + n - 1).min(t.lines() - 1);
                self.want_col = Some(usize::MAX);
                let p = if for_op {
                    t.line_end(l).saturating_sub(1).max(t.line_start(l))
                } else {
                    t.last_col_pos(l)
                };
                if for_op && t.line_end(l) == t.line_start(l) {
                    return Done(t.line_start(l), Kind::Exclusive);
                }
                Done(p, Kind::Inclusive)
            }
            K::Char('|') => horizontal(
                self,
                (t.line_start(line) + n - 1).min(t.last_col_pos(line)),
                Kind::Exclusive,
            ),
            K::Char(w @ ('w' | 'W')) => {
                let mut p = c;
                for _ in 0..n {
                    p = motion::word_forward(t, p, *w == 'W');
                }
                horizontal(self, p, Kind::Exclusive)
            }
            K::Char(b @ ('b' | 'B')) => {
                let mut p = c;
                for _ in 0..n {
                    p = motion::word_back(t, p, *b == 'B');
                }
                horizontal(self, p, Kind::Exclusive)
            }
            K::Char(e @ ('e' | 'E')) => {
                let mut p = c;
                for _ in 0..n {
                    p = motion::word_end(t, p, *e == 'E');
                }
                horizontal(self, p, Kind::Inclusive)
            }
            K::Char('G') => {
                self.want_col = None;
                let l = match count {
                    Some(c) => c - 1,
                    None => t.lines() - 1,
                };
                Done(t.first_non_blank(l.min(t.lines() - 1)), Kind::Linewise)
            }
            K::Char('g') => match keys.get(1) {
                None => More,
                Some(K::Char('g')) => {
                    self.want_col = None;
                    Done(
                        t.first_non_blank((n - 1).min(t.lines() - 1)),
                        Kind::Linewise,
                    )
                }
                Some(K::Char(e @ ('e' | 'E'))) => {
                    let mut p = c;
                    for _ in 0..n {
                        p = motion::word_end_back(t, p, *e == 'E');
                    }
                    horizontal(self, p, Kind::Inclusive)
                }
                Some(K::Char('_')) => {
                    let l = (line + n - 1).min(t.lines() - 1);
                    let (s, mut e) = (t.line_start(l), t.line_end(l));
                    while e > s && matches!(t.chars[e - 1], ' ' | '\t') {
                        e -= 1;
                    }
                    horizontal(self, e.saturating_sub(1).max(s), Kind::Inclusive)
                }
                Some(K::Char('0')) => horizontal(self, t.line_start(line), Kind::Exclusive),
                Some(K::Char('m')) => horizontal(
                    self,
                    (t.line_start(line) + (t.line_end(line) - t.line_start(line)) / 2)
                        .min(t.last_col_pos(line)),
                    Kind::Exclusive,
                ),
                _ => Bad,
            },
            K::Char(f @ ('f' | 'F' | 't' | 'T')) => {
                let Some(target) = keys.get(1) else {
                    return More;
                };
                let K::Char(ch) = target else { return Bad };
                let (fwd, till) = (matches!(f, 'f' | 't'), matches!(f, 't' | 'T'));
                self.last_find = Some((*ch, fwd, till));
                match motion::find_char(t, c, *ch, fwd, till, n) {
                    Some(p) => horizontal(
                        self,
                        p,
                        if fwd {
                            Kind::Inclusive
                        } else {
                            Kind::Exclusive
                        },
                    ),
                    None => Bad,
                }
            }
            K::Char(r @ (';' | ',')) => {
                let Some((ch, fwd, till)) = self.last_find else {
                    return Bad;
                };
                let fwd = if *r == ';' { fwd } else { !fwd };
                // `;` after `t` skips the char right next to the cursor.
                let from = if till && fwd && t.char(c + 1) == Some(ch) {
                    c + 1
                } else if till && !fwd && c > 0 && t.char(c - 1) == Some(ch) {
                    c - 1
                } else {
                    c
                };
                match motion::find_char(t, from, ch, fwd, till, n) {
                    Some(p) => horizontal(
                        self,
                        p,
                        if fwd {
                            Kind::Inclusive
                        } else {
                            Kind::Exclusive
                        },
                    ),
                    None => Bad,
                }
            }
            K::Char('%') => match motion::match_pair(t, c) {
                Some(p) => horizontal(self, p, Kind::Inclusive),
                None => Bad,
            },
            K::Char('}') => {
                let mut p = c;
                for _ in 0..n {
                    p = motion::paragraph_forward(t, p);
                }
                horizontal(self, p, Kind::Exclusive)
            }
            K::Char('{') => {
                let mut p = c;
                for _ in 0..n {
                    p = motion::paragraph_back(t, p);
                }
                horizontal(self, p, Kind::Exclusive)
            }
            K::Char(hml @ ('H' | 'M' | 'L')) => {
                let (first, last) = (
                    ctx.visible.0.min(t.lines() - 1),
                    ctx.visible.1.min(t.lines() - 1),
                );
                let l = match hml {
                    'H' => (first + n - 1).min(last),
                    'L' => last.saturating_sub(n - 1).max(first),
                    _ => first + (last - first) / 2,
                };
                self.want_col = None;
                Done(t.first_non_blank(l), Kind::Linewise)
            }
            // Ctrl+F and Ctrl+B stay YACE's (Find, side bar), as VS Code users expect.
            K::Named(nm) if matches!(nm.as_str(), "ctrl+d" | "ctrl+u" | "pagedown" | "pageup") => {
                let page = (ctx.visible.1.saturating_sub(ctx.visible.0) + 1).max(2);
                let step = if matches!(nm.as_str(), "ctrl+d" | "ctrl+u") {
                    page / 2
                } else {
                    page.saturating_sub(2).max(1)
                };
                let down = matches!(nm.as_str(), "ctrl+d" | "ctrl+f" | "pagedown");
                let l = if down {
                    (line + step * n).min(t.lines() - 1)
                } else {
                    line.saturating_sub(step * n)
                };
                self.want_col = None;
                Done(t.first_non_blank(l), Kind::Linewise)
            }
            K::Char(nn @ ('n' | 'N')) => {
                let Some((pat, fwd)) = self.last_search.clone() else {
                    return Bad;
                };
                let fwd = if *nn == 'n' { fwd } else { !fwd };
                let mut p = c;
                for _ in 0..n {
                    match motion::search(t, p, &pat, fwd) {
                        Some(q) => p = q,
                        None => {
                            self.message = Some(format!("E486: Pattern not found: {pat}"));
                            return Bad;
                        }
                    }
                }
                horizontal(self, p, Kind::Exclusive)
            }
            K::Char(s @ ('*' | '#')) => {
                let Some(word) = motion::word_at(t, c) else {
                    return Bad;
                };
                self.last_search = Some((word.clone(), *s == '*'));
                // Whole words only.
                let whole = |p: usize| {
                    let before = p == 0 || crate::text::class(t.chars[p - 1], false) != 1;
                    let e = p + word.chars().count();
                    let after = e >= t.len() || crate::text::class(t.chars[e], false) != 1;
                    before && after
                };
                let mut p = motion::word_object(
                    t,
                    (c..t.line_end(line))
                        .find(|&i| crate::text::class(t.chars[i], false) == 1)
                        .unwrap_or(c),
                    false,
                    false,
                )
                .map(|x| x.0)
                .unwrap_or(c);
                let start = p;
                for _ in 0..n {
                    let mut q = p;
                    loop {
                        match motion::search(t, q, &word, *s == '*') {
                            Some(r) if whole(r) => {
                                q = r;
                                break;
                            }
                            Some(r) if r != start && r != q => q = r,
                            _ => break,
                        }
                    }
                    p = q;
                }
                horizontal(self, p, Kind::Exclusive)
            }
            _ => Bad,
        }
    }
}

enum MotionResult {
    More,
    Bad,
    Done(usize, Kind),
}

/// A count at `keys[i..]` (not starting with 0): (count, index after it).
fn parse_count(keys: &[K], i: usize) -> (Option<usize>, usize) {
    let mut j = i;
    let mut n: Option<usize> = None;
    while let Some(d) = keys.get(j).and_then(digit) {
        if n.is_none() && d == 0 {
            break;
        }
        n = Some(
            n.unwrap_or(0)
                .saturating_mul(10)
                .saturating_add(d)
                .min(100_000),
        );
        j += 1;
    }
    (n, j)
}

fn toggle_case(c: char) -> char {
    if c.is_uppercase() {
        c.to_lowercase().next().unwrap_or(c)
    } else {
        c.to_uppercase().next().unwrap_or(c)
    }
}

/// A text object `[start, end)` and whether it's whole lines.
fn text_object(t: &Text, pos: usize, around: bool, obj: char) -> Option<(usize, usize, bool)> {
    let r = match obj {
        'w' => motion::word_object(t, pos, around, false),
        'W' => motion::word_object(t, pos, around, true),
        '"' | '\'' | '`' => motion::quote_object(t, pos, obj, around),
        '(' | ')' | 'b' => motion::bracket_object(t, pos, '(', ')', around),
        '{' | '}' | 'B' => motion::bracket_object(t, pos, '{', '}', around),
        '[' | ']' => motion::bracket_object(t, pos, '[', ']', around),
        '<' | '>' => motion::bracket_object(t, pos, '<', '>', around),
        'p' => {
            let (a, b) = motion::paragraph_object(t, pos, around);
            return Some((a, b, true));
        }
        _ => None,
    }?;
    Some((r.0, r.1, false))
}
