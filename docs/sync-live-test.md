# Live Storj acceptance test

Run from the repository root against a dedicated test bucket with fictional data.
The probe uses the production HTTPS/SigV4 adapter and two independently generated
device profiles. It never uploads a profile or private device key.

Provision two different S3 access keys with LIST, GET and PUT access to the test
bucket. The test creates a unique `spaces/<space_name>/` prefix using a random
suffix in the space name. It does not
modify existing prefixes or require DELETE, bucket administration or public access.
See [Storj setup](storj-setup.en.txt) for gateway credentials.

Supply these variables through your local secret manager or shell environment;
do not put their values in source files, shell transcripts, issue comments or chat:

| Variable | Meaning |
| --- | --- |
| `LOCKBOX_STORJ_ENDPOINT` | HTTPS gateway origin, without a path/query |
| `LOCKBOX_STORJ_BUCKET` | Dedicated test bucket |
| `LOCKBOX_STORJ_OWNER_ACCESS_KEY` | First device's access key |
| `LOCKBOX_STORJ_OWNER_SECRET_KEY` | First device's secret key |
| `LOCKBOX_STORJ_EDITOR_ACCESS_KEY` | Second device's different access key |
| `LOCKBOX_STORJ_EDITOR_SECRET_KEY` | Second device's secret key |
| `LOCKBOX_STORJ_REGION` | Optional signing region; defaults to `us-east-1` |

```sh
cargo run --locked --example storj_sync_probe
```

The probe exercises admission, local offline saves, concurrent whole-record
conflicts, a deliberately lost response after a successful PUT, retry, explicit
resolution, reader enforcement, epoch rotation, revocation, excluded drafts and
reopening both encrypted profiles. The same scenario runs without network in
`tests/sync_scenario.rs`; the live run substitutes real Storj clients.

Only a final `PASS` confirms every assertion. A successful HTTP upload alone is
insufficient. Missing configuration fails before network I/O. Errors report
categories without provider response bodies, credentials or secret record values.
The test reports its unique space prefix before exchange, so encrypted fixture
objects can be removed from that exact prefix through the provider console.
Local profiles, disposable passwords and an explicitly injected directory witness
are temporary. The S3 probe does not access the personal Keychain; the separate
[profile protection probe](profile-protection.md) verifies that OS adapter.

A gateway with delayed visibility may require another run; the acceptance probe
uses strict sequential assertions rather than hiding incomplete exchanges behind
unbounded retry. The desktop engine performs bounded discovery and retry. A live
failure should be investigated and recorded, not reclassified as a pass.

This probe does not complete a security audit, simulate full-machine rollback,
or validate large-bucket pagination. Those remain separate release evidence in
[the production plan](sync-production-plan.md).
