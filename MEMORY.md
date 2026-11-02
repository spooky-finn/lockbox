# Project feedback

- Cloud recovery: user wants access after total local loss using same root/12 words + Storj credentials/bucket. Derive recovery capability; keep device/epoch keys random. Encrypted controller-signed capsules, fresh recovered device, explicit cloud freshness trust, independent-anchor API. No silent empty recreation. See docs/cloud-recovery.txt.

- Record history: local encrypted revisions; cloud verified signer fingerprints, causal order, no wall-clock times. Secrets hidden by default.
- Trash: deletion retains values; restore same ID/files; missing folder restores root. No purge. Shared conflicts excluded from Trash.
- File formats v6/v7; reads v1–v5. Old clients reject new files. Local history starts after upgrade; cloud history requires epoch keys.
- Record fields: single click username/password/URL copies via guarded clipboard; successful copy shows green “Copied” beside text briefly. Keyboard activation supported; password stays masked.
- Scrolling: app stays viewport height; sidebar record tree and detail scroll independently. Toolbar, search, sidebar footer, status stay visible. Desktop/mobile layout checked.

- Website icons: user accepts unencrypted persistent caching of icons/site metadata. Keep disk cache at OS cache dir `lockbox/favicons/v1` across locks/restarts; SHA-256 filenames, 30-day images, 5-minute failures, Unix directories 0700/files 0600. Bundled default; network opt-in under Security, exact HTTPS origin, pinned public IPs, no redirects/proxies. Concurrent per-site requests share downloads; cache failures preserve usable images.
