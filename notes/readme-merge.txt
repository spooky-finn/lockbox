README screenshot + dev merge

- Screenshot stored docs/images/lockbox-screenshot.png; README links near title.
- Company labels randomized; username replaced demo@example.com.
- Merge keeps main save-on-Enter, navigation, native title-bar reservation; adds dev tabs + compact layout.
- Native title bar owns window controls; hide old tab-strip drag overlay so first tab stays clickable.
- Checks pass: format, 537 Rust unit + 5 integration + 2 doc tests, 25 browser + 9 keyboard + 33 release-script tests.
- Ignored: 3 Rust probes; skipped: 1 real-browser probe.
- Strict Clippy blocked by existing unused forget_path, FolderX, Settings::remember.
