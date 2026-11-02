# User feedback

- 2026-11-01: Unencrypted favicon storage is acceptable. Preserve persistent icon caching across locking and restarting, while keeping private filesystem permissions and secure download controls.
- 2026-11-01: Use the official AWS S3 SDK for Storj integration; do not use third-party S3 libraries.
- 2026-11-01: Simplify `membership.rs` through encapsulation.
- 2026-11-01: Keep one current protocol during interactive development. Drop old formats and compatibility branches as it changes. Versioning may return when compatibility becomes necessary.
