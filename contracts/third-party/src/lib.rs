#![cfg_attr(not(feature = "std"), no_std)]
#![allow(unexpected_cfgs)]
#![allow(clippy::new_without_default)]

//! # PropChain Third-Party Service Integration
//!
//! Orchestrates interactions between PropChain contracts and external services:
//! - KYC/AML Providers (Identity verification, status checking)
//! - Fiat Payment Gateways (Bridging fiat payments to on-chain operations)
//! - Off-chain Monitoring and Alerting systems
//! - Service API endpoints and credential management
//!
//! Resolves: https://github.com/MettaChain/PropChain-contract/issues/113

use ink::prelude::string::String;
use ink::prelude::vec::Vec;
use ink::storage::Mapping;

#[ink::contract]
mod propchain_third_party {
    use super::*;

    // Data types extracted to types.rs (Issue #101)
    include!("types.rs");

    // Error types extracted to errors.rs (Issue #101)
    include!("errors.rs");

    // ========================================================================
    // EVENTS
    // ========================================================================

    #[ink(event)]
    pub struct ServiceRegistered {
        #[ink(topic)]
        service_id: ServiceId,
        service_type: ServiceType,
        name: String,
        provider_account: AccountId,
    }

    #[ink(event)]
    pub struct ServiceStatusChanged {
        #[ink(topic)]
        service_id: ServiceId,
        old_status: ServiceStatus,
        new_status: ServiceStatus,
    }

    #[ink(event)]
    pub struct KycRequestInitiated {
        #[ink(topic)]
        request_id: RequestId,
        #[ink(topic)]
        user: AccountId,
        service_id: ServiceId,
    }

    #[ink(event)]
    pub struct KycStatusUpdated {
        #[ink(topic)]
        request_id: RequestId,
        #[ink(topic)]
        user: AccountId,
        status: RequestStatus,
        verification_level: u8,
    }

    #[ink(event)]
    pub struct PaymentInitiated {
        #[ink(topic)]
        request_id: RequestId,
        #[ink(topic)]
        payer: AccountId,
        service_id: ServiceId,
        fiat_amount: u128,
        currency: String,
    }

    #[ink(event)]
    pub struct PaymentCompleted {
        #[ink(topic)]
        request_id: RequestId,
        status: RequestStatus,
        equivalent_tokens: u128,
    }

    #[ink(event)]
    pub struct MonitoringAlert {
        #[ink(topic)]
        service_id: ServiceId,
        #[ink(topic)]
        severity: u8,
        message: String,
        timestamp: u64,
    }

    // ========================================================================
    // CONTRACT STORAGE
    // ========================================================================

    #[ink(storage)]
    pub struct ThirdPartyIntegration {
        /// Contract admin
        admin: AccountId,
        /// Registered services
        services: Mapping<ServiceId, ServiceConfig>,
        /// Number of services
        service_counter: ServiceId,
        /// Provider account to service ID mapped
        provider_services: Mapping<AccountId, Vec<ServiceId>>,

        /// KYC records (User -> Record)
        kyc_records: Mapping<AccountId, KycRecord>,
        /// KYC requests
        kyc_requests: Mapping<RequestId, KycRequest>,

        /// Payment requests
        payment_requests: Mapping<RequestId, PaymentRequest>,

        /// Request counter
        request_counter: RequestId,
    }

    // ========================================================================
    // IMPLEMENTATION
    // ========================================================================

    impl ThirdPartyIntegration {
        #[ink(constructor)]
        pub fn new() -> Self {
            let caller = Self::env().caller();
            Self {
                admin: caller,
                services: Mapping::default(),
                service_counter: 0,
                provider_services: Mapping::default(),
                kyc_records: Mapping::default(),
                kyc_requests: Mapping::default(),
                payment_requests: Mapping::default(),
                request_counter: 0,
            }
        }

        // ====================================================================
        // SERVICE MANAGEMENT
        // ====================================================================

