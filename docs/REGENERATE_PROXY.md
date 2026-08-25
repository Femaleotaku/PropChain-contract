# PropChain Transparent Proxy — Design & Regeneration Guide

## Overview

The `TransparentProxy` contract provides upgradeable contract routing with DAO-governed upgrade governance. It stores an implementation code hash and forwards calls to the implementation via cross-contract calls.

## Architecture

### Storage Layout

The proxy's storage is intentionally separate from the implementation's storage:

```rust
pub struct TransparentProxy {
    implementation_code_hash: Hash,   // Current implementation
    pending_upgrade: Hash,             // Staged new implementation (zero if none)
    upgrade_effective_at: u64,         // Block number when pending upgrade activates
    admin: AccountId,                  // DAO timelock address
    upgrade_delay_blocks: u64,         // Delay between staging and confirmation
}
```

### Upgrade Governance

1. **Stage**: Admin calls `set_code_hash(new_hash)` to propose an upgrade.
2. **Wait**: `upgrade_delay_blocks` blocks must pass (default: 100 blocks).
3. **Confirm**: Admin calls `confirm_code_hash()` to commit the new implementation.

This two-step pattern prevents immediate upgrades and gives time for community review.

### Call Forwarding

Admin messages (`set_code_hash`, `confirm_code_hash`, `set_upgrade_delay_blocks`, read-only queries) are handled by the proxy itself.

All other calls are forwarded to the implementation via `call_implementation(selector, input)` using cross-contract calls with `TAIL_CALL` semantics.

**Note**: ink! does not support native delegatecall. The implementation runs with its own storage. For true delegatecall semantics, a low-level storage overlay would be required (out of scope for this iteration).

## Regeneration

To regenerate this contract from scratch:

1. Read `contracts/proxy/src/lib.rs`
2. Read `docs/proxy_upgrade_governance.md` for governance constraints
3. Implement the storage struct, constructor, admin messages, and fallback
4. Add tests covering: construction, staging, delay, confirmation, unauthorized access
5. Verify: `cargo test -p propchain-proxy` and `cargo clippy --all-targets --all-features -- -D warnings`
