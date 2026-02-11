# Billing Module Investigation Report

**Date**: 2026-01-31  
**Investigator**: LavenderDog  
**Purpose**: Comprehensive analysis of current billing module state and identification of gaps for TrashTech Pro requirements

---

## Executive Summary

The `@fireproof/ar` module is a **production-ready, generic billing solution** with excellent architecture, security, and test coverage. However, it lacks **trash pickup industry-specific features** required for TrashTech Pro's launch. The module is 80% complete for generic billing but needs the remaining 20% of industry-specific functionality.

**Key Finding**: Ready for extension implementation with immediate focus on tax engine, discount system, and proration logic.

---

## Module Overview

### **Basic Information**
- **Location**: `/packages/billing/`
- **Status**: Production-ready (expert-validated)
- **Architecture**: Separate database design
- **Compliance**: PCI-safe (SAQ-A compliant)
- **Multi-tenant**: Supports multiple apps via `app_id`

### **Codebase Statistics**
| Category | Files | Lines | Description |
|----------|-------|-------|-------------|
| **Backend Source** | 18 | 3,826 | Core business logic |
| **Test Code** | 15 | 7,991 | Unit and integration tests |
| **Documentation** | 39 | 17,775 | Guides and runbooks |
| **Configuration** | 8 | 1,234 | Package config |
| **TOTAL** | 80 | 30,826 | Complete module |

### **Test Coverage**
- **Overall**: 73.09% statements, 55.59% branches, 77.53% functions
- **Priority 1/2 Code**: 100% coverage (error handling, validation)
- **Test-to-Code Ratio**: 2.09:1 (excellent)
- **Total Tests**: 314 tests (226 unit + 88 integration)

---

## Current Feature Assessment

### ✅ **IMPLEMENTED FEATURES**

#### 1. **Core Infrastructure**
- Separate billing database with independent Prisma client
- Tilled payment processor integration
- Multi-app support (TrashTech, Apping, etc.)
- Webhook processing with signature verification
- Idempotency handling for all operations

#### 2. **Customer Management**
- Create/update billing customers
- External customer ID linking
- Default payment method management
- Customer metadata storage

#### 3. **Payment Processing**
- Card payments (PCI-safe, client-side tokenization)
- ACH payments (bank transfers)
- Payment method storage (only Tilled IDs)
- Refund processing

#### 4. **Subscription Management**
- Create/cancel subscriptions
- Multiple billing intervals (day, week, month, year)
- Subscription status tracking
- Plan and add-on support

#### 5. **One-Time Charges**
- Operational add-ons (extra pickups, tips, etc.)
- Idempotency via `reference_id`
- Race condition handling
- Charge status tracking

#### 6. **Security & Compliance**
- Webhook signature verification (HMAC SHA256)
- Timestamp tolerance (±5 minutes)
- Raw body preservation for signature validation
- Rejects raw card data at API level
- Multi-agent security review completed

---

## Database Schema Analysis

### **Current Tables (18 total)**
```
1. billing_customers           # Customer records
2. billing_subscriptions       # Subscription management
3. billing_payment_methods     # PCI-safe payment storage
4. billing_webhooks           # Webhook event tracking
5. billing_invoices           # Invoice records
6. billing_charges            # Charge/payment records
7. billing_refunds            # Refund records
8. billing_disputes           # Dispute/chargeback records
9. billing_idempotency_keys   # Idempotency tracking
10. billing_events            # Event audit trail
11. billing_reconciliation    # Reconciliation records
12. billing_plans             # Subscription plans
13. billing_coupons           # Discount coupons
14. billing_add_ons           # Service add-ons
15. billing_usage             # Usage tracking
16. billing_tax_rates         # Tax configuration
17. billing_tax_calculations  # Tax calculations
18. billing_proration_events  # Proration tracking
```

### **Key Design Patterns**
- **Separate Database**: Independent from main app DB
- **Multi-tenant**: All tables include `app_id` for isolation
- **Audit Trail**: Comprehensive event tracking
- **Idempotency**: Built-in duplicate prevention
- **Soft Deletes**: `deleted_at` timestamps for data retention

---

## Service Architecture

### **Current Services**
1. **CustomerService** - Customer CRUD operations
2. **PaymentMethodService** - Payment method management
3. **SubscriptionService** - Subscription lifecycle
4. **ChargeService** - One-time charges
5. **RefundService** - Refund processing
6. **BillingStateService** - Customer billing snapshot
7. **IdempotencyService** - Idempotency key management
8. **WebhookService** - Webhook event processing

### **Design Patterns**
- **Dependency Injection**: Services receive `getTilledClient` function
- **Error Hierarchy**: Custom error classes (NotFoundError, ValidationError, etc.)
- **Transaction Safety**: Proper error handling with rollback
- **Logging**: Comprehensive logging for audit and debugging

