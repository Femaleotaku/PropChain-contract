use ink::prelude::vec::Vec;
use ink::primitives::{AccountId, Hash};

use crate::constants::MIN_RANDOMNESS_PARTICIPANTS;
use crate::crypto::{finalize_randomness, verify_commitment, CryptoError};

// ── Types ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, scale::Encode, scale::Decode)]
#[cfg_attr(
    feature = "std",
    derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout)
)]
pub enum RandomnessStatus {
    Committing,
    Revealing,
    Finalized,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, scale::Encode, scale::Decode)]
#[cfg_attr(
    feature = "std",
    derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout)
)]
pub struct CommitEntry {
    pub committer: AccountId,
    pub commit_hash: Hash,
}

#[derive(Debug, Clone, PartialEq, Eq, scale::Encode, scale::Decode)]
#[cfg_attr(
    feature = "std",
    derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout)
)]
pub struct RevealEntry {
    pub revealer: AccountId,
    pub secret: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, scale::Encode, scale::Decode)]
#[cfg_attr(
    feature = "std",
    derive(scale_info::TypeInfo, ink::storage::traits::StorageLayout)
)]
pub struct CommitRevealRound {
    pub round_id: u64,
    pub commit_deadline: u32,
    pub reveal_deadline: u32,
    pub commits: Vec<CommitEntry>,
    pub reveals: Vec<RevealEntry>,
    pub final_random: Option<Hash>,
    pub status: RandomnessStatus,
}

// ── Round Lifecycle ─────────────────────────────────────────────────────────

/// Create a new commitment-reveal round.
pub fn create_round(
    round_id: u64,
    current_block: u32,
    commit_blocks: u32,
    reveal_blocks: u32,
) -> CommitRevealRound {
    CommitRevealRound {
        round_id,
        commit_deadline: current_block.saturating_add(commit_blocks),
        reveal_deadline: current_block
            .saturating_add(commit_blocks)
            .saturating_add(reveal_blocks),
        commits: Vec::new(),
        reveals: Vec::new(),
        final_random: None,
        status: RandomnessStatus::Committing,
    }
}

/// Add a commitment to a round during the commit phase.
pub fn add_commit(
    round: &mut CommitRevealRound,
    committer: AccountId,
    commit_hash: Hash,
    current_block: u32,
) -> Result<(), CryptoError> {
    if round.status != RandomnessStatus::Committing || current_block > round.commit_deadline {
        return Err(CryptoError::InvalidRandomnessPhase);
    }
    // Prevent duplicate commits from the same account
    if round.commits.iter().any(|c| c.committer == committer) {
        return Err(CryptoError::InvalidRandomnessPhase);
    }
    round.commits.push(CommitEntry {
        committer,
        commit_hash,
    });
    Ok(())
}

/// Transition round from committing to revealing phase.
pub fn start_reveal_phase(
    round: &mut CommitRevealRound,
    current_block: u32,
) -> Result<(), CryptoError> {
    if round.status != RandomnessStatus::Committing {
        return Err(CryptoError::InvalidRandomnessPhase);
    }
    if current_block <= round.commit_deadline {
        return Err(CryptoError::InvalidRandomnessPhase);
    }
    if u32::try_from(round.commits.len()).unwrap_or(u32::MAX) < MIN_RANDOMNESS_PARTICIPANTS {
        round.status = RandomnessStatus::Failed;
        return Err(CryptoError::InsufficientReveals);
    }
    round.status = RandomnessStatus::Revealing;
    Ok(())
}

/// Reveal a secret during the reveal phase. Verifies against the commitment.
pub fn add_reveal(
    round: &mut CommitRevealRound,
    revealer: AccountId,
    secret: [u8; 32],
    current_block: u32,
) -> Result<(), CryptoError> {
    if round.status != RandomnessStatus::Revealing || current_block > round.reveal_deadline {
        return Err(CryptoError::InvalidRandomnessPhase);
    }
    // Find the matching commitment
    let commit = round
        .commits
        .iter()
        .find(|c| c.committer == revealer)
        .ok_or(CryptoError::InvalidRandomnessPhase)?;

    // Verify the reveal matches the commitment
    if !verify_commitment(&secret, &revealer, &commit.commit_hash) {
        return Err(CryptoError::CommitMismatch);
    }

    // Prevent duplicate reveals
    if round.reveals.iter().any(|r| r.revealer == revealer) {
        return Err(CryptoError::InvalidRandomnessPhase);
    }

    round.reveals.push(RevealEntry { revealer, secret });
    Ok(())
}