        /// Register a new third-party service (Admin only)
        #[ink(message)]
        pub fn register_service(
            &mut self,
            service_type: ServiceType,
            name: String,
            provider_account: AccountId,
            endpoint_url: String,
            api_version: String,
            fee_percentage: u16,
        ) -> Result<ServiceId, Error> {
            self.ensure_admin()?;

            if fee_percentage > 10000 {
                return Err(Error::InvalidFeePercentage);
            }

            self.service_counter += 1;
            let service_id = self.service_counter;

            let config = ServiceConfig {
                service_id,
                service_type: service_type.clone(),
                name: name.clone(),
                provider_account,
                endpoint_url,
                api_version,
                status: ServiceStatus::Active,
                registered_at: self.env().block_timestamp(),
                fees_collected: 0,
                fee_percentage,
            };

            self.services.insert(service_id, &config);

            let mut provider_list = self
                .provider_services
                .get(provider_account)
                .unwrap_or_default();
            provider_list.push(service_id);
            self.provider_services
                .insert(provider_account, &provider_list);

            self.env().emit_event(ServiceRegistered {
                service_id,
                service_type,
                name,
                provider_account,
            });

            Ok(service_id)
        }

        /// Update service status (Admin or Provider)
        #[ink(message)]
        pub fn update_service_status(
            &mut self,
            service_id: ServiceId,
            new_status: ServiceStatus,
        ) -> Result<(), Error> {
            let caller = self.env().caller();
            let mut service = self.get_service_mut(service_id)?;

            if caller != self.admin && caller != service.provider_account {
                return Err(Error::Unauthorized);
            }

            let old_status = service.status.clone();
            service.status = new_status.clone();
            self.services.insert(service_id, &service);

            self.env().emit_event(ServiceStatusChanged {
                service_id,
                old_status,
                new_status,
            });

            Ok(())
        }

        // ====================================================================
        // KYC INTEGRATION
        // ====================================================================

        /// Initiate KYC request (User or Admin)
        #[ink(message)]
        pub fn initiate_kyc_request(
            &mut self,
            service_id: ServiceId,
            user: AccountId,
            reference_id: String,
        ) -> Result<RequestId, Error> {
            let caller = self.env().caller();
            if caller != user && caller != self.admin {
                return Err(Error::Unauthorized);
            }

            self.ensure_service_active(service_id, ServiceType::KycProvider)?;

            self.request_counter += 1;
            let request_id = self.request_counter;

            let req = KycRequest {
                request_id,
                user,
                service_id,
                reference_id,
                status: RequestStatus::Pending,
                initiated_at: self.env().block_timestamp(),
                updated_at: self.env().block_timestamp(),
                expiry_date: None,
            };

            self.kyc_requests.insert(request_id, &req);

            self.env().emit_event(KycRequestInitiated {
                request_id,
                user,
                service_id,
            });

            Ok(request_id)
        }

        /// Update KYC status (Provider only)
        #[ink(message)]
        pub fn update_kyc_status(
            &mut self,
            request_id: RequestId,
            status: RequestStatus,
            verification_level: u8,
            valid_for_days: u64,
        ) -> Result<(), Error> {
            let caller = self.env().caller();

            let mut req = self
                .kyc_requests
                .get(request_id)
                .ok_or(Error::RequestNotFound)?;
            let service = self.get_service(req.service_id)?;

            if caller != service.provider_account {
                return Err(Error::Unauthorized);
            }

            // Only update active statuses
            if req.status == RequestStatus::Approved || req.status == RequestStatus::Rejected {
                return Err(Error::InvalidStatusTransition);
            }

            let timestamp = self.env().block_timestamp();
            req.status = status.clone();
            req.updated_at = timestamp;

            if status == RequestStatus::Approved {
                let expires_at = timestamp + (valid_for_days * 86_400_000);
                req.expiry_date = Some(expires_at);

                let record = KycRecord {
                    user: req.user,
                    provider_id: req.service_id,
                    verification_level,
                    verified_at: timestamp,
                    expires_at,
                    is_active: true,
                };
                self.kyc_records.insert(req.user, &record);
            }

            self.kyc_requests.insert(request_id, &req);

            self.env().emit_event(KycStatusUpdated {
                request_id,
                user: req.user,
                status,
                verification_level,
            });

            Ok(())
        }

