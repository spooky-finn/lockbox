# Documentation guide

Start with the [application README](../README.md) for usage and data protection,
or the [developer guide](development.md) for build commands and layer boundaries.

| Question | Document | Scope |
| --- | --- | --- |
| How does sync work today? | [Sync engine](sync-engine.md) | Implemented experimental `LBX-SYNC-0` behavior, API and limits |
| Where should a sync change live? | [Sync architecture](sync-architecture.md) | Module ownership, extension points and durability contracts |
| How do rollback protection and recovery work? | [Profile protection](profile-protection.md) | Local witness, installation binding and recovery boundaries |
| How can I verify a real provider? | [Live Storj test](sync-live-test.md) | Opt-in acceptance probe with fictional data |
| What remains before release? | [Production delivery plan](sync-production-plan.md) | Release gates and dated validation evidence |
| What is the broader design? | [Protocol proposal](lockbox-protocol.md) | Proposed v1 design; not a claim of implemented interoperability |
| What did initial delivery cover? | [Initial implementation checklist](implementation-plan.md) | Historical milestones; use the production plan for current priorities |
| What can we simplify next? | [Maintenance review](maintenance-review.md) | Prioritized code and structure opportunities |

The [English](storj-setup.en.txt) and [Russian](storj-setup.txt) Storj instructions
are bundled into the application. Keep both versions aligned when changing setup.
[Feedback](feedback.md) records explicit project preferences.

## Version terminology

These are independent version numbers:

- Local encrypted file format: new local vaults write **v4**, shared profiles **v5**.
  Older v1/v2/v3 files remain readable.
- Bootstrap profile schema: new shared profiles use **3**, including witness protection.
- Remote wire profile: **LBX-SYNC-0**, experimental.
- Protocol proposal: **v1**, a broader target design.

Keep implemented behavior in the engine/protection guides, code ownership in the
architecture guide, and future work in the production plan. Link to the owning
guide instead of copying its tables or validation snapshots. Dated test results
describe that run only; they do not certify later working-tree changes.
