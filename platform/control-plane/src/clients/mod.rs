/// HTTP/DB clients used by the control-plane service.
///
/// - `tenant_registry`: Reads eligible tenants for platform billing.
/// - `ar`: Creates AR invoices and customers under the PLATFORM app_id.
pub mod ar;
pub mod tenant_registry;