        /// Check if a user is KYC verified (view function for other contracts)
        #[ink(message)]
        pub fn is_kyc_verified(&self, user: AccountId, required_level: u8) -> bool {
            if let Some(record) = self.kyc_records.get(user) {
                if record.is_active
                    && record.verification_level >= required_level
                    && record.expires_at > self.env().block_timestamp()
                {
                    return true;
                }
            }
            false
        }

        // ====================================================================
        // FIAT PAYMENT GATEWAY INTEGRATION
        // ====================================================================

        /// Initiate fiat payment bridging
        #[ink(message)]
        pub fn initiate_fiat_payment(
            &mut self,
            service_id: ServiceId,
            target_contract: AccountId,
            operation_type: u8,
            fiat_amount: u128,
            fiat_currency: String,
            payment_reference: String,
        ) -> Result<RequestId, Error> {
            let payer = self.env().caller();
            self.ensure_service_active(service_id, ServiceType::PaymentGateway)?;

            self.request_counter += 1;
            let request_id = self.request_counter;

            let req = PaymentRequest {
                request_id,
                payer,
                service_id,
                target_contract,
                operation_type,
                fiat_amount,
                fiat_currency: fiat_currency.clone(),
                equivalent_tokens: 0,
                payment_reference,
                status: RequestStatus::Pending,
                init_time: self.env().block_timestamp(),
                complete_time: None,
            };

            self.payment_requests.insert(request_id, &req);

            self.env().emit_event(PaymentInitiated {
                request_id,
                payer,
                service_id,
                fiat_amount,
                currency: fiat_currency,
            });

            Ok(request_id)
        }

        /// Complete fiat payment (Provider only)
        #[ink(message)]
        pub fn complete_payment(
            &mut self,
            request_id: RequestId,
            success: bool,
            equivalent_tokens: u128,
        ) -> Result<(), Error> {
            let caller = self.env().caller();

            let mut req = self
                .payment_requests
                .get(request_id)
                .ok_or(Error::RequestNotFound)?;
            let service = self.get_service(req.service_id)?;

            if caller != service.provider_account {
                return Err(Error::Unauthorized);
            }

            if req.status != RequestStatus::Pending && req.status != RequestStatus::Processing {
                return Err(Error::InvalidStatusTransition);
            }

            req.status = if success {
                RequestStatus::Approved
            } else {
                RequestStatus::Failed
            };
            req.equivalent_tokens = equivalent_tokens;
            req.complete_time = Some(self.env().block_timestamp());

            self.payment_requests.insert(request_id, &req);

            self.env().emit_event(PaymentCompleted {
                request_id,
                status: req.status,
                equivalent_tokens,
            });

            Ok(())
        }

        // ====================================================================
        // MONITORING & ALERTING
        // ====================================================================

        /// Log an alert from an external monitoring system
        #[ink(message)]
        pub fn log_alert(
            &mut self,
            service_id: ServiceId,
            severity: u8,
            message: String,
        ) -> Result<(), Error> {
            let caller = self.env().caller();
            let service = self.get_service(service_id)?;

            if caller != service.provider_account && service.service_type == ServiceType::Monitoring
            {
                return Err(Error::Unauthorized);
            }

            self.env().emit_event(MonitoringAlert {
                service_id,
                severity,
                message,
                timestamp: self.env().block_timestamp(),
            });

            Ok(())
        }

        // ====================================================================
        // QUERIES
        // ====================================================================

        #[ink(message)]
        pub fn get_service_config(&self, service_id: ServiceId) -> Option<ServiceConfig> {
            self.services.get(service_id)
        }

