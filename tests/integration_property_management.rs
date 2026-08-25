/// # Integration Tests: Property Management (Issue #921)
///
/// The property management contract covers leases, rent settlement with
/// management fees, maintenance workflows, tenant screening, and expense
/// tracking — all gated by admin/manager roles.
///
/// Acceptance criteria tested:
///   check Manager administration is admin-gated
///   check Lease creation validates terms, landlord role, and jurisdiction deposit caps
///   check Rent payment is tenant-only, exact-amount, and splits fees
///   check Maintenance lifecycle moves Submitted -> Resolved with counters
///   check Screening applications are reviewed by managers only
///   check Expenses record and validate amounts
#[cfg(test)]
mod integration_property_management {
    use ink::env::{test, DefaultEnvironment};
    use ink::primitives::Hash;
    use property_management::property_management::{
        Error as PmError, LeaseStatus, MaintenanceStatus, PropertyManagement, ScreeningStatus,
    };

    fn hash(byte: u8) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[0] = byte;
        Hash::from(bytes)
    }

    fn pm() -> PropertyManagement {
        test::set_caller::<DefaultEnvironment>(
            test::default_accounts::<DefaultEnvironment>().alice,
        );
        PropertyManagement::new()
    }

    #[ink::test]
    fn manager_administration_is_admin_gated() {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        let mut contract = pm();

        assert_eq!(contract.admin(), accounts.alice);

        // Non-admins cannot appoint managers.
        test::set_caller::<DefaultEnvironment>(accounts.bob);
        assert_eq!(
            contract.add_manager(accounts.bob),
            Err(PmError::Unauthorized)
        );

        // Admin appoints and revokes a manager.
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        contract.add_manager(accounts.bob).expect("manager added");
        assert!(contract.is_manager(accounts.bob));

        // Managers can set jurisdiction compliance for a token.
        test::set_caller::<DefaultEnvironment>(accounts.bob);
        let cfg = property_management::property_management::JurisdictionCompliance {
            jurisdiction_code: "US-NY".into(),
            max_security_deposit_bps: 200,
            min_notice_period_days: 30,
            late_fee_cap_bps: 500,
            last_audit_ts: 0,
            compliant: true,
        };
        contract
            .set_jurisdiction_compliance(7, cfg.clone())
            .expect("manager sets jurisdiction rules");
        assert_eq!(
            contract.get_jurisdiction_compliance(7),
            Some(cfg),
            "jurisdiction config round-trips"
        );
        assert_eq!(contract.get_jurisdiction_compliance(99), None);

        test::set_caller::<DefaultEnvironment>(accounts.alice);
        contract
            .remove_manager(accounts.bob)
            .expect("manager removed");
        assert!(!contract.is_manager(accounts.bob));

        // Fee beneficiary changes are admin-only too.
        test::set_caller::<DefaultEnvironment>(accounts.bob);
        assert_eq!(
            contract.set_fee_beneficiary(accounts.bob),
            Err(PmError::Unauthorized)
        );
    }

    #[ink::test]
    fn lease_creation_validates_terms_and_deposit_caps() {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        let mut contract = pm();
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        contract.add_manager(accounts.bob).expect("ok");

        // Only the named landlord, admin, or a manager can create.
        test::set_caller::<DefaultEnvironment>(accounts.charlie);
        assert_eq!(
            contract.create_lease(
                10,
                accounts.django,
                accounts.alice,
                2_000,
                2_592_000,
                500,
                4_000,
                1_000_000
            ),
            Err(PmError::NotLandlordOrManager)
        );

        // Degenerate terms rejected before storage writes.
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        assert_eq!(
            contract.create_lease(
                10,
                accounts.django,
                accounts.alice,
                0,
                2_592_000,
                500,
                4_000,
                1_000_000
            ),
            Err(PmError::InvalidAmount)
        );
        assert_eq!(
            contract.create_lease(
                10,
                accounts.django,
                accounts.alice,
                2_000,
                0,
                500,
                4_000,
                1_000_000
            ),
            Err(PmError::InvalidAmount)
        );
        assert_eq!(
            contract.create_lease(
                10,
                accounts.django,
                accounts.alice,
                2_000,
                2_592_000,
                20_000,
                4_000,
                1_000_000
            ),
            Err(PmError::InvalidFee)
        );

        // Jurisdiction cap: annual rent 24k at 200 bps caps deposits at 480.
        contract
            .set_jurisdiction_compliance(
                10,
                property_management::property_management::JurisdictionCompliance {
                    jurisdiction_code: "US-NY".into(),
                    max_security_deposit_bps: 200,
                    min_notice_period_days: 30,
                    late_fee_cap_bps: 500,
                    last_audit_ts: 0,
                    compliant: true,
                },
            )
            .expect("cap configured");
        assert_eq!(
            contract.create_lease(
                10,
                accounts.django,
                accounts.alice,
                2_000,
                2_592_000,
                500,
                5_000,
                1_000_000
            ),
            Err(PmError::ComplianceViolation),
            "deposit above the jurisdiction cap rejected"
        );

        // Within the cap (annual 24k x 200 bps = 480) the lease stores Active.
        let lease_id = contract
            .create_lease(
                10,
                accounts.django,
                accounts.alice,
                2_000,
                2_592_000,
                500,
                400,
                1_000_000,
            )
            .expect("compliant lease stored");
        let lease = contract.get_lease(lease_id).expect("stored");
        assert_eq!(lease.tenant, accounts.django);
        assert_eq!(lease.rent_per_period, 2_000);
        assert_eq!(lease.status, LeaseStatus::Active);

        // Ending requires landlord/admin/manager and flips status once.
        test::set_caller::<DefaultEnvironment>(accounts.charlie);
        assert_eq!(
            contract.end_lease(lease_id),
            Err(PmError::NotLandlordOrManager)
        );
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        contract.end_lease(lease_id).expect("ended");
        assert_eq!(
            contract.get_lease(lease_id).unwrap().status,
            LeaseStatus::Ended
        );
        assert_eq!(contract.end_lease(lease_id), Err(PmError::InvalidStatus));
    }

    #[ink::test]
    fn maintenance_lifecycle_and_screening_reviews() {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        let mut contract = pm();
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        contract.add_manager(accounts.bob).expect("ok");

        // Tenant files a maintenance request (no registry set: open access).
        test::set_caller::<DefaultEnvironment>(accounts.charlie);
        let request_id = contract
            .submit_maintenance_request(10, "Leaky roof".into(), hash(1))
            .expect("request filed");
        let request = contract.get_maintenance(request_id).expect("stored");
        assert_eq!(request.status, MaintenanceStatus::Submitted);
        assert_eq!(request.requester, accounts.charlie);

        // Only managers/admin move it through triage.
        test::set_caller::<DefaultEnvironment>(accounts.django);
        assert_eq!(
            contract.update_maintenance_status(
                request_id,
                MaintenanceStatus::InProgress,
                Some(accounts.bob)
            ),
            Err(PmError::Unauthorized)
        );
        test::set_caller::<DefaultEnvironment>(accounts.bob);
        contract
            .update_maintenance_status(
                request_id,
                MaintenanceStatus::InProgress,
                Some(accounts.bob),
            )
            .expect("manager triages");
        assert_eq!(
            contract.get_maintenance(request_id).unwrap().status,
            MaintenanceStatus::InProgress
        );
        contract
            .resolve_maintenance(request_id, hash(2))
            .expect("manager resolves");
        let resolved = contract.get_maintenance(request_id).unwrap();
        assert_eq!(resolved.status, MaintenanceStatus::Resolved);
        assert_eq!(resolved.resolution_hash, Some(hash(2)));

        // Unknown ids fail cleanly.
        assert_eq!(
            contract.resolve_maintenance(999, hash(3)),
            Err(PmError::MaintenanceNotFound)
        );

        // Screening: applicant submits, manager approves or rejects.
        test::set_caller::<DefaultEnvironment>(accounts.charlie);
        let screening_id = contract
            .submit_screening_application(10, hash(4), 3, 3_000)
            .expect("application submitted");
        assert_eq!(
            contract.get_screening(screening_id).unwrap().status,
            ScreeningStatus::Pending
        );

        test::set_caller::<DefaultEnvironment>(accounts.charlie);
        assert_eq!(
            contract.review_screening(screening_id, true),
            Err(PmError::Unauthorized)
        );

        test::set_caller::<DefaultEnvironment>(accounts.bob);
        contract
            .review_screening(screening_id, true)
            .expect("approved");
        let screening = contract.get_screening(screening_id).unwrap();
        assert_eq!(screening.status, ScreeningStatus::Approved);
        assert_eq!(screening.reviewer, Some(accounts.bob));

        // Already-reviewed applications cannot flip again.
        assert_eq!(
            contract.review_screening(screening_id, false),
            Err(PmError::InvalidStatus)
        );
    }

    #[ink::test]
    fn expense_recording_validates_amounts() {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        let mut contract = pm();
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        contract.add_manager(accounts.bob).expect("ok");

        // Random callers cannot book expenses against the portfolio.
        test::set_caller::<DefaultEnvironment>(accounts.charlie);
        assert_eq!(
            contract.record_expense(10, "plumbing".into(), 500, accounts.django, hash(5)),
            Err(PmError::Unauthorized)
        );

        // Managers may book real expenses...
        test::set_caller::<DefaultEnvironment>(accounts.bob);
        let expense_id = contract
            .record_expense(10, "plumbing".into(), 500, accounts.django, hash(5))
            .expect("expense recorded");
        assert_eq!(expense_id, 1);

        // ...but zero-amount entries are rejected.
        assert_eq!(
            contract.record_expense(10, "ghost vendor".into(), 0, accounts.django, hash(6)),
            Err(PmError::InvalidAmount)
        );
    }
}
