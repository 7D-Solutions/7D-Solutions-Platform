INSERT INTO permissions (key, description)
VALUES ('admin.users.read', 'List users within a tenant')
ON CONFLICT (key) DO NOTHING;