        #[ink(message)]
        pub fn get_kyc_record(&self, user: AccountId) -> Option<KycRecord> {
            self.kyc_records.get(user)
        }

        #[ink(message)]
        pub fn get_payment_request(&self, request_id: RequestId) -> Option<PaymentRequest> {
            self.payment_requests.get(request_id)
        }

        // ====================================================================
        // INTERNAL
        // ====================================================================

        fn ensure_admin(&self) -> Result<(), Error> {
            if self.env().caller() != self.admin {
                return Err(Error::Unauthorized);
            }
            Ok(())
        }

        fn get_service(&self, service_id: ServiceId) -> Result<ServiceConfig, Error> {
            self.services.get(service_id).ok_or(Error::ServiceNotFound)
        }

        fn get_service_mut(&self, service_id: ServiceId) -> Result<ServiceConfig, Error> {
            self.services.get(service_id).ok_or(Error::ServiceNotFound)
        }

        fn ensure_service_active(
            &self,
            service_id: ServiceId,
            expected_type: ServiceType,
        ) -> Result<(), Error> {
            let service = self.get_service(service_id)?;
            if service.status != ServiceStatus::Active {
                return Err(Error::ServiceInactive);
            }
            if service.service_type != expected_type {
                return Err(Error::ServiceNotFound);
            }
            Ok(())
        }
    }

    impl Default for ThirdPartyIntegration {
        fn default() -> Self {
            Self::new()
        }
    }

    // ========================================================================
    // UNIT TESTS
    // ========================================================================

    #[cfg(test)]
    mod tests {
        use ink::env::{test, DefaultEnvironment};

        use super::*;

        fn setup() -> ThirdPartyIntegration {
            test::set_caller::<DefaultEnvironment>(
                test::default_accounts::<DefaultEnvironment>().alice,
            );
            ThirdPartyIntegration::new()
        }

        fn register_kyc_provider(contract: &mut ThirdPartyIntegration, provider: AccountId) -> u32 {
            contract
                .register_service(
                    ServiceType::KycProvider,
                    "KYC Partner".into(),
                    provider,
                    "https://kyc.example".into(),
                    "v1".into(),
                    100,
                )
                .unwrap()
        }

        #[ink::test]
        fn test_register_service_admin_only_and_validates_fee() {
            let accounts = test::default_accounts::<DefaultEnvironment>();
            let mut contract = setup();

            // Non-admin registration is rejected
            test::set_caller::<DefaultEnvironment>(accounts.bob);
            assert_eq!(
                contract.register_service(
                    ServiceType::KycProvider,
                    "Rogue".into(),
                    accounts.bob,
                    "https://evil.example".into(),
                    "v1".into(),
                    0,
                ),
                Err(Error::Unauthorized)
            );

            // Admin can register and the config is queryable
            test::set_caller::<DefaultEnvironment>(accounts.alice);
            let service_id = register_kyc_provider(&mut contract, accounts.charlie);
            assert_eq!(service_id, 1);
            let config = contract.get_service_config(service_id).unwrap();
            assert_eq!(config.service_type, ServiceType::KycProvider);
            assert_eq!(config.provider_account, accounts.charlie);
            assert_eq!(config.status, ServiceStatus::Active);
            assert_eq!(config.fee_percentage, 100);

            // Fee percentage above the 10000 bps cap is rejected
            assert_eq!(
                contract.register_service(
                    ServiceType::Other,
                    "Greedy".into(),
                    accounts.bob,
                    "https://x.example".into(),
                    "v1".into(),
                    10_001,
                ),
                Err(Error::InvalidFeePercentage)
            );
        }

