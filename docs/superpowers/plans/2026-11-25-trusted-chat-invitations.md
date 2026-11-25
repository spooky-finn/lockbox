# Trusted chat invitations implementation plan

**Goal** — Join with guest consent and one owner approval through a trusted conversation.

**Architecture** — New managed tickets carry an optional signed role. A confirmation identifies the ticket and full request hash. Only a guest's persisted Join decision for that exact signed ticket may automate addressed consent.

**Tech stack** — Rust, Dioxus, existing signed pairing protocol and encrypted profile.

**Spec** — [Owner-approved invitations](../specs/2026-11-25-trusted-chat-invitations-design.md).

## Global constraints

Retain legacy invitation verification. Opening links must never join or approve. Preserve device-bound storage delivery, deadlines and one-device admission.

## Review focus

Check wrong-ticket references, out-of-order requests, competing devices, restart between Join and offer, and changes to the offered role.

## Tasks

- [ ] Add optional signed chat role to TicketBody and canonical ConfirmationLink in sync/protocol/pairing. Implement role-bound creation, explicit trusted Join, confirmation lookup and approval, and exact-term automatic consent in vault/pairing. Exercise both legacy and trusted paths in vault/tests/pairing.rs.
- [ ] In sharing/views/links, add owner role at issuance, guest Join with signed preview, guest confirmation copy and owner confirmation paste/review. Add English and Russian strings and view regressions.
- [ ] In desktop/links route native confirmation to the issuing owner's sharing screen, preserving pending input across unlock; cover wrong profile, locked owner and absence of approval on open.
- [ ] Update invitation documentation. Review all three integrations, run cargo test --locked and cargo clippy --locked --all-targets -- -D warnings, and verify formatting.
