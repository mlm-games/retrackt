# retrackt

TrackMania-style time-trial racer and track editor on repame + repose.

Fixed 120 Hz simulation, arcade car that sticks to the surface it drives on
(loops, banks, wall rides), gate racing with best times, and replay recording.
The editor places shared track pieces; the same document feeds rendering,
collision and the share-code fingerprint.

## Run

```bash
cargo run              # desktop
cargo test --workspace # sim, format, session, save suites
trunk serve            # web
```

## Structure

```
src/
├── app/       frame loop, input, screens, chase camera, scene
├── sim/       arcade car and track world, stepped at SIM_HZ
├── ui/        title, race HUD, editor, results views
├── session/   run timing, checkpoints, results
└── save/      best times and track files
crates/
└── retrackt-format/  track document, pieces, replay tape, share codes
```

`SIM_HZ` is 120 and the step is fixed, so a replay reproduces a run exactly.

## License

GPL-3.0