        #[ink::test]
        fn test_update_service_status_admin_or_provider_only() {
            let accounts = test::default_accounts::<DefaultEnvironment>();
            let mut contract = setup();
            let service_id = register_kyc_provider(&mut contract, accounts.charlie);

            // A random account may not change the status
            test::set_caller::<DefaultEnvironment>(accounts.eve);
            assert_eq!(
                contract.update_service_status(service_id, ServiceStatus::Suspended),
                Err(Error::Unauthorized)
            );

            // The provider itself can suspend its own service
            test::set_caller::<DefaultEnvironment>(accounts.charlie);
            assert_eq!(
                contract.update_service_status(service_id, ServiceStatus::Suspended),
                Ok(())
            );
            let suspended = contract.get_service_config(service_id).unwrap();
            assert_eq!(suspended.status, ServiceStatus::Suspended);

            // Requests against a suspended service are rejected
            test::set_caller::<DefaultEnvironment>(accounts.bob);
            assert_eq!(
                contract.initiate_kyc_request(service_id, accounts.bob, "ref-1".into()),
                Err(Error::ServiceInactive)
            );
        }

        #[ink::test]
        fn test_kyc_flow_updates_record_and_rejects_double_update() {
            let accounts = test::default_accounts::<DefaultEnvironment>();
            let mut contract = setup();
            let service_id = register_kyc_provider(&mut contract, accounts.charlie);

            // Only the user itself (or admin) may initiate a request
            test::set_caller::<DefaultEnvironment>(accounts.eve);
            assert_eq!(
                contract.initiate_kyc_request(service_id, accounts.bob, "ref-2".into()),
                Err(Error::Unauthorized)
            );

            test::set_caller::<DefaultEnvironment>(accounts.bob);
            let request_id = contract
                .initiate_kyc_request(service_id, accounts.bob, "ref-3".into())
                .unwrap();

            // Only the provider may resolve the request
            test::set_caller::<DefaultEnvironment>(accounts.eve);
            assert_eq!(
                contract.update_kyc_status(request_id, RequestStatus::Approved, 3, 30),
                Err(Error::Unauthorized)
            );

            test::set_caller::<DefaultEnvironment>(accounts.charlie);
            assert_eq!(
                contract.update_kyc_status(request_id, RequestStatus::Approved, 3, 30),
                Ok(())
            );

            // Verification level gates hold; unknown users are unverified
            assert!(contract.is_kyc_verified(accounts.bob, 3));
            assert!(!contract.is_kyc_verified(accounts.bob, 4));
            assert!(!contract.is_kyc_verified(accounts.eve, 1));

            // An already-resolved request cannot be updated again
            assert_eq!(
                contract.update_kyc_status(request_id, RequestStatus::Rejected, 0, 0),
                Err(Error::InvalidStatusTransition)
            );
        }

        #[ink::test]
        fn test_payment_flow_completes_by_provider() {
            let accounts = test::default_accounts::<DefaultEnvironment>();
            let mut contract = setup();
            test::set_caller::<DefaultEnvironment>(accounts.alice);
            let payment_service = contract
                .register_service(
                    ServiceType::PaymentGateway,
                    "Fiat Bridge".into(),
                    accounts.charlie,
                    "https://pay.example".into(),
                    "v1".into(),
                    50,
                )
                .unwrap();

            test::set_caller::<DefaultEnvironment>(accounts.bob);
            let request_id = contract
                .initiate_fiat_payment(
                    payment_service,
                    accounts.alice,
                    1,
                    1_000,
                    "USD".into(),
                    "invoice-42".into(),
                )
                .unwrap();

            // A non-provider cannot complete the payment
            test::set_caller::<DefaultEnvironment>(accounts.eve);
            assert_eq!(
                contract.complete_payment(request_id, true, 500),
                Err(Error::Unauthorized)
            );

            // The provider completes it with the token equivalence recorded
            test::set_caller::<DefaultEnvironment>(accounts.charlie);
            assert_eq!(contract.complete_payment(request_id, true, 500), Ok(()));
            let completed = contract.get_payment_request(request_id).unwrap();
            assert_eq!(completed.status, RequestStatus::Approved);
            assert_eq!(completed.equivalent_tokens, 500);
        }
    }
}
