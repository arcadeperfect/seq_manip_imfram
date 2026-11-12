# seq_manip_imfram

seq_manip (the Tauri app at `../sequence_manipulator`) as a native Rust
window on imfram. The original is not touched: `seq-core` compiles its
`state.rs`, `thumbs.rs` **and `commands.rs`** straight from that checkout
via `#[path]`; `tauri-shim` gives `commands.rs` just enough Tauri
(`State`, `Channel`, `AppHandle`, `spawn_blocking`, a no-op `#[command]`)
to build. So every command the webview called is the same code here.

```
cargo run -p seq                          # opens on the last folder
cargo run -p seq -- --dir PATH
cargo run -p seq -- --shot DIR            # render once offscreen to DIR/main.png
cargo run -p seq -- --shot DIR --play NAME
cargo test -p seq-core -p seq
```

Fixtures: `../sequence_manipulator/scripts/make_fixtures.sh DIR`.
`seq-core/binaries` links to the Tauri app's bundled ffmpeg. The thumbnail
and playback caches are the Tauri app's own (`~/Library/Caches/com.alex.seq-manip`).

| crate / file | what |
|---|---|
| `seq-core` | the app's `state`, `thumbs`, `commands` (unchanged) + `backend`: one plain call per command |
| `tauri-shim(-macros)` | the few Tauri names `commands.rs` uses |
| `seq/app.rs` | tabs, state, every action (stage, commit gates, bin, delete queue, search, Nuke, wobble) |
| `seq/browser.rs` | path bar (Tab completes), toolbar, filters, grid / list, type-ahead, drops |
| `seq/player.rs` | sequences and movies, RAM buffer, J K L, hands, view transform, exposure |
| `seq/panels.rs` | Folders (favourites + tree), Info, Pending, Bin, Delete queue, cards |
| `seq/logic.rs` | filtering / sorting, completion, type-ahead, frame numbers (tested) |

Q over any panel shows it alone; Q again puts the layout back.

Left out on purpose: Sentry, the beta expiry, the onboarding coachmarks
(a splash stands in).
