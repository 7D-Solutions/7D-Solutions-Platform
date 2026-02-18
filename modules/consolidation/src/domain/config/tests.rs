//! Integrated tests for consolidation config — real DB, no mocks.

#[cfg(test)]
mod tests {
    use crate::domain::config::{models::*, service, service_rules, ConfigError};
    use serial_test::serial;
    use sqlx::PgPool;

    const TEST_TENANT: &str = "test-csl-config";

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://consolidation_user:consolidation_pass@localhost:5446/consolidation_db"
                .to_string()
        })
    }

    async fn test_pool() -> PgPool {
        PgPool::connect(&test_db_url()).await.expect("Failed to connect to consolidation test DB")
    }

    async fn cleanup(pool: &PgPool) {
        for table in &["csl_fx_policies", "csl_elimination_rules", "csl_coa_mappings", "csl_group_entities"] {
            sqlx::query(&format!(
                "DELETE FROM {} WHERE group_id IN (SELECT id FROM csl_groups WHERE tenant_id = $1)",
                table
            ))
            .bind(TEST_TENANT).execute(pool).await.ok();
        }
        sqlx::query("DELETE FROM csl_groups WHERE tenant_id = $1")
            .bind(TEST_TENANT).execute(pool).await.ok();
    }

    fn group_req(name: &str) -> CreateGroupRequest {
        CreateGroupRequest {
            name: name.to_string(),
            description: Some("Test group".to_string()),
            reporting_currency: "USD".to_string(),
            fiscal_year_end_month: Some(12),
        }
    }

    fn entity_req(tenant: &str, name: &str, currency: &str) -> CreateEntityRequest {
        CreateEntityRequest {
            entity_tenant_id: tenant.to_string(),
            entity_name: name.to_string(),
            functional_currency: currency.to_string(),
            ownership_pct_bp: Some(10000),
            consolidation_method: Some("full".to_string()),
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_group_crud() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let group = service::create_group(&pool, TEST_TENANT, &group_req("Acme Holdings"))
            .await.unwrap();
        assert_eq!(group.name, "Acme Holdings");
        assert_eq!(group.tenant_id, TEST_TENANT);
        assert_eq!(group.reporting_currency, "USD");
        assert!(group.is_active);

        let groups = service::list_groups(&pool, TEST_TENANT, false).await.unwrap();
        assert_eq!(groups.len(), 1);

        let fetched = service::get_group(&pool, TEST_TENANT, group.id).await.unwrap();
        assert_eq!(fetched.id, group.id);

        let updated = service::update_group(&pool, TEST_TENANT, group.id, &UpdateGroupRequest {
            name: Some("Acme Global".to_string()),
            description: None, reporting_currency: None,
            fiscal_year_end_month: None, is_active: None,
        }).await.unwrap();
        assert_eq!(updated.name, "Acme Global");

        service::delete_group(&pool, TEST_TENANT, group.id).await.unwrap();
        assert!(service::list_groups(&pool, TEST_TENANT, true).await.unwrap().is_empty());
        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_group_duplicate_name_rejected() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        service::create_group(&pool, TEST_TENANT, &group_req("Dup Group")).await.unwrap();
        let err = service::create_group(&pool, TEST_TENANT, &group_req("Dup Group"))
            .await.unwrap_err();
        assert!(matches!(err, ConfigError::Conflict(_)));
        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_group_validation() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let blank = CreateGroupRequest {
            name: "  ".into(), description: None,
            reporting_currency: "USD".into(), fiscal_year_end_month: None,
        };
        assert!(matches!(service::create_group(&pool, TEST_TENANT, &blank).await,
            Err(ConfigError::Validation(_))));

        let bad_cur = CreateGroupRequest {
            name: "Test".into(), description: None,
            reporting_currency: "X".into(), fiscal_year_end_month: None,
        };
        assert!(matches!(service::create_group(&pool, TEST_TENANT, &bad_cur).await,
            Err(ConfigError::Validation(_))));
        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_entity_crud() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let group = service::create_group(&pool, TEST_TENANT, &group_req("Entity Test"))
            .await.unwrap();
        let entity = service::create_entity(
            &pool, TEST_TENANT, group.id, &entity_req("sub-us", "US Sub", "USD"),
        ).await.unwrap();
        assert_eq!(entity.entity_name, "US Sub");
        assert_eq!(entity.ownership_pct_bp, 10000);

        assert_eq!(service::list_entities(&pool, TEST_TENANT, group.id, false)
            .await.unwrap().len(), 1);

        let updated = service::update_entity(&pool, TEST_TENANT, entity.id,
            &UpdateEntityRequest {
                entity_name: Some("US Subsidiary".into()), functional_currency: None,
                ownership_pct_bp: Some(8000), consolidation_method: None, is_active: None,
            }).await.unwrap();
        assert_eq!(updated.entity_name, "US Subsidiary");
        assert_eq!(updated.ownership_pct_bp, 8000);

        service::delete_entity(&pool, TEST_TENANT, entity.id).await.unwrap();
        assert!(service::list_entities(&pool, TEST_TENANT, group.id, true)
            .await.unwrap().is_empty());
        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_coa_mapping_crud() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let group = service::create_group(&pool, TEST_TENANT, &group_req("COA Test"))
            .await.unwrap();
        let mapping = service::create_coa_mapping(&pool, TEST_TENANT, group.id,
            &CreateCoaMappingRequest {
                entity_tenant_id: "sub-us".into(), source_account_code: "1010".into(),
                target_account_code: "CASH-001".into(),
                target_account_name: Some("Consolidated Cash".into()),
            }).await.unwrap();
        assert_eq!(mapping.source_account_code, "1010");

        assert_eq!(service::list_coa_mappings(&pool, TEST_TENANT, group.id, None)
            .await.unwrap().len(), 1);
        assert_eq!(service::list_coa_mappings(&pool, TEST_TENANT, group.id, Some("sub-us"))
            .await.unwrap().len(), 1);

        // Duplicate rejected
        let dup_err = service::create_coa_mapping(&pool, TEST_TENANT, group.id,
            &CreateCoaMappingRequest {
                entity_tenant_id: "sub-us".into(), source_account_code: "1010".into(),
                target_account_code: "OTHER".into(), target_account_name: None,
            }).await.unwrap_err();
        assert!(matches!(dup_err, ConfigError::Conflict(_)));

        service::delete_coa_mapping(&pool, TEST_TENANT, mapping.id).await.unwrap();
        assert!(service::list_coa_mappings(&pool, TEST_TENANT, group.id, None)
            .await.unwrap().is_empty());
        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_elimination_rule_crud() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let group = service::create_group(&pool, TEST_TENANT, &group_req("Elim Test"))
            .await.unwrap();
        let rule = service_rules::create_elimination_rule(&pool, TEST_TENANT, group.id,
            &CreateEliminationRuleRequest {
                rule_name: "IC Revenue".into(),
                rule_type: "intercompany_revenue_cost".into(),
                debit_account_code: "REV-IC".into(),
                credit_account_code: "COGS-IC".into(),
                description: Some("Eliminate IC revenue/cost".into()),
            }).await.unwrap();
        assert_eq!(rule.rule_name, "IC Revenue");

        assert_eq!(service_rules::list_elimination_rules(&pool, TEST_TENANT, group.id, false)
            .await.unwrap().len(), 1);

        let updated = service_rules::update_elimination_rule(&pool, TEST_TENANT, rule.id,
            &UpdateEliminationRuleRequest {
                rule_name: None, rule_type: None,
                debit_account_code: Some("REV-IC-2".into()),
                credit_account_code: None, description: None, is_active: None,
            }).await.unwrap();
        assert_eq!(updated.debit_account_code, "REV-IC-2");

        service_rules::delete_elimination_rule(&pool, TEST_TENANT, rule.id).await.unwrap();
        assert!(service_rules::list_elimination_rules(&pool, TEST_TENANT, group.id, true)
            .await.unwrap().is_empty());
        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_fx_policy_upsert() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let group = service::create_group(&pool, TEST_TENANT, &group_req("FX Test"))
            .await.unwrap();
        let policy = service_rules::upsert_fx_policy(&pool, TEST_TENANT, group.id,
            &UpsertFxPolicyRequest {
                entity_tenant_id: "sub-uk".into(),
                bs_rate_type: Some("closing".into()), pl_rate_type: Some("average".into()),
                equity_rate_type: Some("historical".into()), fx_rate_source: None,
            }).await.unwrap();
        assert_eq!(policy.bs_rate_type, "closing");
        assert_eq!(policy.fx_rate_source, "gl");

        // Upsert updates existing
        let updated = service_rules::upsert_fx_policy(&pool, TEST_TENANT, group.id,
            &UpsertFxPolicyRequest {
                entity_tenant_id: "sub-uk".into(),
                bs_rate_type: Some("average".into()), pl_rate_type: None,
                equity_rate_type: None, fx_rate_source: None,
            }).await.unwrap();
        assert_eq!(updated.bs_rate_type, "average");

        assert_eq!(service_rules::list_fx_policies(&pool, TEST_TENANT, group.id)
            .await.unwrap().len(), 1);

        service_rules::delete_fx_policy(&pool, TEST_TENANT, updated.id).await.unwrap();
        assert!(service_rules::list_fx_policies(&pool, TEST_TENANT, group.id)
            .await.unwrap().is_empty());
        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_validation_incomplete_then_complete() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let group = service::create_group(&pool, TEST_TENANT, &group_req("Validate Test"))
            .await.unwrap();
        service::create_entity(&pool, TEST_TENANT, group.id,
            &entity_req("sub-uk", "UK Sub", "GBP")).await.unwrap();
        service::create_entity(&pool, TEST_TENANT, group.id,
            &entity_req("sub-us", "US Sub", "USD")).await.unwrap();

        // Both missing COA, UK missing FX
        let result = service::validate_group_completeness(&pool, TEST_TENANT, group.id)
            .await.unwrap();
        assert!(!result.is_complete);
        assert_eq!(result.missing_coa_mappings.len(), 2);
        assert_eq!(result.missing_fx_policies.len(), 1);
        assert!(result.missing_fx_policies.contains(&"sub-uk".to_string()));

        // Add COA mappings
        for (eid, code) in [("sub-uk", "1010"), ("sub-us", "1010")] {
            service::create_coa_mapping(&pool, TEST_TENANT, group.id,
                &CreateCoaMappingRequest {
                    entity_tenant_id: eid.into(), source_account_code: code.into(),
                    target_account_code: "CASH".into(), target_account_name: None,
                }).await.unwrap();
        }
        // Add FX policy for UK
        service_rules::upsert_fx_policy(&pool, TEST_TENANT, group.id,
            &UpsertFxPolicyRequest {
                entity_tenant_id: "sub-uk".into(), bs_rate_type: None,
                pl_rate_type: None, equity_rate_type: None, fx_rate_source: None,
            }).await.unwrap();

        let result = service::validate_group_completeness(&pool, TEST_TENANT, group.id)
            .await.unwrap();
        assert!(result.is_complete);
        assert!(result.missing_coa_mappings.is_empty());
        assert!(result.missing_fx_policies.is_empty());
        cleanup(&pool).await;
    }
}
