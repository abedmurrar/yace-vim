//! Vim keys for YACE. The engine (`engine.rs`) knows Vim; this file feeds it
//! the keys YACE captures for it and applies what it returns.

pub mod engine;
pub mod motion;
pub mod text;

use engine::{Action, Ctx, K, Mode, Vim};
use serde_json::Value;
use text::Text;
use yace_plugin::{Edit, Host, Key, Level, Plugin, Result, StatusItem, export_plugin};

pub struct VimPlugin {
    host: Host,
    vim: Vim,
    enabled: bool,
    /// (doc, version, text): fetched again only when the document changed.
    cache: Option<(u64, u64, Text)>,
    last_doc: Option<u64>,
}

impl VimPlugin {
    fn show(&self) {
        let text = if self.enabled {
            self.vim.status()
        } else {
            String::new()
        };
        let item = (!text.is_empty()).then(|| StatusItem {
            text,
            ..Default::default()
        });
        let _ = self.host.set_status_item("vim.mode", item.as_ref());
        let style = match (self.enabled, self.vim.mode) {
            (false, _) => "default",
            (_, Mode::Insert) if self.vim.cmdline.is_none() => "bar",
            (_, Mode::Replace) => "underline",
            _ => "block",
        };
        let _ = self.host.set_cursor_style(style);
    }

    fn set_enabled(&mut self, on: bool) -> Result<()> {
        if !self.host.has_editor_control() {
            self.host
                .notify(Level::Warning, "Vim needs YACE 0.15 or later");
            return Ok(());
        }
        self.enabled = on;
        self.vim = Vim::new();
        self.host.capture_keys(on)?;
        self.show();
        Ok(())
    }

    fn text(&mut self, doc: u64, version: u64) -> Result<Text> {
        if let Some((d, v, t)) = &self.cache
            && *d == doc
            && *v == version
        {
            return Ok(t.clone());
        }
        let s = self.host.document_text(doc, 0, u64::MAX)?;
        Ok(Text::new(&s))
    }

    fn handle(&mut self, key: &Key) -> Result<bool> {
        let k = K::from_editor(&key.key, key.text.as_deref());
        // Typing in Insert mode: no need to look at the document.
        if self.vim.insert_passes(&k) {
            return Ok(false);
        }
        let Some(doc) = self.host.active_document()? else {
            return Ok(false);
        };
        if self.last_doc != Some(doc.doc) {
            // Another editor: start over in Normal mode (keep registers).
            self.last_doc = Some(doc.doc);
            if self.vim.mode != Mode::Insert {
                self.vim.mode = Mode::Normal;
            }
            self.vim.cmdline = None;
        }
        let sel = doc
            .selections
            .first()
            .copied()
            .unwrap_or(yace_plugin::Selection { anchor: 0, head: 0 });
        let mut t = self.text(doc.doc, doc.version)?;
        let visible = doc
            .visible_lines
            .map(|v| (v.first as usize, v.last as usize))
            .unwrap_or((0, 40));
        let ctx = Ctx {
            visible,
            tab_size: doc.tab_size.unwrap_or(4) as usize,
            insert_spaces: doc.insert_spaces.unwrap_or(true),
            popup: key.popup,
        };
        let actions = self
            .vim
            .feed(k, &mut t, sel.head as usize, sel.anchor as usize, &ctx);
        let mut version = doc.version;
        let mut handled = true;
        for a in actions {
            match a {
                Action::Edit { start, end, text } => {
                    self.host.apply_edits(
                        doc.doc,
                        version,
                        &[Edit {
                            start: start as u64,
                            end: end as u64,
                            text,
                        }],
                    )?;
                    version += 1;
                }
                Action::Cursor(p) => {
                    let _ = self
                        .host
                        .set_selections(doc.doc, version, &[(p as u64, p as u64)]);
                }
                Action::Select { anchor, head } => {
                    let _ =
                        self.host
                            .set_selections(doc.doc, version, &[(anchor as u64, head as u64)]);
                }
                Action::Command(id) => self.host.execute_command(&id, None)?,
                Action::Open(path) => {
                    let _ = self.host.open_file(&path, 0, 0);
                }
                Action::Pass => handled = false,
            }
        }
        self.cache = Some((doc.doc, version, t));
        self.show();
        Ok(handled)
    }
}

impl Plugin for VimPlugin {
    fn new(host: Host) -> Self {
        Self {
            host,
            vim: Vim::new(),
            enabled: false,
            cache: None,
            last_doc: None,
        }
    }

    fn activate(&mut self) -> Result<()> {
        self.set_enabled(true)
    }

    fn deactivate(&mut self) {
        if self.enabled {
            let _ = self.host.capture_keys(false);
            let _ = self.host.set_cursor_style("default");
        }
    }

    fn command(&mut self, id: &str, _args: Option<Value>) -> Result<()> {
        match id {
            "vim.toggle" => {
                let on = !self.enabled;
                self.set_enabled(on)?;
                self.host
                    .notify(Level::Info, if on { "Vim keys on" } else { "Vim keys off" });
                Ok(())
            }
            _ => Err(yace_plugin::Error::unsupported()),
        }
    }

    fn key(&mut self, key: &Key) -> bool {
        if !self.enabled {
            return false;
        }
        match self.handle(key) {
            Ok(h) => h,
            Err(e) => {
                self.host.log(Level::Warning, &format!("{}: {e}", key.key));
                self.cache = None;
                // A failure must not swallow typing in Insert mode.
                self.vim.mode != Mode::Insert
            }
        }
    }
}

export_plugin!(VimPlugin);

#[cfg(test)]
mod tests;
