/// # Integration Tests: Identity Registry (Issue #924)
///
/// The identity registry manages DID-backed identities, verifier-driven
/// verification, reputation tracking, and social recovery initiation.
///
/// Acceptance criteria tested:
///   check Identity creation validates DID format and rejects duplicates
///   check Verification is restricted to the admin / authorized verifiers
///   check Verification raises trust score and stamps expiry
///   check Reputation updates move scores in both directions
///   check Recovery initiation requires a well-sized signature and flips active state
///   check Guardian approvals are required and unauthorized callers rejected
///
/// Note: there is currently no public message for adding guardians, so a
/// recovery cannot be *completed* through the public API; these tests cover
/// everything reachable: initiation state changes and approval gating.
#[cfg(test)]
mod integration_identity {
    use ink::env::{test, DefaultEnvironment};
    use propchain_identity::propchain_identity::{
        IdentityError, IdentityRegistry, PrivacySettings, VerificationLevel,
    };

    fn privacy() -> PrivacySettings {
        PrivacySettings {
            public_reputation: true,
            public_verification: true,
            data_sharing_consent: false,
            zero_knowledge_proof: false,
            selective_disclosure: Vec::new(),
        }
    }

    fn sig64() -> Vec<u8> {
        vec![0xAB; 64]
    }

    #[ink::test]
    fn identity_creation_validates_did_and_uniqueness() {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        let mut registry = IdentityRegistry::new();

        // Malformed DIDs are rejected.
        assert_eq!(
            registry.create_identity(
                "not-a-did".into(),
                vec![1; 32],
                "Ed25519".into(),
                None,
                privacy()
            ),
            Err(IdentityError::InvalidDid)
        );

        // A valid did:method:id creates the identity with neutral standing.
        registry
            .create_identity(
                "did:prop:alice".into(),
                vec![1; 32],
                "Ed25519".into(),
                None,
                privacy(),
            )
            .expect("valid DID accepted");

        let identity = registry.get_identity(accounts.alice).expect("stored");
        assert_eq!(identity.did_document.did, "did:prop:alice");
        assert_eq!(identity.reputation_score, 500);
        assert_eq!(identity.verification_level, VerificationLevel::None);
        assert!(!identity.is_verified);

        // Duplicate registration for the same account fails.
        assert_eq!(
            registry.create_identity(
                "did:prop:alice-2".into(),
                vec![2; 32],
                "Ed25519".into(),
                None,
                privacy()
            ),
            Err(IdentityError::IdentityAlreadyExists)
        );
    }

    #[ink::test]
    fn verification_is_verifier_gated_and_scores_trust() {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        let mut registry = IdentityRegistry::new();

        test::set_caller::<DefaultEnvironment>(accounts.bob);
        registry
            .create_identity(
                "did:prop:bob".into(),
                vec![2; 32],
                "Ed25519".into(),
                None,
                privacy(),
            )
            .expect("bob registers");

        // Random accounts are not verifiers.
        test::set_caller::<DefaultEnvironment>(accounts.charlie);
        assert_eq!(
            registry.verify_identity(accounts.bob, VerificationLevel::Standard, None),
            Err(IdentityError::Unauthorized)
        );

        // Verifying a non-existent account reports it.
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        assert_eq!(
            registry.verify_identity(accounts.eve, VerificationLevel::Standard, None),
            Err(IdentityError::IdentityNotFound)
        );

        // The admin verifies bob at Standard level with expiry.
        test::set_block_timestamp::<DefaultEnvironment>(10_000);
        registry
            .verify_identity(accounts.bob, VerificationLevel::Standard, Some(30))
            .expect("admin verifies bob");

        let identity = registry.get_identity(accounts.bob).unwrap();
        assert!(identity.is_verified);
        assert_eq!(identity.trust_score, 75);
        assert_eq!(identity.verified_at, Some(10_000));
        assert_eq!(identity.verification_expires, Some(10_000 + 30 * 86_400));

        // Reputation updates also require the admin role...
        test::set_caller::<DefaultEnvironment>(accounts.charlie);
        assert_eq!(
            registry.update_reputation(accounts.bob, true, 1_000),
            Err(IdentityError::Unauthorized)
        );

        // ...and move scores up on success and down on failure.
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        registry
            .update_reputation(accounts.bob, true, 5_000)
            .expect("successful transaction recorded");
        assert_eq!(
            registry
                .get_reputation_metrics(accounts.bob)
                .unwrap()
                .reputation_score,
            505
        );

        registry
            .update_reputation(accounts.bob, false, 500)
            .expect("failed transaction recorded");
        assert_eq!(
            registry
                .get_reputation_metrics(accounts.bob)
                .unwrap()
                .reputation_score,
            495
        );

        assert!(!registry.meets_reputation_threshold(accounts.bob, 600));
        assert!(registry.meets_reputation_threshold(accounts.bob, 400));
    }

    #[ink::test]
    fn recovery_initiation_and_guardian_gating() {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        let mut registry = IdentityRegistry::new();

        registry
            .create_identity(
                "did:prop:alice".into(),
                vec![1; 32],
                "Ed25519".into(),
                None,
                privacy(),
            )
            .expect("alice registers");

        // Undersized signatures are rejected outright.
        assert_eq!(
            registry.initiate_recovery(accounts.eve, vec![0u8; 63]),
            Err(IdentityError::InvalidSignature)
        );

        // A correctly sized signature activates the recovery process.
        registry
            .initiate_recovery(accounts.eve, sig64())
            .expect("valid signature starts recovery");

        // The active process blocks re-initiation.
        assert_eq!(
            registry.initiate_recovery(accounts.eve, sig64()),
            Err(IdentityError::RecoveryInProgress)
        );

        // Approvals require guardianship, and alice has no guardians yet:
        // even she cannot approve, which pins the guardian gating contract.
        assert_eq!(
            registry.approve_recovery(accounts.alice, accounts.eve),
            Err(IdentityError::Unauthorized)
        );

        // Non-existent identities have nothing to recover or approve.
        test::set_caller::<DefaultEnvironment>(accounts.bob);
        assert_eq!(
            registry.initiate_recovery(accounts.eve, sig64()),
            Err(IdentityError::IdentityNotFound)
        );
        assert_eq!(
            registry.approve_recovery(accounts.bob, accounts.eve),
            Err(IdentityError::IdentityNotFound)
        );
    }
}
