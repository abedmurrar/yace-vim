# YACE Vim

Vim keys for [YACE](https://github.com/abedmurrar/yace), the terminal code
editor: Normal, Insert and Visual modes, with YACE's own shortcuts
(`Ctrl+P`, `Ctrl+Shift+P`, `Ctrl+S`, `Ctrl+B`, …) still working next to them.

## Install

In YACE 0.15 or later: **Extensions: Browse Plugins…** and pick Vim, or
from a shell:

```sh
yace --install-plugin io.github.abedmurrar.yace-vim
```

It's native code (built for x86_64 and arm64 Linux), so YACE shows its
SHA-256 and the two capabilities it needs before installing: `keyboard`
(see the keys you type in an editor first) and `editor.selections` (move
the cursor). Vim keys start right away; **Vim: Toggle Vim Keys** turns them
off and on.

## What works

| | |
|---|---|
| Modes | Normal, Insert, Visual, Visual Line, Replace (`R`); the status bar shows the mode, and the cursor is a block in Normal mode and a bar in Insert |
| Motions | `h j k l`, `w W b B e E ge gE`, `0 ^ $ g_ gm \|`, `gg G` (with a count), `f F t T ; ,`, `%`, `{ }`, `H M L`, `+ - _ Enter`, `Ctrl+D Ctrl+U`, `/ ? n N * #` |
| Operators | `d c y > < g~ gu gU`, doubled (`dd`, `cc`, `>>`…) and with any motion or text object |
| Text objects | `iw aw iW aW`, `i" a" i' a' i\` a\``, `i( a( ib`, `i{ a{ iB`, `i[ a[`, `i< a<`, `ip ap` |
| Editing | `x X s S D C Y r ~ J gJ`, `p P` (charwise and linewise), `i a I A o O gi`, `u Ctrl+R` (YACE's undo), counts everywhere |
| Registers | `"` (unnamed), `"0` (last yank), `"a`–`"z` (`"A` appends), `"_` (black hole) |
| Repeat | `.` (with a new count: `3.`), macros `q{a-z}` … `q`, `@{a-z}`, `@@` |
| Visual | `v V`, motions and text objects, `o`, `d x c s y > < ~ u U J p`, `I A`, `gv` |
| Insert | typing goes straight to YACE (completion, auto-closing, snippets work as usual); `Ctrl+W`, `Ctrl+U`; `Esc` or `Ctrl+[` |
| Command line | `:w :wa :q :q! :wq :x :qa :wqa :e path :e! :{line} :sp :vs :bn :bp :noh`, `:s/old/new/[g]` and `:%s/…` (literal text) |

**Not (yet):** Vim script and `.vimrc`, marks, `"+` (the system clipboard:
use YACE's `Ctrl+C`/`Ctrl+V`), regular expressions in `/` and `:s` (literal
text, lower case matches any case), `zz`/`zt`/`zb`, block Visual mode
(`Ctrl+V` stays YACE's paste), window commands (`Ctrl+W` in Normal mode
stays YACE's Close Editor). `Ctrl+F`, `Ctrl+B`, `Ctrl+P`, `Ctrl+N` and
`Ctrl+J` keep their YACE meaning.

## Development

The engine (`src/engine.rs`, `src/motion.rs`, `src/text.rs`) knows Vim and
nothing about YACE: keys in, edits and cursor moves out. `cargo test` runs
it on about 80 typed sequences (`src/tests.rs`). `src/lib.rs` connects it
to YACE through the plugin SDK.

`scripts/package.sh` builds the library for this machine and packages it as
`dist/yace-vim-<version>-linux-<arch>.tar.gz` (needs `yace` on `PATH`).
YACE's repository has an end-to-end test that loads it into the editor:

```sh
cd path/to/yace
YACE_TEST_VIM=/path/to/an/unpacked/package cargo test -p yace --test m8 real_vim_plugin -- --nocapture
```

To release: bump `version` in `yace-plugin.toml` and `Cargo.toml`, commit,
and push a tag `vX.Y.Z`. The release workflow builds both architectures
and publishes them. Then list them in the YACE plugin index, from a YACE
checkout:

```sh
scripts/plugin_index.py add https://github.com/abedmurrar/yace-vim/releases/download/vX.Y.Z/yace-vim-X.Y.Z-linux-x86_64.tar.gz --target linux-x86_64
scripts/plugin_index.py add https://github.com/abedmurrar/yace-vim/releases/download/vX.Y.Z/yace-vim-X.Y.Z-linux-aarch64.tar.gz --target linux-aarch64
```

## Licence

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option.