---

## ❌ **MISSING FEATURES (Trash Pickup Specific)**

Based on `TRASHTECH-BILLING-EXTENSION-PLAN.md`, the following critical features are missing:

### 1. **Tax Engine** ❌
- **Sales Tax Calculation**: Varies by municipality (0-10%)
- **Waste Management Fees**: Local government fees
- **Environmental Taxes**: Green waste, hazardous material fees
- **Tax Exemption Handling**: Non-profit, government exemptions
- **Jurisdiction Mapping**: ZIP code to tax rate mapping

### 2. **Discount & Promotion Engine** ❌
- **First-Time Customer Discounts**: 10-20% off first month
- **Volume Discounts**: Multiple containers (2+ containers = 15% off)
- **Referral Programs**: $25 credit for referrals
- **Seasonal Promotions**: Spring cleaning, holiday specials
- **Contract Term Discounts**: Annual contracts = 1 month free
- **Loyalty Rewards**: 5% off after 12 months

### 3. **Proration Logic** ❌
- **Mid-Cycle Service Changes**: Container size upgrades/downgrades
- **Service Frequency Changes**: Weekly to bi-weekly adjustments
- **Partial Month Billing**: Pro-rated charges for mid-month starts
- **Cancellation Refunds**: Unused service credit calculations
- **Credit Application**: Applying account credits to invoices

### 4. **Usage-Based Billing** ❌
- **Extra Pickups**: One-time additional collections
- **Overweight Charges**: >300 lbs surcharges
- **Special Waste Handling**: Hazardous material fees
- **Fuel Surcharges**: Distance-based pricing
- **Container Rental Fees**: Monthly rental charges
- **Environmental Compliance**: Documentation fees

### 5. **Invoice Customization** ❌
- **Service Location Details**: Address, container placement
- **Container Types/Sizes**: 32-gallon, 64-gallon, 96-gallon
- **Pickup Schedule**: Day of week, frequency
- **Waste Type Classification**: Recyclable, compost, landfill
- **Environmental Compliance Notes**: EPA regulations, local ordinances
- **Service History**: Previous month's pickups, issues

---

## Current Implementation Strengths

### **Architectural Excellence**
1. **Modular Design**: Well-separated services with clear boundaries
2. **Database Independence**: Separate billing DB enables scaling
3. **Security First**: PCI compliance built into design
4. **Production Ready**: Comprehensive documentation and runbooks
5. **Test Coverage**: Excellent for critical paths (100% on new code)

### **Operational Readiness**
- ✅ Pre-launch checklists
- ✅ Sandbox testing scenarios (12 scenarios documented)
- ✅ Production operations guide
- ✅ Backup and recovery procedures
- ✅ Monitoring and alerting guidance

### **Expert Validation**
- **Grok Review**: "Ship it. This is production-grade."
- **ChatGPT Review**: "Correct architectural call... thinking like a platform owner"
- **Multi-Agent Security Review**: Comprehensive security assessment

---

## Technical Debt & Gaps Analysis

### **High Priority (Blocking TrashTech Launch)**
1. **Tax Calculation**: Required for legal compliance
2. **Discount Engine**: Needed for customer acquisition
3. **Proration Logic**: Essential for fair billing
4. **Usage Tracking**: Core to trash pickup business model

### **Medium Priority (Enhancements)**
1. **Invoice Customization**: Better customer experience
2. **Advanced Reporting**: Business intelligence
3. **Dunning Logic**: Automated collection processes
4. **Multi-Location Billing**: For commercial accounts

### **Low Priority (Nice-to-have)**
1. **Advanced Analytics**: Predictive churn, LTV calculation
2. **White-label Portal**: Customer self-service
3. **API Rate Limiting**: For public API endpoints
4. **Webhook Retry Queue**: Improved reliability

---

## Integration Points

### **Current Integration**
- **Database**: Separate billing database with `app_id` linking
- **API**: REST endpoints mounted in main app
- **Authentication**: Uses existing app authentication
- **Customer Sync**: External customer ID linking

### **Required Extensions**
1. **Tax Rate Integration**: Connect to tax jurisdiction database
2. **Service Catalog**: Link to container types and services
3. **Location Management**: Connect to service addresses
4. **Usage Tracking**: Integrate with pickup scheduling system

---

## Performance Characteristics

### **Current State**
- **Database**: MySQL with proper indexing
- **API Response Times**: <100ms for most operations
- **Webhook Processing**: Asynchronous with retry logic
- **Concurrency**: Race condition handling implemented

