/// # Integration Tests: Database Sync, Snapshots, and Export (Issue #925)
///
/// The database contract coordinates off-chain indexers: authorized
/// publishers emit sync events, registered indexers confirm them, the admin
/// records analytics snapshots, and data exports are requested and completed
/// with integrity checksums.
///
/// Acceptance criteria tested:
///   check Sync event lifecycle: emit -> confirm by indexer -> checksum verification
///   check Only the admin or an authorized publisher can emit sync events
///   check Only registered indexers can confirm syncs
///   check Analytics snapshots are admin-gated and retrievable by id
///   check Data export requests validate ranges and complete with checksums
///   check Indexer registration enforces admin rights and uniqueness
#[cfg(test)]
mod integration_database {
    use ink::env::{test, DefaultEnvironment};
    use ink::primitives::Hash;
    use propchain_database::propchain_database::{
        DataType, DatabaseIntegration, Error as DatabaseError, SyncStatus,
    };

    fn hash(byte: u8) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[0] = byte;
        Hash::from(bytes)
    }

    #[ink::test]
    fn sync_event_lifecycle_end_to_end() {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        let mut db = DatabaseIntegration::new();

        // Admin registers an indexer for the confirmation step.
        db.register_indexer(accounts.charlie, "main-indexer".into())
            .expect("admin registers indexer");
        test::set_block_number::<DefaultEnvironment>(42);

        // Admin emits a sync event for the properties dataset.
        let sync_id = db
            .emit_sync_event(DataType::Properties, hash(1), 10)
            .expect("admin may emit sync events");
        assert_eq!(sync_id, 1);
        assert_eq!(db.total_syncs(), 1);

        let record = db.get_sync_record(sync_id).expect("record stored");
        assert_eq!(record.status, SyncStatus::Initiated);
        assert_eq!(record.data_checksum, hash(1));
        assert_eq!(record.record_count, 10);
        assert_eq!(record.block_number, 42);
        assert_eq!(
            db.last_synced_block(DataType::Properties),
            42,
            "last synced block tracked per data type"
        );

        // A non-indexer cannot confirm the sync.
        test::set_caller::<DefaultEnvironment>(accounts.bob);
        assert_eq!(
            db.confirm_sync(sync_id),
            Err(DatabaseError::IndexerNotFound)
        );

        // The registered indexer confirms; its cursor advances.
        test::set_caller::<DefaultEnvironment>(accounts.charlie);
        db.confirm_sync(sync_id)
            .expect("registered indexer confirms");
        assert_eq!(
            db.get_sync_record(sync_id).unwrap().status,
            SyncStatus::Confirmed
        );
        assert_eq!(
            db.get_indexer(accounts.charlie).unwrap().last_synced_block,
            42
        );

        // Checksum verification accepts the original digest, rejects others.
        assert_eq!(db.verify_sync(sync_id, hash(1)), Ok(true));
        assert_eq!(db.verify_sync(sync_id, hash(2)), Ok(false));

        // Unknown sync ids are rejected.
        assert_eq!(db.confirm_sync(999), Err(DatabaseError::SyncNotFound));
    }

    #[ink::test]
    fn publisher_authorization_gates_sync_emission() {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        let mut db = DatabaseIntegration::new();

        // Unauthorized accounts cannot publish.
        test::set_caller::<DefaultEnvironment>(accounts.bob);
        assert_eq!(
            db.emit_sync_event(DataType::Transfers, hash(1), 1),
            Err(DatabaseError::Unauthorized)
        );

        // Admin authorizes bob as a data publisher.
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        db.authorize_publisher(accounts.bob)
            .expect("admin authorizes publisher");

        test::set_caller::<DefaultEnvironment>(accounts.bob);
        let sync_id = db
            .emit_sync_event(DataType::Transfers, hash(2), 5)
            .expect("authorized publisher may emit");
        assert_eq!(sync_id, 1);

        // Revocation cuts the publisher off again.
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        db.revoke_publisher(accounts.bob)
            .expect("admin revokes publisher");

        test::set_caller::<DefaultEnvironment>(accounts.bob);
        assert_eq!(
            db.emit_sync_event(DataType::Transfers, hash(3), 7),
            Err(DatabaseError::Unauthorized)
        );
    }

    #[ink::test]
    fn analytics_snapshots_are_admin_gated_and_retrievable() {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        let mut db = DatabaseIntegration::new();

        // Non-admin callers are rejected.
        test::set_caller::<DefaultEnvironment>(accounts.bob);
        assert_eq!(
            db.record_analytics_snapshot(1, 1, 1, 1, 1, 1, hash(9)),
            Err(DatabaseError::Unauthorized)
        );

        test::set_caller::<DefaultEnvironment>(accounts.alice);
        let first = db
            .record_analytics_snapshot(120, 340, 25, 9_000_000, 75_000, 88, hash(1))
            .expect("admin records snapshot");
        let second = db
            .record_analytics_snapshot(121, 341, 26, 9_100_000, 75_200, 90, hash(2))
            .expect("second snapshot recorded");
        assert_eq!(first, 1);
        assert_eq!(second, 2);
        assert_eq!(db.latest_snapshot_id(), 2);

        let snap = db.get_analytics_snapshot(first).expect("snapshot stored");
        assert_eq!(snap.total_properties, 120);
        assert_eq!(snap.total_valuation, 9_000_000);
        assert_eq!(snap.avg_valuation, 75_000);
        assert_eq!(snap.integrity_checksum, hash(1));
        assert_eq!(snap.created_by, accounts.alice);

        assert!(db.get_analytics_snapshot(999).is_none());
    }

    #[ink::test]
    fn data_export_request_and_completion() {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        let mut db = DatabaseIntegration::new();

        // Inverted ranges are rejected up front.
        assert_eq!(
            db.request_data_export(DataType::Properties, 10, 5, 0, 100),
            Err(DatabaseError::InvalidDataRange)
        );
        assert_eq!(
            db.request_data_export(DataType::Properties, 0, 10, 200, 100),
            Err(DatabaseError::InvalidDataRange)
        );

        // Non-admin callers cannot request exports.
        test::set_caller::<DefaultEnvironment>(accounts.bob);
        assert_eq!(
            db.request_data_export(DataType::Properties, 0, 10, 0, 100),
            Err(DatabaseError::Unauthorized)
        );

        test::set_caller::<DefaultEnvironment>(accounts.alice);
        let batch = db
            .request_data_export(DataType::Properties, 1, 500, 0, 1_000)
            .expect("admin requests export");
        assert_eq!(batch, 1);

        let request = db.get_export_request(batch).expect("export stored");
        assert_eq!(request.data_type, DataType::Properties);
        assert_eq!((request.from_id, request.to_id), (1, 500));
        assert_eq!((request.from_block, request.to_block), (0, 1_000));
        assert!(!request.completed);
        assert_eq!(request.export_checksum, None);

        // Completion stamps the export with an integrity checksum.
        db.complete_data_export(batch, hash(7))
            .expect("admin completes export");
        let completed = db.get_export_request(batch).unwrap();
        assert!(completed.completed);
        assert_eq!(completed.export_checksum, Some(hash(7)));

        // Completing an unknown batch fails cleanly.
        assert_eq!(
            db.complete_data_export(999, hash(8)),
            Err(DatabaseError::ExportNotFound)
        );
    }

    #[ink::test]
    fn indexer_registration_enforces_admin_and_uniqueness() {
        let accounts = test::default_accounts::<DefaultEnvironment>();
        test::set_caller::<DefaultEnvironment>(accounts.alice);
        let mut db = DatabaseIntegration::new();

        // Non-admin registration is rejected.
        test::set_caller::<DefaultEnvironment>(accounts.bob);
        assert_eq!(
            db.register_indexer(accounts.bob, "rogue".into()),
            Err(DatabaseError::Unauthorized)
        );

        test::set_caller::<DefaultEnvironment>(accounts.alice);
        db.register_indexer(accounts.charlie, "charlie-indexer".into())
            .expect("registers charlie");
        db.register_indexer(accounts.eve, "dave-indexer".into())
            .expect("registers dave");

        assert_eq!(
            db.register_indexer(accounts.charlie, "duplicate".into()),
            Err(DatabaseError::IndexerAlreadyRegistered),
            "double registration rejected"
        );

        assert_eq!(db.get_indexer_list().len(), 2);
        let info = db.get_indexer(accounts.charlie).expect("indexer found");
        assert_eq!(info.name, "charlie-indexer");
        assert!(info.is_active);

        // Deactivation flips the flag but keeps the registration.
        db.deactivate_indexer(accounts.charlie)
            .expect("admin deactivates indexer");
        assert!(!db.get_indexer(accounts.charlie).unwrap().is_active);

        assert_eq!(
            db.deactivate_indexer(accounts.bob),
            Err(DatabaseError::IndexerNotFound)
        );
    }
}
