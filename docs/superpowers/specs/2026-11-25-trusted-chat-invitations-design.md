# Owner-approved invitations through a trusted chat

## Decision

For new managed Storj invitations, use the existing private conversation to authenticate both handoffs. The guest trusts the invitation received from the owner. The owner trusts the confirmation received from the intended person in that conversation. Each person can recognize the other's chat account. Lockbox can authenticate signed messages and device keys. The messaging app supplies the human identity context.

## Approaches

The owner chose explicit approval. Returning a confirmation link adds one handoff and lets the owner identify the exact requesting device. Automatic admission on first redemption would give that decision to the first holder of the invitation. The existing independent-code and fingerprint flow remains available for previously issued invitations.

## Interaction and protocol

1. The owner picks the vault, role and expiry before sending the link. The controller signs the role and invitation mode along with the vault and session. An existing member's additional device must match the member's current role.
2. The guest opens the link and reviews the vault and role. Tapping **Join** records local consent to those exact terms in the encrypted profile. Lockbox generates the signed device request and provides a confirmation link to send back to the owner in the same chat. A copy action supports messengers without desktop sharing integration.
3. The owner opens the guest's confirmation from that conversation. The link identifies the ticket and full signed request hash. Lockbox verifies the stored ticket, request signature and certificate before presenting **Approve**. A display name or another pending mailbox request cannot select the device. If the exact request has not arrived, the owner can retry. Opening either link makes no admission decision.
4. Approval pins one request and issues the addressed offer. The guest's unlocked app can sign the existing protocol consent automatically only after checking the controller, ticket, vault, recipient device, role, expiry and pinned control proof against the persisted Join decision. A mismatch stops admission. A role change requires a fresh invitation. Signed admission, encrypted credential delivery and per-device revocation continue through the existing protocol.

## Acceptance checks

A forwarded invitation alone cannot grant access. Concurrent requests cannot inherit each other's approval. Cancellation and expiry prevent new admissions. Restarting either app resumes its recorded decision without another tap or duplicate membership. Neither chat link carries a reusable vault storage secret. Legacy links keep their original verification requirements. On platforms without a registered URL handler, both handoffs support paste.