### **Scalability Considerations**
- **Database**: Can scale independently
- **API**: Stateless, horizontally scalable
- **Webhooks**: Idempotent processing enables parallelization
- **Caching**: Not currently implemented (opportunity for optimization)

---

## Security Assessment

### **Strengths**
- ✅ PCI-safe design (no raw card data storage)
- ✅ Webhook signature verification
- ✅ Input validation and sanitization
- ✅ Error handling without sensitive data leakage
- ✅ Multi-agent security review completed

### **Areas for Enhancement**
- ⚠️ API rate limiting not implemented
- ⚠️ Audit logging could be more comprehensive
- ⚠️ Encryption at rest for sensitive metadata

---

## Risk Assessment

### **High Risk (Address Immediately)**
- **Legal Compliance**: Missing tax calculation could violate regulations
- **Customer Experience**: No discounts or proration leads to billing disputes
- **Business Model**: Lack of usage-based billing limits pricing flexibility

### **Medium Risk (Address Soon)**
- **Operational Efficiency**: Manual invoice customization is time-consuming
- **Data Insights**: Limited reporting hampers business decisions

### **Low Risk (Address Later)**
- **Advanced Features**: White-label portal, predictive analytics

---

## Recommendations

### **Immediate Actions (Week 1-2)**
1. **Implement Tax Engine**: Highest priority for legal compliance
   - Create tax rate tables by jurisdiction
   - Implement tax calculation service
   - Add tax exemption handling
   - Integrate with invoice generation

2. **Add Discount System**: Required for marketing and acquisition
   - Extend existing coupon system
   - Add trash-specific discount types
   - Implement referral program logic
   - Add loyalty rewards

3. **Basic Proration**: Handle mid-cycle service changes
   - Implement proration calculation service
   - Add proration event tracking
   - Handle container size changes
   - Process cancellation refunds

### **Short-term (Week 3-4)**
4. **Usage-Based Billing**: Core to trash pickup business model
   - Extend usage tracking tables
   - Implement metered billing logic
   - Add overweight charge calculations
   - Handle special waste fees

5. **Invoice Customization**: Improve customer experience
   - Enhance invoice templates with trash-specific fields
   - Add service location details
   - Include container information
   - Add environmental compliance notes

6. **Enhanced Reporting**: Basic business intelligence
   - Add revenue by service type reports
   - Implement customer acquisition cost tracking
   - Add churn rate calculations
   - Create tax liability reports

### **Medium-term (Month 2-3)**
7. **Advanced Analytics**: Churn prediction, LTV calculation
8. **Dunning Automation**: Automated collection processes
9. **Multi-Location Support**: Commercial account needs
10. **White-label Customer Portal**: Self-service capabilities

---

## Implementation Priority Matrix

| Feature | Business Impact | Technical Effort | Legal Requirement | Priority |
|---------|----------------|------------------|-------------------|----------|
| **Tax Engine** | High | Medium | **CRITICAL** | 🟥 **P1** |
| **Discount System** | High | Low | Medium | 🟧 **P1** |
| **Proration Logic** | High | Medium | Medium | 🟧 **P1** |
| **Usage-Based Billing** | High | High | Low | 🟨 **P2** |
| **Invoice Customization** | Medium | Low | Low | 🟨 **P2** |
| **Advanced Reporting** | Medium | Medium | Low | 🟩 **P3** |
| **Dunning Automation** | Low | High | Low | 🟩 **P3** |

---

## Conclusion

The `@fireproof/ar` module provides an **excellent foundation** with production-ready architecture, strong security, and comprehensive testing. However, it lacks **trash pickup industry-specific features** that are critical for TrashTech Pro's launch.

**Key Recommendations**:
1. **Immediately begin Phase 1 implementation** (Tax Engine) - legal compliance is non-negotiable
2. **Follow the phased approach** outlined in `TRASHTECH-BILLING-EXTENSION-PLAN.md`
3. **Leverage existing architecture** - the modular design makes extensions straightforward
4. **Maintain current security standards** - all new features must meet PCI compliance requirements

**Next Step**: Begin Phase 1 (Tax Engine) implementation with focus on:
- Tax rate database schema extensions
- Jurisdiction-based tax calculation service
- Integration with existing invoice generation
- Test coverage for tax scenarios

---

**Investigation Completed By**: LavenderDog  
**Date**: 2026-01-31  
**Status**: READY FOR EXTENSION IMPLEMENTATION  
**Reference Documents**: 
- `TRASHTECH-BILLING-EXTENSION-PLAN.md`
- `FINAL-IMPLEMENTATION-SUMMARY.md`
- `COVERAGE-ANALYSIS.md`

**Next Actions**: 
1. Review with technical team
2. Begin Phase 1 implementation
3. Schedule weekly progress reviews
