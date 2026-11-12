# seq_manip_imfram

seq_manip (the Tauri app at `../sequence_manipulator`) as a native Rust
window on imfram. The original is not touched: `seq-core` compiles its
`state.rs` and `thumbs.rs` straight from that checkout via `#[path]`.

```
cargo run -p seq                      # opens on ~
cargo run -p seq -- --dir PATH
cargo run -p seq -- --shot DIR        # render once offscreen to DIR/main.png
cargo test -p seq-core
```

| crate      | what |
|------------|------|
| `seq-core` | seq_manip's `state` + `thumbs` (unchanged), plus `session`: open / stage / revert / commit as plain calls |
| `seq`      | imfram window: Places, Browser (tiles or list, scrub), Info, Pending; verbs; Stage / Instant; commit on a thread with progress |

Not ported yet: player, recursive search, bin, favourites, export h264,
delete review page (deletes go to Trash on commit), send to wobble_edit.
