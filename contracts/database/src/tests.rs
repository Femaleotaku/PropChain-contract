// Unit tests for the database contract (Issue #101 - extracted from lib.rs)

#[cfg(test)]
mod tests {
    use super::*;

    #[ink::test]
    fn new_initializes_correctly() {
        let contract = DatabaseIntegration::new();
        assert_eq!(contract.total_syncs(), 0);
        assert_eq!(contract.latest_snapshot_id(), 0);
    }

    #[ink::test]
    fn emit_sync_event_works() {
        let mut contract = DatabaseIntegration::new();
        let result = contract.emit_sync_event(DataType::Properties, Hash::from([0x01; 32]), 10);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1);
        assert_eq!(contract.total_syncs(), 1);

        let record = contract.get_sync_record(1).unwrap();
        assert_eq!(record.data_type, DataType::Properties);
        assert_eq!(record.record_count, 10);
        assert_eq!(record.status, SyncStatus::Initiated);
    }

    #[ink::test]
    fn analytics_snapshot_works() {
        let mut contract = DatabaseIntegration::new();
        let result = contract.record_analytics_snapshot(
            100,
            50,
            20,
            10_000_000,
            100_000,
            30,
            Hash::from([0x02; 32]),
        );
        assert!(result.is_ok());

        let snapshot = contract.get_analytics_snapshot(1).unwrap();
        assert_eq!(snapshot.total_properties, 100);
        assert_eq!(snapshot.total_valuation, 10_000_000);
    }

    #[ink::test]
    fn data_export_works() {
        let mut contract = DatabaseIntegration::new();
        let result = contract.request_data_export(DataType::Properties, 1, 100, 0, 1000);
        assert!(result.is_ok());

        let batch_id = result.unwrap();
        let request = contract.get_export_request(batch_id).unwrap();
        assert!(!request.completed);

        let complete_result = contract.complete_data_export(batch_id, Hash::from([0x03; 32]));
        assert!(complete_result.is_ok());

        let completed = contract.get_export_request(batch_id).unwrap();
        assert!(completed.completed);
    }

    #[ink::test]
    fn verify_sync_works() {
        let mut contract = DatabaseIntegration::new();
        let checksum = Hash::from([0x01; 32]);
        contract
            .emit_sync_event(DataType::Transfers, checksum, 5)
            .unwrap();

        let result = contract.verify_sync(1, checksum);
        assert_eq!(result, Ok(true));

        let record = contract.get_sync_record(1).unwrap();
        assert_eq!(record.status, SyncStatus::Verified);
    }

    #[ink::test]
    fn indexer_registration_works() {
        let mut contract = DatabaseIntegration::new();
        let indexer = AccountId::from([0x02; 32]);

        let result = contract.register_indexer(indexer, String::from("TestIndexer"));
        assert!(result.is_ok());

        let info = contract.get_indexer(indexer).unwrap();
        assert_eq!(info.name, "TestIndexer");
        assert!(info.is_active);

        let list = contract.get_indexer_list();
        assert_eq!(list.len(), 1);
    }

    // ========================================================================
    // Export-request lifecycle (Issue #1016)
    // ========================================================================

    fn new_admin_contract() -> DatabaseIntegration {
        let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
        ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.alice);
        DatabaseIntegration::new()
    }

    #[ink::test]
    fn request_data_export_is_admin_only() {
        let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
        let mut contract = new_admin_contract();

        // Non-admin is rejected
        ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.bob);
        let forbidden = contract.request_data_export(DataType::Properties, 1, 100, 0, 1000);
        assert_eq!(forbidden, Err(Error::Unauthorized));

        // Admin succeeds
        ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.alice);
        let batch = contract
            .request_data_export(DataType::Properties, 1, 100, 0, 1000)
            .expect("Admin should request an export");
        assert_eq!(batch, 1);
    }

    #[ink::test]
    fn inverted_id_range_with_equal_blocks_is_rejected() {
        let mut contract = new_admin_contract();
        let result = contract.request_data_export(DataType::Transfers, 100, 1, 500, 500);
        assert_eq!(result, Err(Error::InvalidDataRange));
    }

    #[ink::test]
    fn inverted_block_range_with_equal_ids_is_rejected() {
        let mut contract = new_admin_contract();
        let result = contract.request_data_export(DataType::Escrows, 42, 42, 900, 100);
        assert_eq!(result, Err(Error::InvalidDataRange));
    }

    #[ink::test]
    fn fully_inverted_ranges_are_rejected() {
        let mut contract = new_admin_contract();
        let result = contract.request_data_export(DataType::Valuations, 100, 1, 900, 100);
        assert_eq!(result, Err(Error::InvalidDataRange));
    }

    #[ink::test]
    fn equal_boundaries_are_accepted_as_valid_range() {
        let mut contract = new_admin_contract();
        let result =
            contract.request_data_export(DataType::Compliance, 7, 7, 1234, 1234);
        assert!(
            result.is_ok(),
            "from == to on both axes must be a valid single-record range"
        );
    }

    #[ink::test]
    fn valid_request_stores_exact_fields_and_increments_batch_ids() {
        let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
        let mut contract = new_admin_contract();

        let first = contract
            .request_data_export(DataType::Properties, 10, 20, 100, 200)
            .expect("first export");
        let second = contract
            .request_data_export(DataType::Tokens, 30, 40, 300, 400)
            .expect("second export");

        assert_eq!(first, 1, "batch ids increment sequentially");
        assert_eq!(second, 2);

        let stored = contract.get_export_request(first).unwrap();
        assert_eq!(stored.batch_id, first);
        assert_eq!(stored.data_type, DataType::Properties);
        assert_eq!(stored.from_id, 10);
        assert_eq!(stored.to_id, 20);
        assert_eq!(stored.from_block, 100);
        assert_eq!(stored.to_block, 200);
        assert_eq!(stored.requested_by, accounts.alice);
        assert!(!stored.completed);
        assert_eq!(stored.export_checksum, None);
    }

    #[ink::test]
    fn completion_sets_completed_flag_and_checksum() {
        let checksum = Hash::from([0xAB; 32]);
        let mut contract = new_admin_contract();

        let batch = contract
            .request_data_export(DataType::Analytics, 1, 9, 5, 50)
            .expect("export request");
        assert!(!contract.get_export_request(batch).unwrap().completed);

        contract
            .complete_data_export(batch, checksum)
            .expect("Admin completes the export");

        let completed = contract.get_export_request(batch).unwrap();
        assert!(completed.completed);
        assert_eq!(completed.export_checksum, Some(checksum));
    }

    #[ink::test]
    fn completing_unknown_batch_returns_export_not_found() {
        let mut contract = new_admin_contract();
        let result = contract.complete_data_export(999, Hash::from([0x01; 32]));
        assert_eq!(result, Err(Error::ExportNotFound));
    }

    #[ink::test]
    fn get_export_request_returns_none_for_unknown_batch() {
        let contract = new_admin_contract();
        assert!(contract.get_export_request(12345).is_none());
    }
}
