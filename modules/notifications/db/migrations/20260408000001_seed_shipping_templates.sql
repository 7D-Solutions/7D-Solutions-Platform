-- Phase bd-mozdb: Seed notification templates for shipping events.
--
-- Seeds two email templates used by the outbound_shipped and outbound_delivered
-- consumers. The '_platform_' tenant_id marks these as system-level seeds;
-- tenant-specific overrides can be published via the template API at version 2+.
--
-- Templates use {{var}} substitution (matched by templates::render() in the
-- in-memory registry). required_vars is stored for documentation only —
-- the dispatcher calls templates::render(), not the DB render path.

INSERT INTO notification_templates (
    tenant_id, template_key, version, channel,
    subject, body, required_vars, created_by
) VALUES (
    '_platform_',
    'order_shipped',
    1,
    'email',
    'Your order has shipped — tracking {{tracking_number}}',
    '<p>Hi {{recipient_name}},</p>
<p>Your shipment has been handed to <strong>{{carrier}}</strong>.</p>
<p>Tracking number: <strong>{{tracking_number}}</strong></p>
<p>Shipped at: {{shipped_at}}</p>
<p>Use your tracking number to follow your delivery status.</p>',
    '["tracking_number","carrier","shipped_at","recipient_name"]',
    'platform-seed'
) ON CONFLICT (tenant_id, template_key, version) DO NOTHING;

INSERT INTO notification_templates (
    tenant_id, template_key, version, channel,
    subject, body, required_vars, created_by
) VALUES (
    '_platform_',
    'delivery_confirmed',
    1,
    'email',
    'Your order has been delivered',
    '<p>Hi {{recipient_name}},</p>
<p>Your shipment has been delivered.</p>
<p>Delivered at: {{delivered_at}}</p>
<p>Thank you for your order!</p>',
    '["delivered_at","recipient_name"]',
    'platform-seed'
) ON CONFLICT (tenant_id, template_key, version) DO NOTHING;
