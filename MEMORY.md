# Project feedback

- 2026-11-01: Simplify `membership.rs` through encapsulation. Small facades;
  private types own roster, authority and epoch-encryption rules. Preserve signed
  wire format, error precedence and durability boundaries.
- 2026-11-01: Entry attachments: dedicated worktree; encrypt names + bytes in
  local vaults and cloud sync. Save/Cancel own attachment edits.