/// Finalize the round and compute the random value from all revealed secrets.
pub fn finalize_round(
    round: &mut CommitRevealRound,
    current_block: u32,
) -> Result<Hash, CryptoError> {
    if round.status != RandomnessStatus::Revealing {
        return Err(CryptoError::InvalidRandomnessPhase);
    }
    if current_block <= round.reveal_deadline {
        // Can only finalize after reveal deadline to prevent early finalization attacks
        return Err(CryptoError::InvalidRandomnessPhase);
    }
    if u32::try_from(round.reveals.len()).unwrap_or(u32::MAX) < MIN_RANDOMNESS_PARTICIPANTS {
        round.status = RandomnessStatus::Failed;
        return Err(CryptoError::InsufficientReveals);
    }

    let secrets: Vec<[u8; 32]> = round.reveals.iter().map(|r| r.secret).collect();
    let random = finalize_randomness(&secrets);
    round.final_random = Some(random);
    round.status = RandomnessStatus::Finalized;
    Ok(random)
}

#[cfg(test)]
mod tests {
    use ink::primitives::AccountId;

    use super::*;
    use crate::crypto::compute_commitment;

    fn account(byte: u8) -> AccountId {
        AccountId::from([byte; 32])
    }

    #[test]
    fn commit_reveal_round_produces_final_random() {
        let mut round = create_round(7, 100, 10, 10);
        assert_eq!(round.status, RandomnessStatus::Committing);
        assert_eq!(round.commit_deadline, 110);
        assert_eq!(round.reveal_deadline, 120);

        let secret_a = [1u8; 32];
        let secret_b = [2u8; 32];
        let commit_a = compute_commitment(&secret_a, &account(1));
        let commit_b = compute_commitment(&secret_b, &account(2));

        assert!(add_commit(&mut round, account(1), commit_a, 105).is_ok());
        // Duplicate commits from the same account are rejected.
        assert_eq!(
            add_commit(&mut round, account(1), commit_a, 105),
            Err(CryptoError::InvalidRandomnessPhase)
        );
        assert!(add_commit(&mut round, account(2), commit_b, 105).is_ok());

        // The reveal phase cannot start before the commit deadline.
        assert_eq!(
            start_reveal_phase(&mut round, 105),
            Err(CryptoError::InvalidRandomnessPhase)
        );
        assert!(start_reveal_phase(&mut round, 111).is_ok());

        // A wrong secret fails commitment verification.
        assert_eq!(
            add_reveal(&mut round, account(1), [9u8; 32], 112),
            Err(CryptoError::CommitMismatch)
        );
        // An account that never committed cannot reveal.
        assert_eq!(
            add_reveal(&mut round, account(3), [3u8; 32], 112),
            Err(CryptoError::InvalidRandomnessPhase)
        );
        assert!(add_reveal(&mut round, account(1), secret_a, 112).is_ok());
        assert!(add_reveal(&mut round, account(2), secret_b, 112).is_ok());

        // Early finalization is blocked to prevent last-revealer attacks.
        assert_eq!(
            finalize_round(&mut round, 115),
            Err(CryptoError::InvalidRandomnessPhase)
        );
        let random = finalize_round(&mut round, 121).expect("finalizes");
        assert_eq!(round.status, RandomnessStatus::Finalized);
        assert_eq!(round.final_random, Some(random));

        // The same revealed secrets always finalize to the same value.
        assert_eq!(finalize_randomness(&[secret_a, secret_b]), random);
    }

    #[test]
    fn too_few_commits_fail_the_round_instead_of_revealing() {
        let mut round = create_round(1, 0, 5, 5);
        let commit = compute_commitment(&[3u8; 32], &account(1));
        assert!(add_commit(&mut round, account(1), commit, 3).is_ok());

        assert_eq!(
            start_reveal_phase(&mut round, 6),
            Err(CryptoError::InsufficientReveals)
        );
        assert_eq!(round.status, RandomnessStatus::Failed);
    }

    #[test]
    fn commits_are_rejected_after_the_deadline() {
        let mut round = create_round(2, 0, 5, 5);
        let commit = compute_commitment(&[4u8; 32], &account(1));
        assert_eq!(
            add_commit(&mut round, account(1), commit, 6),
            Err(CryptoError::InvalidRandomnessPhase)
        );
        assert!(round.commits.is_empty());
    }
}
