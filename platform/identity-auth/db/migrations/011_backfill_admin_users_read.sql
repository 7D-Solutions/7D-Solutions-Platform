INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id
FROM roles r
JOIN permissions p ON p.key = 'admin.users.read'
WHERE r.is_system = true
  AND r.name = 'admin'
ON CONFLICT (role_id, permission_id) DO NOTHING;
